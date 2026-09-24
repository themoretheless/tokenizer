//! A lossless Markdown engine: line-oriented lexer, recovering structural pass.
//!
//! Markdown is a markup language, not a C-like one: this crate owns its own
//! [`SyntaxKind`] vocabulary, has no `comment` kind, and only ever reports
//! Markdown diagnostics. Concatenating token text reconstructs the source
//! byte-for-byte even for abandoned constructs, which stay in the stream flagged
//! [`LexToken::has_error`].
//!
//! ```
//! use themoretheless_tokenizer_markdown::{BlockKind, SyntaxKind, parse};
//!
//! let source = "# Title\n\nSome **bold** text and `code`.\n";
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//!
//! let lexed = parsed.lexed();
//! assert!(lexed.is_lossless());
//! let kinds = lexed.tokens().iter().map(|token| token.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::HeadingMarker));
//! assert!(kinds.contains(&SyntaxKind::Strong));
//! assert_eq!(
//!     parsed.blocks().iter().map(|block| block.kind).collect::<Vec<_>>(),
//!     vec![BlockKind::Heading, BlockKind::Paragraph]
//! );
//! ```
//!
//! Block constructs are decided by a line's leading context, so `*` at a line
//! start is a list marker rather than emphasis and `---` under a paragraph is a
//! setext underline rather than a thematic break. Inline constructs stop at the
//! line break that contains them: an unclosed one becomes a flagged token of its
//! own kind, never a synthesized one, so `is_valid` can be `false` while the
//! token stream stays complete.

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};

pub use parser::{Block, BlockKind, DiagnosticKind, Parse, parse, validate};

pub use themoretheless_tokenizer_core::{Diagnostic, Span};

// ─── Host adapter ────────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, language_descriptor,
    require_default_dialect,
};

/// Host token kind: Markdown's own block and inline vocabulary, so an editor can
/// tell a heading marker from a list marker, or a table pipe from punctuation.
fn host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::HeadingMarker => "heading-marker",
        SyntaxKind::HeadingText => "heading-text",
        SyntaxKind::SetextUnderline => "setext-underline",
        SyntaxKind::ThematicBreak => "thematic-break",
        SyntaxKind::BlockquoteMarker => "blockquote-marker",
        SyntaxKind::ListMarker => "list-marker",
        SyntaxKind::CodeFenceMarker => "code-fence-marker",
        SyntaxKind::FenceInfo => "fence-info",
        SyntaxKind::CodeBlockLine => "code-block-line",
        SyntaxKind::TableDelimiter => "table-delimiter",
        SyntaxKind::TablePipe => "table-pipe",
        SyntaxKind::TableCell => "table-cell",
        SyntaxKind::LinkLabel => "link-label",
        SyntaxKind::LinkDestination => "link-destination",
        SyntaxKind::FrontMatterDelimiter => "front-matter-delimiter",
        SyntaxKind::FrontMatter => "front-matter",
        SyntaxKind::HtmlBlock => "html-block",
        SyntaxKind::HardBreak => "hard-break",
        SyntaxKind::LineBreak => "line-break",
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Strong => "strong",
        SyntaxKind::Emphasis => "emphasis",
        SyntaxKind::Strikethrough => "strikethrough",
        SyntaxKind::CodeSpan => "code-span",
        SyntaxKind::LinkText => "link-text",
        SyntaxKind::ImageMarker => "image-marker",
        SyntaxKind::Autolink => "autolink",
        SyntaxKind::EmailAutolink => "email-autolink",
        SyntaxKind::FootnoteRef => "footnote-ref",
        SyntaxKind::FootnoteDefinitionLabel => "footnote-definition-label",
        SyntaxKind::HtmlInline => "html-inline",
        SyntaxKind::Punctuation => "punctuation",
        SyntaxKind::Text => "text",
        SyntaxKind::Error => "error",
    }
}

fn host_tokenization(parsed: &Parse<'_>) -> HostTokenization {
    let mut tokens = Vec::new();
    for token in parsed.lexed().tokens() {
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
    for diagnostic in parsed.diagnostics() {
        diagnostics.push(HostDiagnostic {
            code: Cow::Borrowed(diagnostic.code),
            message: Cow::Borrowed(diagnostic.message),
            span: HostSpan::from(diagnostic.span),
            severity: Severity::Error,
        });
    }
    HostTokenization {
        tokens,
        diagnostics,
        valid: parsed.is_valid(),
    }
}

/// Host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity tree,
/// cursor, or visitor here, so none of those capabilities are advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::MARKDOWN,
    "Markdown",
    &["md"],
    &[".md", ".markdown"],
    &["text/markdown"],
    env!("CARGO_PKG_VERSION"),
    Capabilities::LEX
        .union(Capabilities::PARSE)
        .union(Capabilities::SEMANTIC)
        .union(Capabilities::VALIDATE),
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
        self.lex(source, opts)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        Ok(host_tokenization(&parse(source)).diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "# Title\n",
        "\n",
        "Some **bold**, *em* and `code` in a paragraph.\n",
        "\n",
        "- item one\n",
        "- [link](https://example.com)\n",
        "\n",
        "> quoted\n",
        "\n",
        "```rust\n",
        "fn main() {}\n",
        "```\n",
        "\n",
        "| a | b |\n",
        "| --- | --- |\n",
        "| 1 | 2 |\n",
        "\n",
        "See [^1].\n",
        "\n",
        "[^1]: note\n",
    );

    #[test]
    fn host_lex_uses_markdown_kinds() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        let kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        for expected in [
            "heading-marker",
            "heading-text",
            "strong",
            "emphasis",
            "code-span",
            "list-marker",
            "blockquote-marker",
            "code-fence-marker",
            "fence-info",
            "code-block-line",
            "table-delimiter",
            "table-pipe",
            "table-cell",
            "link-text",
            "link-destination",
            "footnote-ref",
            "footnote-definition-label",
            "line-break",
            "text",
        ] {
            assert!(kinds.contains(&expected), "missing {expected}: {kinds:?}");
        }
        assert!(!kinds.contains(&"comment"), "{kinds:?}");
    }

    #[test]
    fn host_diagnose_reports_only_markdown_codes() {
        let opts = HostAnalysisOptions::default();
        let broken = concat!(
            "```rust\n",
            "fn main() {}\n",
            "```\n",
            "\n",
            "**unclosed\n",
            "\n",
            "See [^y] and [^x].\n",
            "\n",
            "[^x]: note\n",
            "\n",
            "```\n",
        );
        let diagnostics = ENGINE.diagnose(broken, &opts).unwrap();
        let codes: Vec<&str> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_ref())
            .collect();
        assert!(codes.contains(&"unclosed-code-fence"), "{codes:?}");
        assert!(codes.contains(&"unclosed-emphasis"), "{codes:?}");
        assert!(codes.contains(&"undefined-footnote-reference"), "{codes:?}");
        assert_eq!(codes.len(), 3, "{codes:?}");
    }

    #[test]
    fn sample_reconstructs_byte_for_byte() {
        assert!(lex(SAMPLE).is_lossless());
    }

    #[test]
    fn descriptor_advertises_no_cst_or_visitor() {
        let caps = DESCRIPTOR.capabilities;
        assert!(caps.contains(Capabilities::LEX | Capabilities::PARSE | Capabilities::VALIDATE));
        assert!(!caps.contains(Capabilities::CST));
        assert!(!caps.contains(Capabilities::NAVIGATE));
        assert!(!caps.contains(Capabilities::VISITOR));
    }
}
