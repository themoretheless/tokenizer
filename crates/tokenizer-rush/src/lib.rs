//! Rush editor engine with a dedicated lexer, syntax tree and recovering parser.
//! See the crate README for the supported grammar. The experimental evaluator
//! executes a bounded functional subset; parsing supports a broader grammar.

mod project;
pub use project::{
    ModuleEditorAnalysis, ProjectEdit, analyze_editor_project, import_completions,
    rename_project_symbol,
};
mod analysis;
pub use analysis::{LexicalBinding, MemberCompletion, NameReference};
mod ast;
mod format;
mod graphics;
mod host_object;
pub use format::{FormatError, format_source};
pub use host_object::HostObject;
mod matrix;
mod mesh;
mod noise;
pub use mesh::Mesh;
mod quaternion;
pub use matrix::Matrix4;
pub use quaternion::Quaternion;
mod lexer;
pub use graphics::Polygon;
mod parser;
mod runtime;
mod string_literal;
pub use ast::*;
pub use runtime::{
    Builtin, CallFrame, CancellationToken, Closure, CoroutineId, CoroutineScheduler,
    CoroutineState, ExecutionLimits, HostCallback, HostFunction, HostRegistration, HostSequence,
    HostSequenceIterator, ModelInstance, ModelNode, ModelProgram, OwnedCoroutineScheduler,
    OwnedScriptInstance, Program, RegionStats, RuntimeError, ScheduledState, ScheduledStep,
    ScriptInstance, ScriptState, Sequence, SourceLocation, StateValue, UserData, Value, ValueType,
    WakeRequest, builtin_catalog, evaluate, json_parse, json_stringify,
};
pub mod repl;
pub use repl::{ReplCommand, ReplOutcome, ReplSession, is_input_complete, pretty_print_value};
pub mod bytecode;
pub use bytecode::{BytecodeClosure, BytecodeFunction, BytecodeProgram, evaluate_bytecode};

use themoretheless_tokenizer_core::{
    Capabilities, Diagnostic, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, InputLimits, LanguageDescriptor, LanguageId, Lexed, SemanticToken,
    SemanticTokenization, Span, full_descriptor, require_default_dialect,
};

/// Lossless, case-sensitive Rush lexer using conservative resource limits.
#[must_use]
pub fn lex(source: &str) -> Lexed {
    lexer::run(source, InputLimits::conservative()).0
}

/// Parse the documented Rush grammar, recovering at statement boundaries.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    parse_with(source, InputLimits::conservative())
}

/// Parse with explicit input, token, diagnostic and nesting budgets.
/// A zero diagnostic budget suppresses messages, never the invalid status.
#[must_use]
pub fn parse_with(source: &str, limits: InputLimits) -> Parse<'_> {
    if limits.exceeds_input_bytes(source.len()) {
        let span = Span::new(0, source.len());
        return Parse {
            source,
            lexed: Lexed::default(),
            module: Module {
                span,
                items: vec![],
            },
            diagnostics: if limits.max_diagnostics == 0 {
                vec![]
            } else {
                vec![Diagnostic::new(
                    span,
                    "input-limit",
                    "Input byte limit exceeded",
                )]
            },
            valid: false,
            roles: vec![],
        };
    }
    let (lexed, valid) = lexer::run(source, limits);
    parser::run(source, lexed, valid, limits)
}

/// Parse and check lexical bindings. Unknown names are accepted for host integration.
/// `const` and function names cannot be rebound; inner scopes may shadow names.
#[must_use]
pub fn analyze(source: &str) -> Parse<'_> {
    analyze_with(source, InputLimits::conservative())
}

/// Binding analysis with the same resource and diagnostic budgets as parsing.
#[must_use]
pub fn analyze_with(source: &str, limits: InputLimits) -> Parse<'_> {
    let mut parsed = parse_with(source, limits);
    analysis::check(&mut parsed, limits.max_diagnostics);
    parsed
}

fn semantic(parsed: &Parse<'_>, syntax: bool) -> SemanticTokenization {
    let roles: std::collections::HashMap<_, _> = parsed.roles.iter().copied().collect();
    SemanticTokenization {
        tokens: parsed
            .lexed
            .tokens
            .iter()
            .map(|token| SemanticToken {
                span: token.span,
                kind: if syntax {
                    token.kind.as_str()
                } else {
                    roles
                        .get(&token.span)
                        .copied()
                        .unwrap_or(token.kind.as_str())
                },
            })
            .collect(),
        diagnostics: parsed.diagnostics.clone(),
    }
}

/// Syntax-aware highlighting; declarations, parameters, types and members have
/// distinct roles. This does not resolve references or infer types.
#[must_use]
pub fn tokenize(source: &str) -> SemanticTokenization {
    semantic(&parse(source), false)
}

/// Diagnostics from the dedicated Rush parser.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Host;
pub static ENGINE: Host = Host;
pub static DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
    capabilities: Capabilities::LEX
        .union(Capabilities::PARSE)
        .union(Capabilities::SEMANTIC)
        .union(Capabilities::VALIDATE),
    ..full_descriptor(
        LanguageId::RUSH,
        "rush",
        &["modelgraph-text", "mg"],
        &[".r"],
        &["text/x-rush"],
        env!("CARGO_PKG_VERSION"),
    )
};

fn checked<'s>(source: &'s str, opts: &HostAnalysisOptions) -> Result<Parse<'s>, HostError> {
    require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
    if opts.limits.exceeds_input_bytes(source.len()) {
        return Err(HostError::InputTooLarge {
            max: opts.limits.max_input_bytes,
            actual: source.len(),
        });
    }
    Ok(parse_with(source, opts.limits))
}
fn host_tokens(parsed: Parse<'_>, syntax: bool) -> HostTokenization {
    let mut result = semantic(&parsed, syntax).to_host();
    result.valid = parsed.is_valid();
    result
}
impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }
    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        Ok(host_tokens(checked(source, opts)?, true))
    }
    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        Ok(host_tokens(checked(source, opts)?, false))
    }
    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(checked(source, opts)?
            .diagnostics
            .into_iter()
            .map(HostDiagnostic::from_diagnostic)
            .collect())
    }
}

/// Analyze lexical bindings and return source links for editor navigation.
/// Unresolved names have no definition; they may belong to the embedding host.
/// Recovered syntax errors suppress links to avoid misleading navigation.
pub fn analyze_references(source: &str) -> (Parse<'_>, Vec<NameReference>) {
    let mut parsed = parse(source);
    let references = analysis::check(&mut parsed, InputLimits::conservative().max_diagnostics);
    (parsed, references)
}

/// Shared lexical analysis for editor navigation and scope-aware completion.
/// Syntax errors suppress both outputs, except a missing final member name.
/// Bindings appear after their initializer; recovery never makes execution valid.
pub fn analyze_editor(source: &str) -> (Parse<'_>, Vec<NameReference>, Vec<LexicalBinding>) {
    let details = analyze_editor_details(source);
    (details.parsed, details.references, details.bindings)
}

/// Editor metadata from one parse and lexical pass, including expression receivers.
pub struct EditorAnalysis<'s> {
    pub parsed: Parse<'s>,
    pub references: Vec<NameReference>,
    pub bindings: Vec<LexicalBinding>,
    pub member_completions: Vec<MemberCompletion>,
}

/// Statically determine member candidates without running user expressions.
pub fn analyze_editor_details(source: &str) -> EditorAnalysis<'_> {
    let mut parsed = parse(source);
    let (references, bindings, member_completions) =
        analysis::editor(&mut parsed, InputLimits::conservative().max_diagnostics);
    EditorAnalysis {
        parsed,
        references,
        bindings,
        member_completions,
    }
}

/// Inspect explicit module interfaces without initializing modules or invoking host code.
pub fn analyze_editor_modules<'s>(
    source: &'s str,
    modules: &[(&str, &Program<'_>)],
) -> EditorAnalysis<'s> {
    let mut parsed = parse(source);
    let exports = analysis::module_interfaces(modules);
    let (references, bindings, member_completions) = analysis::editor_modules(&mut parsed, exports);
    EditorAnalysis {
        parsed,
        references,
        bindings,
        member_completions,
    }
}

/// Check lexical names against builtins plus explicitly supplied host/input names.
/// This opt-in check reports unresolved references even in unexecuted branches.
pub fn analyze_names<'s>(source: &'s str, external_names: &[&str]) -> Parse<'s> {
    let (mut parsed, references) = analyze_references(source);
    let limit = InputLimits::conservative().max_diagnostics;
    check_external_names(&mut parsed, &references, external_names, limit);

    parsed
}

fn check_external_names(
    parsed: &mut Parse<'_>,
    references: &[NameReference],
    external_names: &[&str],
    limit: usize,
) {
    for reference in references {
        if reference.definition.is_some() {
            continue;
        }
        let name = &parsed.source[reference.usage.start..reference.usage.end];
        if builtin_catalog()
            .iter()
            .any(|(builtin, _)| *builtin == name)
            || external_names.contains(&name)
        {
            continue;
        }
        parsed.valid = false;
        if parsed.diagnostics.len() < limit {
            parsed.diagnostics.push(Diagnostic::new(
                reference.usage,
                "unknown-name",
                "Name is not declared or registered by the host",
            ));
        }
    }
}

/// Editor output using the same host function objects as execution. Functions expose
/// names, parameter types and result types for completion without invoking callbacks.
pub struct HostEditorAnalysis<'s> {
    pub parsed: Parse<'s>,
    pub references: Vec<NameReference>,
    pub bindings: Vec<LexicalBinding>,
    pub member_completions: Vec<MemberCompletion>,
    pub functions: Vec<std::rc::Rc<HostFunction>>,
}

/// Check names, calls and host registrations while producing navigation/completion
/// metadata. Input names are separate from callable registrations, as in Program::run.
/// Local bindings shadow registered functions. Only a missing final member name
/// permits editor recovery; the source still cannot execute.
pub fn analyze_editor_with_host<'s>(
    source: &'s str,
    input_names: &[&str],
    functions: &[std::rc::Rc<HostFunction>],
) -> HostEditorAnalysis<'s> {
    let mut parsed = parse(source);
    let limit = InputLimits::conservative().max_diagnostics;
    let signatures = functions
        .iter()
        .map(|f| (f.name.to_owned(), f.clone()))
        .collect();
    let (references, bindings, member_completions) =
        analysis::editor_with_hosts(&mut parsed, limit, true, signatures);
    let mut names: std::collections::HashSet<&str> =
        builtin_catalog().iter().map(|(name, _)| *name).collect();
    for name in functions
        .iter()
        .map(|f| f.name)
        .chain(input_names.iter().copied())
    {
        if !names.insert(name) {
            parsed.valid = false;
            if parsed.diagnostics.len() < limit {
                parsed.diagnostics.push(Diagnostic::new(
                    Span::new(0, 0),
                    "duplicate-host-name",
                    "Host or input name conflicts with another registration",
                ));
            }
        }
    }
    let external_names: Vec<_> = names.into_iter().collect();
    check_external_names(&mut parsed, &references, &external_names, limit);
    HostEditorAnalysis {
        parsed,
        references,
        bindings,
        functions: functions.to_vec(),
        member_completions,
    }
}

/// Opt-in checks for direct builtin call arity, including implicit pipeline input.
/// Local bindings shadow builtin names; dynamic function values are not inferred.
pub fn analyze_calls(source: &str) -> Parse<'_> {
    let mut parsed = parse(source);
    analysis::check_calls(
        &mut parsed,
        InputLimits::conservative().max_diagnostics,
        true,
    );
    parsed
}

/// Check arity and provably incompatible argument types using runtime registrations.
/// Literals and known host results propagate through direct calls, aliases and pipes.
/// Unknown values remain runtime checks; lexical bindings shadow host names.
pub fn analyze_host_calls<'s>(
    source: &'s str,
    functions: &[std::rc::Rc<HostFunction>],
) -> Parse<'s> {
    let mut parsed = parse(source);
    let signatures = functions
        .iter()
        .map(|function| (function.name.to_owned(), function.clone()))
        .collect();
    analysis::check_host_calls(
        &mut parsed,
        InputLimits::conservative().max_diagnostics,
        true,
        signatures,
    );
    parsed
}

#[cfg(not(target_arch = "wasm32"))]
pub mod sys;
