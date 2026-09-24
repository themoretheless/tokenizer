//! A lossless CSS lexer, a recovering structural pass, and a rule list.
//!
//! Token text concatenation always reconstructs the source byte-for-byte, so
//! editors can tokenize half-typed stylesheets. Spans are UTF-8 byte offsets,
//! and every token keeps its own [`SyntaxKind`] rather than a generic
//! programming-language vocabulary: a property, a selector and a value are
//! different kinds, `#id` and `#fff` are told apart by context, and the only
//! diagnostics are CSS ones.
//!
//! ```
//! use themoretheless_tokenizer_css::{SyntaxKind, parse};
//!
//! let source = "body.dark {\n  color: #fff;\n  margin: 0 auto;\n}\n";
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//!
//! let lexed = parsed.lexed();
//! let kinds = lexed.tokens().iter().map(|token| token.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::ClassSelector));
//! assert!(kinds.contains(&SyntaxKind::Property));
//! assert!(kinds.contains(&SyntaxKind::Color));
//! assert_eq!(parsed.rules()[0].prelude, "body.dark");
//! ```
//!
//! The lexer carries context (a brace stack plus a selector-vs-declaration
//! mode) because CSS reuses punctuation: `:` separates a property from its
//! value inside a block but marks a pseudo-class in a prelude. Broken atoms
//! are emitted as single error-flagged tokens instead of being dropped, so
//! [`validate`] can point at one token per problem while the stream stays
//! lossless.

mod lexer;
mod parser;

pub use lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};
pub use parser::{Parse, Rule, RuleKind, parse, validate};

pub use themoretheless_tokenizer_core::Span;

// ─── Host adapter ────────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, language_descriptor,
    require_default_dialect,
};

/// Real engine surface: no CST, cursor navigation or visitor API exists here.
const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC)
    .union(Capabilities::VALIDATE);

/// Host token kind: CSS's own vocabulary, so an editor can tell a property
/// from a type selector, or an id from a color.
fn host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Comment => "comment",
        SyntaxKind::Colon => "colon",
        SyntaxKind::Semicolon => "semicolon",
        SyntaxKind::Comma => "comma",
        SyntaxKind::LeftBrace => "left-brace",
        SyntaxKind::RightBrace => "right-brace",
        SyntaxKind::LeftParen => "left-paren",
        SyntaxKind::RightParen => "right-paren",
        SyntaxKind::LeftBracket => "left-bracket",
        SyntaxKind::RightBracket => "right-bracket",
        SyntaxKind::Operator => "operator",
        SyntaxKind::Dot => "dot",
        SyntaxKind::TypeSelector => "type-selector",
        SyntaxKind::ClassSelector => "class-selector",
        SyntaxKind::IdSelector => "id-selector",
        SyntaxKind::UniversalSelector => "universal-selector",
        SyntaxKind::AttributeSelector => "attribute-selector",
        SyntaxKind::PseudoClass => "pseudo-class",
        SyntaxKind::PseudoElement => "pseudo-element",
        SyntaxKind::NestingSelector => "nesting-selector",
        SyntaxKind::Property => "property",
        SyntaxKind::Variable => "variable",
        SyntaxKind::Important => "important",
        SyntaxKind::AtRule => "at-rule",
        SyntaxKind::AtRulePreludeText => "at-rule-prelude-text",
        SyntaxKind::Function => "function",
        SyntaxKind::Color => "color",
        SyntaxKind::Number => "number",
        SyntaxKind::Percentage => "percentage",
        SyntaxKind::Unit => "unit",
        SyntaxKind::String => "string",
        SyntaxKind::Value => "value",
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
    let diagnostics = parsed
        .diagnostics()
        .iter()
        .map(|diagnostic| HostDiagnostic {
            code: Cow::Borrowed(diagnostic.code),
            message: Cow::Borrowed(diagnostic.message),
            span: HostSpan::from(diagnostic.span),
            severity: Severity::Error,
        })
        .collect();
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

pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::CSS,
    "CSS",
    &[],
    &[".css"],
    &["text/css"],
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

    /// The syntax stream is already context-resolved, so the semantic layer
    /// is the same token list.
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
    use themoretheless_tokenizer_core::HostLanguage;

    /// A representative stylesheet: media query, nested rule, keyframes,
    /// custom properties, `var()`, `calc()` with spaces, `!important`,
    /// pseudo-element, attribute selector and `:nth-child(2n+1)`.
    const THEME: &str = concat!(
        "/* theme */\n",
        ":root {\n",
        "  --brand-red: #b91;\n",
        "  --gutter: 12px;\n",
        "}\n",
        "html, body { margin: 0; padding: var(--gutter); }\n",
        "body.dark h1#title { color: #fff !important; font: 300 16px/1.5 \"Inter\", sans-serif; }\n",
        "a[href^=\"http\"]::after { content: \"\\2197\"; }\n",
        "ul > li:nth-child(2n+1) { background: rgb(0 128 255 / 50%); }\n",
        ".grid { grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); }\n",
        "@media (max-width: 600px) {\n",
        "  .btn:hover::before {\n",
        "    content: \"\\2192\";\n",
        "    width: calc(100% - 4px);\n",
        "  }\n",
        "  .btn & { display: none; }\n",
        "}\n",
        "@keyframes spin {\n",
        "  from { transform: rotate(0deg); }\n",
        "  to { transform: rotate(360deg); }\n",
        "}\n",
        "@font-face {\n",
        "  font-family: \"Inter\";\n",
        "  src: url(\"/fonts/inter.woff2\") format(\"woff2\");\n",
        "}\n",
        "@supports (display: grid) {\n",
        "  .grid { gap: 1rem; }\n",
        "}\n",
    );

    #[test]
    fn valid_stylesheet_produces_no_diagnostics() {
        let parsed = parse(THEME);
        assert_eq!(
            parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| (diagnostic.code, &parsed.source()[diagnostic.span.range()]))
                .collect::<Vec<_>>(),
            Vec::new()
        );
        assert!(parsed.is_valid());
    }

    #[test]
    fn every_sample_rebuilds_its_source() {
        let crlf = "body {\r\n\tcolor: #fff;\r\n}\r\n";
        for source in [
            THEME,
            crlf,
            "body { color: red; }",
            ".перевод { content: \"→\"; color: red; }",
            "a { color: red",
            "a { content: \"unterminated",
            "/*! bang comment */ a { b: c; }",
            "}}}",
            "@media (max-width: 600px) {",
            "",
            "\t\n",
            "a[href$='.pdf']{width:calc( 100% - 2px )}",
        ] {
            let parsed = parse(source);
            assert!(parsed.lexed().verify_lossless().is_ok(), "{source:?}");
            assert_eq!(parsed.lexed().joined(), source, "{source:?}");
        }
    }

    /// The reference sample from the engine brief: kinds and diagnostics are
    /// printed by `cargo test -p … -- --nocapture host_kinds_for_reference_sample`.
    #[test]
    fn host_kinds_for_reference_sample() {
        let source = concat!(
            "/* theme */ :root { --brand: #0a6; }\n",
            "body.dark h1#title { color: #fff !important; font: 300 16px/1.5 \"Inter\", sans-serif; }\n",
            "@media (max-width: 600px) { .btn:hover::before { content: \"→\"; width: calc(100% - 4px); } }\n",
            "@keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }\n",
        );
        let tokenization = ENGINE.lex(source, &HostAnalysisOptions::default()).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.diagnostics.is_empty());
        let mut kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        println!("distinct host kinds ({}):", kinds.len());
        for kind in &kinds {
            println!("  {kind}");
        }
        println!("diagnostics: {}", tokenization.diagnostics.len());
        let parsed = parse(source);
        assert_eq!(parsed.lexed().joined(), source);
    }

    /// Every truncation and every single-byte mangling of the reference sheet
    /// must still lex, stay lossless and make progress.
    #[test]
    fn mangled_input_never_stalls_or_drops() {
        let mut samples = Vec::with_capacity(THEME.len() * 8);
        for cut in 0..THEME.len() {
            samples.push(THEME[..cut].to_string());
            let tail = THEME[..cut].to_string();
            for junk in [
                "\0", "/*", "\"", "'", "@", "}", "{", "[", "(", "!", "#", "$", "\\", "→",
            ] {
                let mut mangled = tail.clone();
                mangled.push_str(junk);
                samples.push(mangled);
            }
        }
        for sample in &samples {
            let parsed = parse(sample);
            assert!(parsed.lexed().verify_lossless().is_ok(), "{sample:?}");
            assert_eq!(parsed.lexed().joined(), sample.as_str(), "{sample:?}");
        }
    }

    #[test]
    fn descriptor_advertises_only_the_real_surface() {
        let wanted = Capabilities::LEX
            | Capabilities::PARSE
            | Capabilities::SEMANTIC
            | Capabilities::VALIDATE;
        assert_eq!(DESCRIPTOR.capabilities, wanted);
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::CST));
        assert_eq!(DESCRIPTOR.extensions, [".css"]);
    }

    #[test]
    fn host_lex_uses_css_kinds() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("body { color: #fff; }", &opts).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.diagnostics.is_empty());
        let kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert!(kinds.contains(&"type-selector"), "{kinds:?}");
        assert!(kinds.contains(&"property"), "{kinds:?}");
        assert!(kinds.contains(&"color"), "{kinds:?}");
    }

    #[test]
    fn host_diagnose_reports_css_codes() {
        let opts = HostAnalysisOptions::default();
        let diagnostics = ENGINE.diagnose("a { color: red }}", &opts).unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unexpected-close-brace"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default().with_limits(
            themoretheless_tokenizer_core::InputLimits::conservative().max_input_bytes(4),
        );
        assert!(matches!(
            ENGINE.lex("body { }", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
    }
}
