//! Full rush engine (lex → parse → AST → semantic).
//!
//! rush is the single language of ruos and open-scad-viewer: indentation
//! blocks, `fn` headers, fluent chains, `foreach ... yield`, `match`,
//! `$variables`, `//` comments and shell pipe sugar.

use themoretheless_tokenizer_core::{
    Diagnostic, FullProfile, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostTokenization, LanguageDescriptor, LanguageId, Lexed, Parse, SemanticTokenization,
    analyze_full_host, full_descriptor, lex_full, lex_to_host, parse_full, require_default_dialect,
    semantic_full,
};

fn profile() -> FullProfile {
    FullProfile {
        keywords: &[
            "if", "else", "fn", "return", "for", "foreach", "in", "yield", "match", "where",
            "select", "count", "run", "and", "or", "not", "param", "show", "assert", "let",
            "async", "await", "break", "const", "continue", "false", "import", "print", "true",
            "while",
        ],
        types: &[
            "int", "f64", "str", "bool", "T", "Geometry", "Row", "float", "string", "list", "map",
        ],
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: true,
        triple_strings: false,
        soft_indent_blocks: true,
    }
}

/// Lossless lexer.
#[must_use]
pub fn lex(source: &str) -> Lexed {
    lex_full(source, &profile())
}

/// Recovering parse with borrowing AST.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    parse_full(source, &profile())
}

/// Parser-aware semantic tokens.
#[must_use]
pub fn tokenize(source: &str) -> SemanticTokenization {
    semantic_full(&parse(source))
}

/// Diagnostics from the full pipeline.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).diagnostics
}

/// Host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = full_descriptor(
    LanguageId::RUSH,
    "rush",
    &["modelgraph-text", "mg"],
    &[".r"],
    &["text/x-rush"],
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
        Ok(lex_to_host(lex(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        Ok(analyze_full_host(source, &profile()))
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(validate(source)
            .into_iter()
            .map(themoretheless_tokenizer_core::HostDiagnostic::from_diagnostic)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_syntax_and_extension() {
        use themoretheless_tokenizer_core::SyntaxKind;
        assert_eq!(DESCRIPTOR.extensions, &[".r"]);
        let source = "fn main() { return 1; } // comment\n";
        let lexed = lex(source);
        assert!(lexed.is_lossless(source));
        assert!(
            lexed
                .tokens
                .iter()
                .any(|t| t.kind == SyntaxKind::Keyword && &source[t.span.range()] == "return")
        );
        assert!(
            lexed
                .tokens
                .iter()
                .any(|t| t.kind == SyntaxKind::LineComment
                    && &source[t.span.range()] == "// comment")
        );
        let old = lex("ret # text");
        assert!(
            !old.tokens
                .iter()
                .any(|t| matches!(t.kind, SyntaxKind::Keyword | SyntaxKind::LineComment))
        );
    }

    #[test]
    fn lossless_lex() {
        let source = "big = ls(\"/docs\").where(size > 100).select(name)\n";
        assert!(lex(source).is_lossless(source));
    }

    #[test]
    fn parse_smoke() {
        let source = "if $status = 0:\n    echo ok\nelse:\n    echo bad\n";
        let p = parse(source);
        assert!(p.lexed.is_lossless(source));
        assert!(!p.module.items.is_empty());
    }

    #[test]
    fn fn_header_and_pipes() {
        let source = "fn greet who: T -> T\n    return who\n\nls | where size > 0 | select name\n";
        assert!(lex(source).is_lossless(source));
    }

    #[test]
    fn foreach_yield_match() {
        let source = "foreach item in items\n    yield match item\n";
        let p = parse(source);
        assert!(p.lexed.is_lossless(source));
    }
}
