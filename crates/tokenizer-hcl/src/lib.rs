//! A lossless HCL engine: lexer, recovering structure pass, role-aware
//! semantic layer.
//!
//! HCL is the configuration language of the HashiCorp toolchain: a body of
//! `name = expression` attributes and `type "label" { ... }` blocks, with
//! `#`, `//` and `/* */` comments, template strings carrying `${...}`
//! interpolations and escape sequences, and heredocs introduced by `<<TAG`.
//! This engine owns its vocabulary — `attribute-name`, `block-type`,
//! `block-label`, `heredoc-open`, `interpolation`, `boolean`, `null`,
//! `newline` — because a generic token stream cannot tell a block label from
//! a value string. Concatenating every token's text reconstructs the source
//! byte-for-byte, including for malformed input: bad spans are flagged,
//! never dropped or synthesized, and every lexer advance guarantees progress,
//! so no document can hang or panic the engine.
//!
//! ```
//! use themoretheless_tokenizer_hcl::{SyntaxKind, parse, validate};
//!
//! let source = "service \"api\" {\n  port = 8080\n}\n";
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//! let first = parsed.lexed().tokens()[0];
//! assert_eq!(first.kind, SyntaxKind::Identifier);
//! assert!(parsed.is_block_type(first.span));
//! assert_eq!(first.text(parsed.lexed().source()), Some("service"));
//!
//! // Recovery keeps every byte and names the fault with a stable code.
//! let codes: Vec<&str> = validate("a = \"oops").iter().map(|d| d.code).collect();
//! assert_eq!(codes, ["unterminated-string"]);
//! let heredoc: Vec<&str> = validate("b = <<EOT\nnever").iter().map(|d| d.code).collect();
//! assert_eq!(heredoc, ["unterminated-heredoc"]);
//! ```
//!
//! The semantic layer re-tags tokens by the role the structure pass found,
//! without moving a single span: the `service` above reads as `block-type`,
//! `"api"` as `block-label`, and a `port` before `=` as `attribute-name`.
//! `true`, `false` and `null` are literal kinds of their own from the lexer
//! up, and newlines stay distinct from blanks because HCL delimits items by
//! line.

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};

pub use parser::{DiagnosticKind, Parse, parse, validate};

pub use themoretheless_tokenizer_core::{Diagnostic, Span};

use themoretheless_tokenizer_core::LanguageId;

// ─── Host adapters ───────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, Severity, language_descriptor,
    require_default_dialect,
};

/// Real engine surface: no CST, cursor navigation or visitor API exists here.
const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC)
    .union(Capabilities::VALIDATE);

/// Lex-layer host kind for a syntax category.
fn lex_host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::LineComment => "line-comment",
        SyntaxKind::BlockComment => "block-comment",
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Newline => "newline",
        SyntaxKind::Identifier => "identifier",
        SyntaxKind::Number => "number",
        SyntaxKind::Boolean => "boolean",
        SyntaxKind::Null => "null",
        SyntaxKind::String => "string",
        SyntaxKind::Escape => "escape",
        SyntaxKind::Interpolation => "interpolation",
        SyntaxKind::HeredocOpen => "heredoc-open",
        SyntaxKind::HeredocBody => "heredoc-body",
        SyntaxKind::HeredocClose => "heredoc-close",
        SyntaxKind::Operator => "operator",
        SyntaxKind::Punctuation => "punctuation",
        SyntaxKind::Error => "error",
    }
}

/// Semantic layer: the syntax kinds plus the roles only the structure pass
/// can see, applied without moving a span. A flagged token always degrades to
/// `error`, so a fault never keeps a confident role.
fn semantic_host_kind(parsed: &Parse<'_>, token: LexToken) -> &'static str {
    if token.has_error() {
        return "error";
    }
    match token.kind {
        SyntaxKind::Identifier if parsed.is_attribute_name(token.span) => "attribute-name",
        SyntaxKind::Identifier if parsed.is_block_type(token.span) => "block-type",
        SyntaxKind::String if parsed.is_block_label(token.span) => "block-label",
        kind => lex_host_kind(kind),
    }
}

fn tokenization(parsed: &Parse<'_>, semantic: bool) -> HostTokenization {
    let tokens = parsed
        .lexed()
        .tokens()
        .iter()
        .map(|token| {
            let kind = if semantic {
                semantic_host_kind(parsed, *token)
            } else if token.has_error() {
                "error"
            } else {
                lex_host_kind(token.kind)
            };
            HostToken {
                kind: Cow::Borrowed(kind),
                span: HostSpan::from(token.span),
                error: token.has_error(),
            }
        })
        .collect();
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

fn analyze(
    descriptor: &LanguageDescriptor,
    source: &str,
    host: &HostAnalysisOptions,
    semantic: bool,
) -> Result<HostTokenization, HostError> {
    require_default_dialect(descriptor, host.dialect.as_ref())?;
    if host.limits.exceeds_input_bytes(source.len()) {
        return Err(HostError::InputTooLarge {
            max: host.limits.max_input_bytes,
            actual: source.len(),
        });
    }
    Ok(tokenization(&parse(source), semantic))
}

/// HCL host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity
/// tree, cursor, or visitor here, so none of those capabilities are
/// advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::HCL,
    "hcl",
    &["hcl"],
    &[".hcl", ".tf"],
    &["text/x-hcl"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(analyze(&DESCRIPTOR, source, opts, true)?.diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use themoretheless_tokenizer_core::{HostAnalysisOptions, InputLimits, verify_lossless_spans};

    /// Every lex-layer kind the engine may emit.
    const LEX_VOCABULARY: [&str; 18] = [
        "bom",
        "line-comment",
        "block-comment",
        "whitespace",
        "newline",
        "identifier",
        "number",
        "boolean",
        "null",
        "string",
        "escape",
        "interpolation",
        "heredoc-open",
        "heredoc-body",
        "heredoc-close",
        "operator",
        "punctuation",
        "error",
    ];
    /// The lex vocabulary plus the three readings the semantic layer may
    /// substitute; it never adds a kind with no span behind it.
    const SEMANTIC_VOCABULARY: [&str; 21] = [
        "bom",
        "line-comment",
        "block-comment",
        "whitespace",
        "newline",
        "identifier",
        "attribute-name",
        "block-type",
        "block-label",
        "number",
        "boolean",
        "null",
        "string",
        "escape",
        "interpolation",
        "heredoc-open",
        "heredoc-body",
        "heredoc-close",
        "operator",
        "punctuation",
        "error",
    ];

    /// The eleven kinds a generated engine would emit for any format.
    const GENERIC_KINDS: [&str; 11] = [
        "class",
        "comment",
        "function",
        "identifier",
        "keyword",
        "number",
        "punctuation",
        "string",
        "type",
        "variable",
        "whitespace",
    ];

    /// A document of the shape real HCL writers produce. It must analyze
    /// clean on both layers: this is the gate the repository's format suite
    /// applies.
    const SAMPLE: &str = concat!(
        "# cluster settings\n",
        "service \"api\" {\n",
        "  enabled  = true\n",
        "  port     = 8080\n",
        "  latency  = -1.5e3\n",
        "  ratio    = 0.25\n",
        "  owner    = null\n",
        "  tags     = [\"web\", \"edge\"]\n",
        "  defaults = { cpu = 2 }\n",
        "  greeting = \"hello ${local.name}!\"\n",
        "  note     = \"said \\\"hi\\\"\\n\"\n",
        "  banner   = <<EOT\n",
        "    cluster ${local.region} online\n",
        "    EOT\n",
        "  toggle   = enabled ? \"on\" : \"off\"\n",
        "  /* trailing note */\n",
        "}\n",
    );

    fn distinct<'a>(kinds: &'a [&'a str]) -> Vec<&'a str> {
        let mut names: Vec<&str> = kinds.iter().map(|kind| kind.as_ref()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    fn kinds_of(tokenization: &HostTokenization) -> Vec<&str> {
        tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect()
    }

    fn host_source_kinds(host: Host, source: &str) -> Vec<String> {
        let opts = HostAnalysisOptions::default();
        let diagnostics = host.diagnose(source, &opts).unwrap();
        let mut codes: Vec<String> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.to_string())
            .collect();
        codes.sort_unstable();
        codes.dedup();
        codes
    }

    #[test]
    fn host_lex_emits_only_the_hcl_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        assert!(tokenization.diagnostics.is_empty());
        let names = kinds_of(&tokenization);
        for expected in [
            "attribute-name",
            "block-type",
            "block-label",
            "boolean",
            "null",
            "heredoc-open",
            "heredoc-body",
            "heredoc-close",
            "interpolation",
            "escape",
            "line-comment",
            "block-comment",
            "newline",
            "operator",
        ] {
            // The lex layer must carry everything except the three roles the
            // semantic pass introduces.
            if matches!(expected, "attribute-name" | "block-type" | "block-label") {
                assert!(!names.contains(&expected), "{expected} is semantic-only");
                continue;
            }
            assert!(names.contains(&expected), "missing {expected}");
        }
        for name in &names {
            assert!(LEX_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    #[test]
    fn host_semantic_emits_only_the_hcl_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        let collected = kinds_of(&tokenization);
        let names = distinct(&collected);
        for expected in [
            "attribute-name",
            "block-type",
            "block-label",
            "boolean",
            "null",
            "interpolation",
            "heredoc-open",
            "heredoc-body",
            "heredoc-close",
            "escape",
            "newline",
            "operator",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        for name in &names {
            assert!(SEMANTIC_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    /// The exact set of non-generic kinds the representative fixture must
    /// produce on the semantic layer, measured from the engine.
    #[test]
    fn fixture_emits_exactly_this_specific_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        let all = kinds_of(&tokenization);
        let specific: Vec<&str> = all
            .into_iter()
            .filter(|kind| !GENERIC_KINDS.contains(kind))
            .collect();
        let measured = distinct(&specific);
        assert_eq!(
            measured,
            vec![
                "attribute-name",
                "block-comment",
                "block-label",
                "block-type",
                "boolean",
                "escape",
                "heredoc-body",
                "heredoc-close",
                "heredoc-open",
                "interpolation",
                "line-comment",
                "newline",
                "null",
                "operator",
            ]
        );
        assert!(
            measured.len() >= 8,
            "the gate wants eight, got {measured:?}"
        );
    }

    #[test]
    fn both_layers_are_lossless_over_the_host_wire() {
        let opts = HostAnalysisOptions::default();
        for source in [
            "",
            "   \t  ",
            "\u{FEFF}",
            "\u{FEFF}a = 1\r\nb = 2\r\n",
            "a = 1\r\n",
            "note = \"名前 🚀\"\n",
            "a = \"oops",
            "b = <<EOT\nnever ends\n",
            "block {",
            "a = \"${x",
            "$ = 1",
            "= 2",
            "a = ,",
            "}",
            "]",
        ] {
            for tokenization in [
                ENGINE.lex(source, &opts).unwrap(),
                ENGINE.semantic_tokens(source, &opts).unwrap(),
            ] {
                let spans: Vec<Span> = tokenization
                    .tokens
                    .iter()
                    .map(|token| Span::from(token.span))
                    .collect();
                verify_lossless_spans(source, spans).unwrap_or_else(|violation| {
                    panic!("{source:?} did not round-trip: {violation:?}")
                });
                for token in &tokenization.tokens {
                    assert!(
                        token.span.start < token.span.end,
                        "{source:?} carried a zero-width host span"
                    );
                }
            }
        }
    }

    /// The robustness gate: none of these may panic or hang, and each keeps a
    /// full byte-for-byte token coverage while reporting its fault.
    #[test]
    fn malformed_input_never_panics_and_always_reports() {
        let opts = HostAnalysisOptions::default();
        for (source, expected_code) in [
            ("\"", "unterminated-string"),
            ("<<EOT", "unterminated-heredoc"),
            ("{", "unclosed-brace"),
            ("$", "unexpected-token"),
            ("${", "unexpected-token"),
            ("a = \"${", "unterminated-interpolation"),
            ("a = \"x", "unterminated-string"),
            ("/*", "unterminated-comment"),
            ("a = [1", "unclosed-bracket"),
        ] {
            let lexed = ENGINE.lex(source, &opts).unwrap();
            let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
            assert!(!lexed.valid, "{source:?} must not validate");
            assert!(!semantic.valid, "{source:?} must not validate");
            let codes: Vec<&str> = lexed
                .diagnostics
                .iter()
                .map(|diagnostic| &*diagnostic.code)
                .collect();
            assert!(codes.contains(&expected_code), "{source:?}: {codes:?}");
            verify_lossless_spans(source, lexed.tokens.iter().map(|t| Span::from(t.span)))
                .unwrap_or_else(|violation| panic!("{source:?}: {violation:?}"));
        }
        // Odd but grammantically legal shapes: they must not panic, hang, or
        // lose a byte, whatever the parser decides about them.
        for source in ["=", ",", "= 2", "a = , 1", "]", "}", "[", "(", ")"] {
            let lexed = ENGINE.lex(source, &opts).unwrap();
            let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
            for tokenization in [&lexed, &semantic] {
                verify_lossless_spans(
                    source,
                    tokenization.tokens.iter().map(|t| Span::from(t.span)),
                )
                .unwrap_or_else(|violation| panic!("{source:?}: {violation:?}"));
            }
        }
    }

    #[test]
    fn semantic_tokens_retag_roles_without_moving_spans() {
        let opts = HostAnalysisOptions::default();
        let source = "service \"api\" { x = 1 }\n";
        let syntax = ENGINE.lex(source, &opts).unwrap();
        let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
        assert!(semantic.valid, "{:?}", semantic.diagnostics);
        assert_eq!(
            syntax
                .tokens
                .iter()
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            semantic
                .tokens
                .iter()
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            "the semantic layer must keep the syntax-layer spans"
        );
        let names = kinds_of(&semantic);
        assert_eq!(
            names,
            vec![
                "block-type",
                "whitespace",
                "block-label",
                "whitespace",
                "punctuation",
                "whitespace",
                "attribute-name",
                "whitespace",
                "operator",
                "whitespace",
                "number",
                "whitespace",
                "punctuation",
                "newline",
            ]
        );
        // The syntax layer never claims the roles.
        assert_eq!(kinds_of(&syntax)[0], "identifier");
        assert_eq!(kinds_of(&syntax)[2], "string");
        assert_eq!(kinds_of(&syntax)[6], "identifier");
    }

    #[test]
    fn flagged_tokens_degrade_to_error_on_both_layers() {
        let opts = HostAnalysisOptions::default();
        let source = "a = \"oops";
        for tokenization in [
            ENGINE.lex(source, &opts).unwrap(),
            ENGINE.semantic_tokens(source, &opts).unwrap(),
        ] {
            assert!(
                tokenization
                    .tokens
                    .iter()
                    .any(|token| token.error && token.kind.as_ref() == "error"),
                "{:?}",
                tokenization.tokens
            );
        }
    }

    #[test]
    fn host_diagnose_reports_only_hcl_codes() {
        assert_eq!(
            host_source_kinds(ENGINE, "a = \"oops"),
            ["unterminated-string"]
        );
        assert_eq!(
            host_source_kinds(ENGINE, "b = <<EOT\nnone"),
            ["unterminated-heredoc"]
        );
        assert_eq!(host_source_kinds(ENGINE, "a = { b = 1"), ["unclosed-brace"]);
        assert_eq!(host_source_kinds(ENGINE, "a = [1"), ["unclosed-bracket"]);
        assert_eq!(host_source_kinds(ENGINE, "a = \"\\q\""), ["invalid-escape"]);
        assert_eq!(host_source_kinds(ENGINE, "a = 1 }"), ["unexpected-token"]);
        assert_eq!(
            host_source_kinds(ENGINE, "a = \"${x"),
            ["unterminated-interpolation"]
        );
        assert_eq!(
            host_source_kinds(ENGINE, "/* oops"),
            ["unterminated-comment"]
        );
        assert!(host_source_kinds(ENGINE, SAMPLE).is_empty());
        assert!(host_source_kinds(ENGINE, "").is_empty());
    }

    #[test]
    fn descriptor_advertises_only_the_real_surface() {
        assert_eq!(DESCRIPTOR.capabilities, CAPABILITIES);
        assert!(DESCRIPTOR.capabilities.contains(
            Capabilities::LEX
                | Capabilities::PARSE
                | Capabilities::SEMANTIC
                | Capabilities::VALIDATE
        ));
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::CST));
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::NAVIGATE));
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::VISITOR));
        assert_eq!(DESCRIPTOR.engine_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(DESCRIPTOR.language, LanguageId::HCL);
        assert_eq!(DESCRIPTOR.display_name, "hcl");
        assert_eq!(DESCRIPTOR.extensions, [".hcl", ".tf"]);
        assert_eq!(DESCRIPTOR.mime_types, ["text/x-hcl"]);
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default()
            .with_limits(InputLimits::conservative().max_input_bytes(4));
        assert!(matches!(
            ENGINE.lex("a = 8080\n", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(matches!(
            ENGINE.semantic_tokens("a = 8080\n", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(matches!(
            ENGINE.diagnose("a = 8080\n", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(ENGINE.lex("a=1\n", &opts).is_ok());
        assert!(ENGINE.lex("", &opts).is_ok());
    }

    #[test]
    fn empty_input_is_valid_and_carries_no_tokens() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("", &opts).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.tokens.is_empty());
        assert!(tokenization.diagnostics.is_empty());
    }

    /// The engine may only claim kinds it can actually emit: every name in
    /// the declared vocabularies must appear in the union of the fixture and
    /// a few probe documents, and nothing measured may escape them.
    #[test]
    fn declared_vocabularies_are_backed_by_real_tokens() {
        let opts = HostAnalysisOptions::default();
        let probes = [SAMPLE, "a = \"oops", "b = <<EOT\nnone\n", "/* x", "$"];
        let mut lex_kinds: Vec<String> = Vec::new();
        let mut sem_kinds: Vec<String> = Vec::new();
        for source in probes {
            let syntax = ENGINE.lex(source, &opts).unwrap();
            let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
            lex_kinds.extend(syntax.tokens.iter().map(|token| token.kind.to_string()));
            sem_kinds.extend(semantic.tokens.iter().map(|token| token.kind.to_string()));
        }
        lex_kinds.sort();
        lex_kinds.dedup();
        sem_kinds.sort();
        sem_kinds.dedup();
        for name in &lex_kinds {
            assert!(LEX_VOCABULARY.contains(&name.as_str()), "undeclared {name}");
        }
        for name in &sem_kinds {
            assert!(
                SEMANTIC_VOCABULARY.contains(&name.as_str()),
                "undeclared semantic {name}"
            );
        }
    }
}
