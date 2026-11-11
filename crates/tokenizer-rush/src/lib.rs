//! Rush editor engine with a dedicated lexer, syntax tree and recovering parser.
//! See the crate README for the supported grammar. The experimental evaluator
//! executes a bounded functional subset; parsing supports a broader grammar.

#[cfg(feature = "runtime")]
mod project;
#[cfg(feature = "runtime")]
pub use project::{
    ModuleEditorAnalysis, ProjectEdit, analyze_editor_project, import_completions,
    rename_project_symbol,
};

mod analysis;
pub use analysis::{LexicalBinding, MemberCompletion, NameReference};
mod ast;
mod catalog;
pub use catalog::{Builtin, ValueType, builtin_catalog};
mod format;
pub use format::{FormatError, format_source};
mod lexer;
mod parser;
mod string_literal;
pub use ast::*;

#[cfg(feature = "runtime")]
mod graphics;
#[cfg(feature = "runtime")]
pub use graphics::Polygon;
#[cfg(feature = "runtime")]
mod host_object;
#[cfg(feature = "runtime")]
pub use host_object::HostObject;
#[cfg(feature = "runtime")]
mod matrix;
#[cfg(feature = "runtime")]
pub use matrix::Matrix4;
#[cfg(feature = "runtime")]
mod mesh;
#[cfg(feature = "runtime")]
pub use mesh::Mesh;
#[cfg(feature = "runtime")]
mod noise;
#[cfg(feature = "runtime")]
mod quaternion;
#[cfg(feature = "runtime")]
pub use quaternion::Quaternion;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "runtime")]
pub use runtime::{
    CallFrame, CancellationToken, Closure, CoroutineId, CoroutineScheduler, CoroutineState,
    ExecutionLimits, HostCallback, HostFunction, HostRegistration, HostSequence,
    HostSequenceIterator, InlineVec, ModelInstance, ModelNode, ModelProgram,
    OwnedCoroutineScheduler, OwnedScriptInstance, Program, RecordMap, RecordShape, RegionStats,
    RuntimeError, ScheduledState, ScheduledStep, ScriptInstance, ScriptState, Sequence,
    SourceLocation, StateValue, UserData, Value, WakeRequest, evaluate, json_parse, json_stringify,
};
#[cfg(all(feature = "runtime", not(target_arch = "wasm32")))]
pub mod repl;
#[cfg(all(feature = "runtime", not(target_arch = "wasm32")))]
pub use repl::{ReplCommand, ReplOutcome, ReplSession, is_input_complete, pretty_print_value};
#[cfg(feature = "runtime")]
pub mod bytecode;
#[cfg(feature = "runtime")]
pub use bytecode::{
    BytecodeClosure, BytecodeFunction, BytecodeIterator, BytecodeProgram, BytecodeSequence,
    evaluate_bytecode,
};

use themoretheless_tokenizer_core::{
    Capabilities, Diagnostic, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostToken, HostTokenization, InputLimits, LanguageDescriptor, LanguageId, Lexed, SemanticToken,
    SemanticTokenization, Span, TokenLayer, full_descriptor,
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
    let len = parsed.lexed.tokens.len();
    let mut tokens = Vec::with_capacity(len);
    if syntax || parsed.roles.is_empty() {
        for token in &parsed.lexed.tokens {
            tokens.push(SemanticToken {
                span: token.span,
                kind: token.kind.as_str(),
            });
        }
    } else {
        let roles = parsed.roles.as_slice();
        let mut cursor = 0usize;
        for token in &parsed.lexed.tokens {
            while cursor < roles.len()
                && (roles[cursor].0.start, roles[cursor].0.end) < (token.span.start, token.span.end)
            {
                cursor += 1;
            }
            let kind = if cursor < roles.len() && roles[cursor].0 == token.span {
                roles[cursor].1
            } else {
                token.kind.as_str()
            };
            tokens.push(SemanticToken {
                span: token.span,
                kind,
            });
        }
    }
    SemanticTokenization {
        tokens,
        diagnostics: parsed.diagnostics.clone(),
    }
}

fn host_tokens_from_parse(parsed: &Parse<'_>, syntax: bool) -> HostTokenization {
    use std::borrow::Cow;
    let mut tokens = Vec::with_capacity(parsed.lexed.tokens.len());
    if syntax || parsed.roles.is_empty() {
        for token in &parsed.lexed.tokens {
            tokens.push(HostToken {
                span: token.span.into(),
                kind: Cow::Borrowed(token.kind.as_str()),
                error: token.kind == themoretheless_tokenizer_core::SyntaxKind::Error,
            });
        }
    } else {
        let roles = parsed.roles.as_slice();
        let mut cursor = 0usize;
        for token in &parsed.lexed.tokens {
            while cursor < roles.len()
                && (roles[cursor].0.start, roles[cursor].0.end) < (token.span.start, token.span.end)
            {
                cursor += 1;
            }
            let kind = if cursor < roles.len() && roles[cursor].0 == token.span {
                roles[cursor].1
            } else {
                token.kind.as_str()
            };
            tokens.push(HostToken {
                span: token.span.into(),
                kind: Cow::Borrowed(kind),
                error: kind == "error",
            });
        }
    }
    let diagnostics = parsed
        .diagnostics
        .iter()
        .cloned()
        .map(HostDiagnostic::from_diagnostic)
        .collect();
    HostTokenization {
        tokens,
        diagnostics,
        valid: parsed.is_valid(),
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
        &["shell"],
        &[".r"],
        &["text/x-rush"],
        env!("CARGO_PKG_VERSION"),
    )
};

fn checked<'s>(
    host: &Host,
    source: &'s str,
    opts: &HostAnalysisOptions,
) -> Result<Parse<'s>, HostError> {
    host.check_entry(source, opts)?;
    Ok(parse_with(source, opts.limits))
}

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }
    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        Ok(host_tokens_from_parse(&checked(self, source, opts)?, true))
    }
    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        Ok(host_tokens_from_parse(&checked(self, source, opts)?, false))
    }
    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(checked(self, source, opts)?
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

/// Unified single-parse playground analysis returning host tokens, execution
/// diagnostics, bindings, references, and member completions without re-lexing
/// or re-parsing the source document.
pub struct PlaygroundAnalysis {
    pub host: HostTokenization,
    pub execution_diagnostics: Vec<Diagnostic>,
    pub references: Vec<NameReference>,
    pub bindings: Vec<LexicalBinding>,
    pub member_completions: Vec<MemberCompletion>,
}

/// Analyze Rush source in a single lex + parse pass for interactive editors and playgrounds.
pub fn analyze_playground(
    source: &str,
    opts: &HostAnalysisOptions,
    layer: TokenLayer,
) -> Result<PlaygroundAnalysis, HostError> {
    let mut parsed = checked(&ENGINE, source, opts)?;
    let host = host_tokens_from_parse(&parsed, layer == TokenLayer::Syntax);
    let grammar_diag_len = parsed.diagnostics.len();
    let limit = opts.limits.max_diagnostics;

    if parsed.is_valid() {
        let (references, bindings, member_completions) =
            analysis::editor_with_hosts(&mut parsed, limit, true, std::collections::HashMap::new());
        check_external_names(&mut parsed, &references, &[], limit);
        let mut execution_diagnostics = Vec::new();
        for diag in parsed.diagnostics.drain(grammar_diag_len..) {
            let key = (diag.code, diag.span);
            if !execution_diagnostics
                .iter()
                .any(|d: &Diagnostic| (d.code, d.span) == key)
                && !host
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_ref() == diag.code && d.span == diag.span.into())
            {
                execution_diagnostics.push(diag);
            }
        }
        Ok(PlaygroundAnalysis {
            host,
            execution_diagnostics,
            references,
            bindings,
            member_completions,
        })
    } else {
        let (references, bindings, member_completions) = analysis::editor(&mut parsed, limit);
        Ok(PlaygroundAnalysis {
            host,
            execution_diagnostics: Vec::new(),
            references,
            bindings,
            member_completions,
        })
    }
}

/// Inspect explicit module interfaces without initializing modules or invoking host code.
#[cfg(feature = "runtime")]
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
        if catalog::lookup_builtin(name).is_some() || external_names.contains(&name) {
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
#[cfg(feature = "runtime")]
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
#[cfg(feature = "runtime")]
pub fn analyze_editor_with_host<'s>(
    source: &'s str,
    input_names: &[&str],
    functions: &[std::rc::Rc<HostFunction>],
) -> HostEditorAnalysis<'s> {
    let mut parsed = parse(source);
    let limit = InputLimits::conservative().max_diagnostics;
    let signatures = functions
        .iter()
        .map(|f| {
            (
                f.name.to_owned(),
                std::rc::Rc::new(analysis::HostSignature {
                    parameters: f.parameters.clone(),
                    result: f.result.clone(),
                    transactional: true,
                }),
            )
        })
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
#[cfg(feature = "runtime")]
pub fn analyze_host_calls<'s>(
    source: &'s str,
    functions: &[std::rc::Rc<HostFunction>],
) -> Parse<'s> {
    let mut parsed = parse(source);
    let signatures = functions
        .iter()
        .map(|function| {
            (
                function.name.to_owned(),
                std::rc::Rc::new(analysis::HostSignature {
                    parameters: function.parameters.clone(),
                    result: function.result.clone(),
                    transactional: true,
                }),
            )
        })
        .collect();
    analysis::check_host_calls(
        &mut parsed,
        InputLimits::conservative().max_diagnostics,
        true,
        signatures,
    );
    parsed
}

#[cfg(all(feature = "runtime", not(target_arch = "wasm32")))]
pub mod sys;
