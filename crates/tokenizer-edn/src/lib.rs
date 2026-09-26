//! A lossless EDN engine: lexer, recovering structure pass, tag- and
//! key_aware semantic layer.
//!
//! EDN (edn_format) is a data notation: typed values in tagged collections —
//! `#inst` and `#uuid`, radix integers and ratios, `#:ns` namespaced maps,
//! `#_` discarded forms, exact `M` decimals and `N` bigints. This engine owns
//! that vocabulary: a `radix_integer` is not a `number`, a `namespaced_map_prefix`
//! is not `punctuation`, and the two bytes `#{` are one `set_open`, because
//! this is a data_interchange format, not a Lisp language. Commas are
//! whitespace, per the spec. Concatenating every token's text reconstructs
//! the source byte_for_byte, including for malformed input: broken spans are
//! flagged and kept, never dropped or synthesized.
//!
//! ```
//! use themoretheless_tokenizer_edn::{SyntaxKind, parse, validate};
//!
//! let source = "{:name \"grace\" :tags #{:admin :team/ops}}";
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//! assert_eq!(parsed.lexed().joined(), source);
//! assert_eq!(parsed.top_level_forms().len(), 1);
//! let kinds: Vec<SyntaxKind> = parsed.lexed().tokens().iter().map(|t| t.kind).collect();
//! assert!(kinds.contains(&SyntaxKind::SetOpen));
//! assert!(kinds.contains(&SyntaxKind::NamespacedKeyword));
//!
//! // Recovery keeps every byte and names the fault with a stable code.
//! let codes: Vec<&str> = validate("[1 2").iter().map(|d| d.code).collect();
//! assert_eq!(codes, ["unterminated-collection"]);
//! let stray: Vec<&str> = validate("^:meta {}").iter().map(|d| d.code).collect();
//! assert_eq!(stray, ["non-edn-construct"]);
//! ```
//!
//! The semantic layer re_reads eight structure facts without moving a span:
//! a discarded form's tokens, a tag's value token, repeating map keys and set
//! elements, and keywords a `#:ns` prefix will namespace.

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};

pub use parser::{DiagnosticKind, Parse, Retag, parse, validate};

pub use themoretheless_tokenizer_core::{Diagnostic, Span};

// ─── Host adapters ───────────────────────────────────────────────────────────

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

/// Host token kind: the data_notation's own vocabulary, so an editor can tell
/// a keyword from a symbol, a radix integer from a float, `#{` from `{`, and
/// a `#_` from a tag.
fn lex_host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Comment => "comment",
        SyntaxKind::String => "string",
        SyntaxKind::StringEscape => "string-escape",
        SyntaxKind::Character => "character",
        SyntaxKind::Symbol => "symbol",
        SyntaxKind::NamespacedSymbol => "namespaced-symbol",
        SyntaxKind::Keyword => "keyword",
        SyntaxKind::NamespacedKeyword => "namespaced-keyword",
        SyntaxKind::BooleanLiteral => "boolean-literal",
        SyntaxKind::NilLiteral => "nil-literal",
        SyntaxKind::Integer => "integer",
        SyntaxKind::BigInteger => "bigint",
        SyntaxKind::RadixInteger => "radix-integer",
        SyntaxKind::Float => "float",
        SyntaxKind::Decimal => "decimal",
        SyntaxKind::Ratio => "ratio",
        SyntaxKind::SpecialNumber => "special-number",
        SyntaxKind::ListOpen => "list-open",
        SyntaxKind::ListClose => "list-close",
        SyntaxKind::VectorOpen => "vector-open",
        SyntaxKind::VectorClose => "vector-close",
        SyntaxKind::MapOpen => "map-open",
        SyntaxKind::MapClose => "map-close",
        SyntaxKind::SetOpen => "set-open",
        SyntaxKind::Discard => "discard",
        SyntaxKind::Tag => "tag",
        SyntaxKind::InstantTag => "instant-tag",
        SyntaxKind::UuidTag => "uuid-tag",
        SyntaxKind::ByteTag => "byte-tag",
        SyntaxKind::NamespacedMapPrefix => "namespaced-map-prefix",
        SyntaxKind::Error => "error",
    }
}

/// The structure readings the semantic layer may substitute for a token's
/// lexical kind. None of them invents or moves a span.
fn retag_host_kind(retag: Retag) -> &'static str {
    match retag {
        Retag::Discarded => "discarded-form",
        Retag::InstantValue => "instant-value",
        Retag::UuidValue => "uuid-value",
        Retag::ByteValue => "byte-value",
        Retag::TaggedValue => "tagged-value",
        Retag::DuplicateKey => "duplicate-key",
        Retag::DuplicateSetElement => "duplicate-set-element",
        Retag::NamespacedMapKey => "namespaced-map-key",
    }
}

/// Semantic layer: the lex vocabulary plus the structure readings, with the
/// error flag winning over both — an error_flagged token is broken data
/// whatever it was about to be.
fn semantic_host_kind(parsed: &Parse<'_>, index: usize, token: LexToken) -> &'static str {
    if token.has_error() {
        return "error";
    }
    match parsed.retag(index) {
        Some(retag) => retag_host_kind(retag),
        None => lex_host_kind(token.kind),
    }
}

fn tokenization(parsed: &Parse<'_>, semantic: bool) -> HostTokenization {
    let tokens = parsed
        .lexed()
        .tokens()
        .iter()
        .enumerate()
        .map(|(index, token)| {
            let kind = if semantic {
                semantic_host_kind(parsed, index, *token)
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

fn diagnose(
    descriptor: &LanguageDescriptor,
    source: &str,
    host: &HostAnalysisOptions,
) -> Result<Vec<HostDiagnostic>, HostError> {
    Ok(analyze(descriptor, source, host, true)?.diagnostics)
}

/// EDN host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node_identity
/// tree, cursor or visitor here, so none of those capabilities are
/// advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::EDN,
    "edn",
    &["edn"],
    &[".edn"],
    &["application/edn"],
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
        diagnose(&DESCRIPTOR, source, opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use themoretheless_tokenizer_core::{InputLimits, verify_lossless_spans};

    /// Every lex_layer kind the engine may emit.
    const LEX_VOCABULARY: [&str; 33] = [
        "bom",
        "whitespace",
        "comment",
        "string",
        "string-escape",
        "character",
        "symbol",
        "namespaced-symbol",
        "keyword",
        "namespaced-keyword",
        "boolean-literal",
        "nil-literal",
        "integer",
        "bigint",
        "radix-integer",
        "float",
        "decimal",
        "ratio",
        "special-number",
        "list-open",
        "list-close",
        "vector-open",
        "vector-close",
        "map-open",
        "map-close",
        "set-open",
        "discard",
        "tag",
        "instant-tag",
        "uuid-tag",
        "byte-tag",
        "namespaced-map-prefix",
        "error",
    ];

    /// The lex vocabulary plus the eight structure readings the semantic
    /// layer may substitute; it never adds a kind with no span behind it.
    const SEMANTIC_VOCABULARY: [&str; 41] = [
        "bom",
        "whitespace",
        "comment",
        "string",
        "string-escape",
        "character",
        "symbol",
        "namespaced-symbol",
        "keyword",
        "namespaced-keyword",
        "boolean-literal",
        "nil-literal",
        "integer",
        "bigint",
        "radix-integer",
        "float",
        "decimal",
        "ratio",
        "special-number",
        "list-open",
        "list-close",
        "vector-open",
        "vector-close",
        "map-open",
        "map-close",
        "set-open",
        "discard",
        "tag",
        "instant-tag",
        "uuid-tag",
        "byte-tag",
        "namespaced-map-prefix",
        "error",
        "discarded-form",
        "instant-value",
        "uuid-value",
        "byte-value",
        "tagged-value",
        "duplicate-key",
        "duplicate-set-element",
        "namespaced-map-key",
    ];

    /// The eleven kinds a generated engine would emit for any format, kept
    /// verbatim from the repository's wiring gate.
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

    /// A document of the shape real EDN writers produce: tagged values,
    /// radix ints, a ratio, exact decimals, a discarded form, a namespaced
    /// map. It must analyze clean — the repository's format gate.
    const SAMPLE: &str = concat!(
        ";; fleet config\n",
        "{:crew         #{ada grace/link \"grace hopper\" :team/alpha},\n",
        " :launch       #inst \"1985-04-12T23:20:50.52-04:00\",\n",
        " :id           #uuid \"f81d4fae-7dec-11d0-a765-00a0c91e6bf6\",\n",
        " :sigil        #b \"\\u0000\\u007f\",\n",
        " :retry        (16rFF -2r1010 1000N ##Inf),\n",
        " :throttle     3/4,\n",
        " :tuning       [3.25M 6.022e23 42 7M],\n",
        " :separator    [\\newline \\space \\u00e9],\n",
        " :note         #myco/Person {:first \"Lucy\" :last \"Mertz\"},\n",
        " #_[:scratch \"dropped\"],\n",
        " :defaults     #:app{:level :info :window 80}}\n",
    );

    fn distinct<'a>(kinds: &'a [&'a str]) -> Vec<&'a str> {
        let mut names: Vec<&str> = kinds.iter().map(|kind| kind.as_ref()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    fn host_join(source: &str, tokenization: &HostTokenization) -> String {
        let mut joined = String::new();
        for token in &tokenization.tokens {
            joined.push_str(&source[token.span.start..token.span.end]);
        }
        joined
    }

    /// The repository's lossless contract, asserted over the host wire:
    /// `verify_lossless_spans` over the emitted spans and no zero_width
    /// span anywhere.
    fn assert_host_lossless(source: &str, tokenization: &HostTokenization) {
        let spans: Vec<Span> = tokenization
            .tokens
            .iter()
            .map(|token| Span::new(token.span.start, token.span.end))
            .collect();
        verify_lossless_spans(source, spans).unwrap_or_else(|violation| {
            panic!("{source:?} not lossless: {violation:?}");
        });
        for token in &tokenization.tokens {
            assert!(
                token.span.start < token.span.end,
                "{source:?} carried a zero-width host span"
            );
        }
        assert_eq!(host_join(source, tokenization), source);
    }

    fn kinds_of(tokenization: &HostTokenization) -> Vec<&str> {
        tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect()
    }

    #[test]
    fn host_lex_emits_only_the_edn_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        assert!(tokenization.diagnostics.is_empty());
        let names = kinds_of(&tokenization);
        for expected in [
            "symbol",
            "namespaced-symbol",
            "keyword",
            "namespaced-keyword",
            "set-open",
            "instant-tag",
            "uuid-tag",
            "byte-tag",
            "tag",
            "radix-integer",
            "bigint",
            "ratio",
            "decimal",
            "special-number",
            "character",
            "string-escape",
            "namespaced-map-prefix",
            "discard",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        for name in &names {
            assert!(LEX_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    #[test]
    fn host_semantic_emits_only_the_edn_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid);
        let collected = kinds_of(&tokenization);
        let names = distinct(&collected);
        for name in &names {
            assert!(SEMANTIC_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
        // Six of the eight structure readings appear in the fixture (it
        // holds no duplicates by design); none may leak into the syntax
        // layer, and the duplicate readings must be absent from both.
        let syntax_tokens = ENGINE.lex(SAMPLE, &opts).unwrap();
        let syntax = kinds_of(&syntax_tokens);
        for reading in [
            "discarded-form",
            "instant-value",
            "uuid-value",
            "byte-value",
            "tagged-value",
            "namespaced-map-key",
        ] {
            assert!(
                names.contains(&reading),
                "{reading} missing from the semantic layer"
            );
            assert!(
                !syntax.contains(&reading),
                "{reading} leaked into the syntax layer"
            );
        }
        for reading in ["duplicate-key", "duplicate-set-element"] {
            assert!(
                !names.contains(&reading),
                "{reading} is not in a clean fixture"
            );
            assert!(!syntax.contains(&reading));
        }
    }

    /// The exact measured non_generic kind set on the representative
    /// fixture, pinned so a regression back to generic lexing fails here.
    #[test]
    fn sample_non_generic_kinds_are_exactly_the_measured_set() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid);
        let collected = kinds_of(&tokenization);
        let all = distinct(&collected);
        let measured: Vec<&str> = all
            .iter()
            .copied()
            .filter(|kind| !GENERIC_KINDS.contains(kind))
            .collect();
        assert!(
            measured.len() >= 8,
            "only {} non-generic kinds: {measured:?}",
            measured.len()
        );
        assert_eq!(
            measured,
            [
                "bigint",
                "byte-tag",
                "byte-value",
                "character",
                "decimal",
                "discard",
                "discarded-form",
                "float",
                "instant-tag",
                "instant-value",
                "integer",
                "list-close",
                "list-open",
                "map-close",
                "map-open",
                "namespaced-keyword",
                "namespaced-map-key",
                "namespaced-map-prefix",
                "namespaced-symbol",
                "radix-integer",
                "ratio",
                "set-open",
                "special-number",
                "string-escape",
                "symbol",
                "tag",
                "tagged-value",
                "uuid-tag",
                "uuid-value",
                "vector-close",
                "vector-open",
            ],
        );
    }

    #[test]
    fn both_layers_are_lossless_over_the_host_wire() {
        let opts = HostAnalysisOptions::default();
        // The mandated edge shapes: empty input, whitespace_only input, a
        // BOM, a comment at EOF with no newline, an unterminated collection,
        // an unterminated string, non-ASCII symbols and a trailing `#_`.
        let sources = [
            SAMPLE,
            "",
            "  ,\n\t ; trivia only , ",
            "\u{FEFF}",
            "\u{FEFF}{:a 1}",
            "; comment at eof, no newline",
            "[1 2",
            "\"oops",
            "πr²/x 中文/名",
            "1 2 #_",
            "#{:a",
            "#b",
            "\\",
            "#{",
            "#",
        ];
        for source in sources {
            for tokenization in [
                ENGINE.lex(source, &opts).unwrap(),
                ENGINE.semantic_tokens(source, &opts).unwrap(),
            ] {
                assert_host_lossless(source, &tokenization);
            }
        }
    }

    #[test]
    fn sample_fixture_is_diagnostic_free_on_both_layers() {
        let opts = HostAnalysisOptions::default();
        for (layer, tokenization) in [
            ("syntax", ENGINE.lex(SAMPLE, &opts).unwrap()),
            ("semantic", ENGINE.semantic_tokens(SAMPLE, &opts).unwrap()),
        ] {
            assert!(tokenization.valid, "{layer} rejected a valid document");
            assert!(tokenization.diagnostics.is_empty(), "{layer}");
            let specific: Vec<&str> = kinds_of(&tokenization)
                .iter()
                .copied()
                .filter(|kind| !GENERIC_KINDS.contains(kind))
                .collect();
            assert!(!specific.is_empty(), "{layer} emits only generic kinds");
            for expected in [
                "symbol",
                "set-open",
                "radix-integer",
                "namespaced-map-prefix",
            ] {
                assert!(specific.contains(&expected), "{layer}: {specific:?}");
            }
        }
        assert!(parse(SAMPLE).is_valid());
        assert!(validate(SAMPLE).is_empty());
    }

    #[test]
    fn semantic_retags_do_not_move_spans() {
        let opts = HostAnalysisOptions::default();
        for source in [SAMPLE, "{:a 1 :a 2} #{1 1} (1 #_2 3) #my/tag {:x 1}"] {
            let syntax = ENGINE.lex(source, &opts).unwrap();
            let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
            let syntax_spans: Vec<HostSpan> =
                syntax.tokens.iter().map(|token| token.span).collect();
            let semantic_spans: Vec<HostSpan> =
                semantic.tokens.iter().map(|token| token.span).collect();
            assert_eq!(
                syntax_spans, semantic_spans,
                "semantic layer must keep the syntax-layer spans for {source:?}"
            );
            assert_host_lossless(source, &semantic);
        }
        // The duplicate reading really lands on the repeated key.
        let semantic = ENGINE.semantic_tokens("{:a 1 :a 2}", &opts).unwrap();
        let kinds = kinds_of(&semantic);
        assert_eq!(kinds[5], "duplicate-key", "{kinds:?}");
    }

    #[test]
    fn host_diagnose_reports_stable_kebab_case_codes() {
        let opts = HostAnalysisOptions::default();
        let codes = |source: &str| -> String {
            ENGINE
                .diagnose(source, &opts)
                .unwrap()
                .iter()
                .map(|diagnostic| diagnostic.code.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(codes("\"abc"), "unterminated-string");
        assert_eq!(codes("[1 2"), "unterminated-collection");
        assert_eq!(codes("(1 2]"), "mismatched-close");
        assert_eq!(codes(")"), "mismatched-close");
        assert_eq!(codes("2r1020"), "invalid-radix-digit");
        assert_eq!(codes("1r11"), "invalid-radix");
        assert_eq!(codes("\\"), "malformed-char");
        assert_eq!(codes("\\nbsp"), "malformed-char");
        assert_eq!(codes("{:a 1 :a 2}"), "duplicate-key");
        assert_eq!(codes("#{1 1}"), "duplicate-set-element");
        assert_eq!(codes("(#my/tag)"), "tag-without-value");
        assert_eq!(codes("[1 #_]"), "discard-without-value");
        assert_eq!(codes("@form"), "non-edn-construct");
        assert_eq!(codes("^:meta {}"), "non-edn-construct");
        assert_eq!(codes("#(inc 1)"), "non-edn-construct");
        assert_eq!(codes("::nskw"), "non-edn-construct");
        assert_eq!(codes("#inst 1"), "invalid-tagged-value");
        assert_eq!(codes("{:a}"), "odd-map-entries");
        assert_eq!(codes("\"\\q\""), "invalid-escape");
        assert_eq!(codes("01"), "invalid-number");
        assert_eq!(codes(":"), "invalid-symbol");
        assert_eq!(codes("#"), "incomplete-dispatch");
        assert_eq!(codes(""), "");
        assert_eq!(codes(SAMPLE), "");
        for code in codes("[1 \"oops ^x {:a 1 1} 2r40 \\nbsp").split(',') {
            assert!(
                !code.is_empty()
                    && code.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'-'),
                "code {code:?} is not kebab-case"
            );
        }
    }

    #[test]
    fn every_mandated_fault_reaches_the_host_wire() {
        let opts = HostAnalysisOptions::default();
        let mut seen: Vec<String> = Vec::new();
        for source in [
            "\"abc",
            "[1 2",
            "(1 2]",
            "2r1020",
            "\\",
            "{:a 1 :a 2}",
            "(#my/tag)",
            "[1 #_]",
            "@form",
            "^x {}",
            "`t",
            "'x",
            "~f",
            "#(1)",
            "#'v",
            "{:a}",
        ] {
            let tokenization = ENGINE.semantic_tokens(source, &opts).unwrap();
            assert!(!tokenization.valid, "{source:?} must not validate");
            assert_host_lossless(source, &tokenization);
            for diagnostic in &tokenization.diagnostics {
                seen.push(diagnostic.code.to_string());
            }
            assert!(
                tokenization
                    .tokens
                    .iter()
                    .any(|token| token.error && token.kind.as_ref() == "error")
                    || !tokenization.diagnostics.is_empty(),
                "{source:?} carries neither an error token nor a diagnostic"
            );
        }
        for required in [
            "unterminated-string",
            "unterminated-collection",
            "mismatched-close",
            "invalid-radix-digit",
            "malformed-char",
            "duplicate-key",
            "tag-without-value",
            "discard-without-value",
            "non-edn-construct",
        ] {
            assert!(
                seen.iter().any(|code| code == required),
                "{required} unreachable"
            );
        }
    }

    #[test]
    fn malformed_documents_surface_as_error_kinds_and_codes() {
        let opts = HostAnalysisOptions::default();
        let source = "{:a 1 :a [2 \"oops ^meta #(f)]}";
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(!tokenization.valid);
        assert!(
            tokenization
                .tokens
                .iter()
                .any(|token| token.kind.as_ref() == "error" && token.error),
            "{:?}",
            tokenization.tokens
        );
        for name in &kinds_of(&tokenization) {
            assert!(LEX_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    #[test]
    fn broken_inputs_never_stall_the_host() {
        let opts = HostAnalysisOptions::default();
        for source in [
            "#", "\\", "#{", "#b", "#_", "#:", "^", "@", "'", "`", "~", "#'", "#=", "##", ":",
            "::", "01", ".5", "1e", "2r", "\\u12", "#inst", "#:", ",", "#my",
        ] {
            let lexed = ENGINE.lex(source, &opts).unwrap();
            let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
            assert_host_lossless(source, &lexed);
            assert_host_lossless(source, &semantic);
            assert!(ENGINE.diagnose(source, &opts).is_ok());
        }
    }

    /// The spec defines no enclosing element at top level — EDN is suitable
    /// for streaming — so a document of several forms is accepted without
    /// warning.
    #[test]
    fn top_level_streams_of_forms_are_accepted() {
        let opts = HostAnalysisOptions::default();
        for source in ["1 2 3", "(a) [b] #{c}", ":kw\nnil"] {
            let tokenization = ENGINE.lex(source, &opts).unwrap();
            assert!(
                tokenization.valid,
                "{source:?}: {:?}",
                tokenization.diagnostics
            );
            assert!(tokenization.diagnostics.is_empty());
        }
        let parsed = parse("1 2 3");
        assert_eq!(parsed.top_level_forms().len(), 3);
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
        assert_eq!(DESCRIPTOR.language, LanguageId::EDN);
        assert_eq!(DESCRIPTOR.display_name, "edn");
        assert_eq!(DESCRIPTOR.extensions, [".edn"]);
        assert_eq!(DESCRIPTOR.mime_types, ["application/edn"]);
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default()
            .with_limits(InputLimits::conservative().max_input_bytes(4));
        assert!(matches!(
            ENGINE.lex("{:a 1}", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(matches!(
            ENGINE.semantic_tokens("{:a 1}", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(matches!(
            ENGINE.diagnose("{:a 1}", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(ENGINE.lex("nil", &opts).is_ok());
        assert!(ENGINE.lex("", &opts).is_ok());
    }

    #[test]
    fn empty_input_is_valid_and_carries_no_tokens() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("", &opts).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.tokens.is_empty());
        assert!(tokenization.diagnostics.is_empty());
        assert_eq!(parse("").top_level_forms().len(), 0);
    }
}
