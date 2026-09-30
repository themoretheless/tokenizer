//! Rush editor engine with a dedicated lexer, syntax tree and recovering parser.
//! See the crate README for the supported grammar. No evaluator or type checker
//! is provided: validation checks syntax and structural control-flow rules.

mod ast;
mod lexer;
mod parser;
pub use ast::*;

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
