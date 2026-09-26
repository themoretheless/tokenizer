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
mod parser;

pub use ast::{
    Alias, Anchor, CollectionStyle, Directive, Document, Entry, Mapping, Node, NodeKind, Scalar,
    ScalarStyle, Sequence, TagHandle,
};

pub use lexer::{
    LexDiagnostic, LexDiagnosticKind, LexToken, Lexed, LexerOptions, SyntaxKind, TokenFlags, lex,
    lex_with,
};

pub use parser::{
    MAX_SUPPORTED_DEPTH, Parse, ParseDiagnostic, ParseDiagnosticKind, ParseOptions, parse,
    parse_with,
};

pub use themoretheless_tokenizer_core::Span;

// ─── Host adapter ────────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, language_descriptor,
    require_default_dialect,
};

/// YAML's grammar is hand-written too, so the same claim as TOML: it rejects
/// unclosed flow collections and reports nothing on valid documents.
const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::VALIDATE);

/// Host token kind: YAML's own node vocabulary, so an editor can tell a key
/// indicator from a flow entry, or an anchor from a plain scalar.
fn host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::LineBreak => "line-break",
        SyntaxKind::Bom => "bom",
        SyntaxKind::Comment => "comment",
        SyntaxKind::DocumentStart => "document-start",
        SyntaxKind::DocumentEnd => "document-end",
        SyntaxKind::Directive => "directive",
        SyntaxKind::BlockEntry => "block-entry",
        SyntaxKind::KeyIndicator => "key-indicator",
        SyntaxKind::ValueIndicator => "value-indicator",
        SyntaxKind::FlowSequenceStart => "flow-sequence-start",
        SyntaxKind::FlowSequenceEnd => "flow-sequence-end",
        SyntaxKind::FlowMappingStart => "flow-mapping-start",
        SyntaxKind::FlowMappingEnd => "flow-mapping-end",
        SyntaxKind::FlowEntry => "flow-entry",
        SyntaxKind::Anchor => "anchor",
        SyntaxKind::Alias => "alias",
        SyntaxKind::Tag => "tag",
        SyntaxKind::BlockScalarHeader => "block-scalar-header",
        SyntaxKind::SingleQuotedScalar => "single-quoted-scalar",
        SyntaxKind::DoubleQuotedScalar => "double-quoted-scalar",
        SyntaxKind::PlainScalar => "plain-scalar",
        SyntaxKind::Error => "error",
    }
}

fn host_tokenization(parse: &Parse<'_>) -> HostTokenization {
    let lexed = parse.lexed();
    let mut tokens = Vec::new();
    for token in lexed.tokens() {
        let error = token.has_error();
        tokens.push(HostToken {
            kind: Cow::Borrowed(if error {
                "error"
            } else {
                host_kind(token.kind)
            }),
            span: HostSpan::from(token.span),
            error,
        });
    }
    let mut diagnostics = Vec::new();
    for diagnostic in parse.diagnostics() {
        diagnostics.push(HostDiagnostic {
            code: Cow::Borrowed(diagnostic.kind.code()),
            message: Cow::Owned(diagnostic.kind.to_string()),
            span: HostSpan::from(diagnostic.span),
            severity: Severity::Error,
        });
    }
    let valid = parse.is_valid();
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

pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::YAML,
    "YAML",
    &["yml"],
    &[".yaml", ".yml"],
    &["application/yaml", "text/yaml"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
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
        Ok(host_tokenization(&parse(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        // SEMANTIC dropped: identical to syntax (measurement-driven capability honesty).
        self.lex(source, opts)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(host_tokenization(&parse(source)).diagnostics)
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
        assert!(kinds.contains(&"value-indicator"), "{kinds:?}");
        assert!(kinds.contains(&"plain-scalar"), "{kinds:?}");
        assert!(kinds.contains(&"line-break"), "{kinds:?}");
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
