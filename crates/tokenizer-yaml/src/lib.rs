//! A lossless YAML 1.2 lexer with recovering diagnostics and a borrowing AST.
//!
//! Like the JSON and TOML engines, token text concatenation always
//! reconstructs the source byte-for-byte, so editors can tokenize incomplete
//! documents. Spans are UTF-8 byte offsets into the source string, and the AST
//! borrows from the parsed source instead of copying string data.
//!
//! Line breaks are significant [`SyntaxKind::LineBreak`] tokens because YAML
//! delimits nodes with line structure and indentation. The lexer tracks only
//! flow context; the parser derives block columns from token spans, so block
//! scalar content appears as ordinary tokens after a
//! [`SyntaxKind::BlockScalarHeader`] and the parser owns dedent detection,
//! folding, and anchor resolution.

mod ast;
mod lexer;

pub use ast::{
    Alias, Anchor, CollectionStyle, Directive, Document, Entry, Mapping, Node, NodeKind, Scalar,
    ScalarStyle, Sequence, TagHandle,
};

pub use lexer::{
    LexDiagnostic, LexDiagnosticKind, LexToken, Lexed, LexerOptions, SyntaxKind, TokenFlags, lex,
    lex_with,
};

pub use themoretheless_tokenizer_core::Span;

// ─── Host adapter ────────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan, HostToken,
    HostTokenization, LanguageDescriptor, LanguageId, Severity, SyntaxKind as CoreSyntaxKind,
    full_descriptor, require_default_dialect,
};

fn core_kind(kind: SyntaxKind) -> CoreSyntaxKind {
    match kind {
        SyntaxKind::Whitespace | SyntaxKind::LineBreak | SyntaxKind::Bom => {
            CoreSyntaxKind::Whitespace
        }
        SyntaxKind::Comment => CoreSyntaxKind::LineComment,
        SyntaxKind::DocumentStart
        | SyntaxKind::DocumentEnd
        | SyntaxKind::BlockEntry
        | SyntaxKind::KeyIndicator
        | SyntaxKind::ValueIndicator
        | SyntaxKind::FlowSequenceStart
        | SyntaxKind::FlowSequenceEnd
        | SyntaxKind::FlowMappingStart
        | SyntaxKind::FlowMappingEnd
        | SyntaxKind::FlowEntry => CoreSyntaxKind::Punctuation,
        SyntaxKind::Directive | SyntaxKind::BlockScalarHeader => CoreSyntaxKind::Operator,
        SyntaxKind::Anchor | SyntaxKind::Alias | SyntaxKind::Tag => CoreSyntaxKind::Identifier,
        SyntaxKind::SingleQuotedScalar | SyntaxKind::DoubleQuotedScalar => {
            CoreSyntaxKind::StringLit
        }
        SyntaxKind::PlainScalar => CoreSyntaxKind::Identifier,
        SyntaxKind::Error => CoreSyntaxKind::Error,
    }
}

fn host_tokenization(lexed: &Lexed<'_>) -> HostTokenization {
    let mut tokens = Vec::new();
    for token in lexed.tokens() {
        let kind = if token.has_error() {
            CoreSyntaxKind::Error
        } else {
            core_kind(token.kind)
        };
        tokens.push(HostToken {
            kind: Cow::Borrowed(kind.as_str()),
            span: HostSpan::from(token.span),
            error: kind == CoreSyntaxKind::Error,
        });
    }
    let mut diagnostics = Vec::new();
    for diagnostic in lexed.diagnostics() {
        diagnostics.push(HostDiagnostic {
            code: Cow::Borrowed(diagnostic.kind.code()),
            message: Cow::Owned(diagnostic.kind.to_string()),
            span: HostSpan::from(diagnostic.span),
            severity: Severity::Error,
        });
    }
    let valid = diagnostics.is_empty();
    HostTokenization {
        tokens,
        diagnostics,
        valid,
    }
}

/// Host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = full_descriptor(
    LanguageId::YAML,
    "YAML",
    &["yml"],
    &[".yaml", ".yml"],
    &["application/yaml", "text/yaml"],
    env!("CARGO_PKG_VERSION"),
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        Ok(host_tokenization(&lex(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        self.lex(source, opts)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(host_tokenization(&lex(source)).diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_lex() {
        let source = "---\nname: yaml\nlist:\n  - 1\n...\n";
        assert!(
            themoretheless_tokenizer_core::verify_lossless_spans(
                source,
                lex(source).tokens().iter().map(|t| t.span)
            )
            .is_ok()
        );
    }

    #[test]
    fn host_lex_uses_engine_tokens() {
        let source = "key: value\n";
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(tokenization.valid);
        let kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert!(kinds.contains(&"punctuation"));
    }

    #[test]
    fn host_diagnose_reports_unterminated_scalar() {
        let source = "key: \"oops\n";
        let opts = HostAnalysisOptions::default();
        let diagnostics = ENGINE.diagnose(source, &opts).unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unterminated-quoted-scalar")
        );
    }
}
