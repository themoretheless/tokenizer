//! A lossless SubRip and WebVTT engine: one dialect-parameterised lexer, a
//! recovering cue pass, and a structure-aware semantic layer.
//!
//! WebVTT is a dialect of the same shape, so both engines live here and differ
//! only through [`Options`] — on rules that change what the bytes mean, not
//! just what they are called. A bare integer line is the cue's ordinal index in
//! SubRip and an opaque `cue-id` in WebVTT, which has no index concept at all;
//! `WEBVTT`, `NOTE`, `STYLE` and `REGION` exist only in WebVTT, and SubRip
//! reads those words as cue text and reports the block as unfinished;
//! `mm:ss.SSS` is a timestamp in WebVTT and a malformed one in SubRip, which
//! requires hours and a `,` before exactly three fractional digits; a timing
//! line may carry `align:`/`position:` settings in WebVTT and nothing at all in
//! SubRip. The engine owns its vocabulary — there is no `number`, no `string`
//! and no `punctuation` kind, because in timed text the structural names are
//! `millisecond`, `timing-arrow` and `cue-text`. Concatenating every token's
//! text reconstructs the source byte-for-byte, including for malformed input:
//! bad spans are flagged, never dropped or synthesized.
//!
//! ```
//! use themoretheless_tokenizer_srt::{Options, SyntaxKind, parse};
//!
//! let source = "1\n00:00:01,000 --> 00:00:04,000\nHello.\n";
//! let parsed = parse(source, Options::SRT);
//! assert!(parsed.is_valid());
//! assert_eq!(parsed.cues().len(), 1);
//! assert_eq!(parsed.lexed().joined(), source);
//!
//! let kinds = parsed.lexed().tokens().iter().map(|t| t.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::CueIndex));
//! assert!(kinds.contains(&SyntaxKind::TimingArrow));
//!
//! // The same bytes under WebVTT rules lack the signature and read their
//! // comma-separated timestamps as malformed.
//! let webvtt = parse(source, Options::VTT);
//! assert_eq!(webvtt.cues().len(), 1);
//! let codes = webvtt
//!     .diagnostics()
//!     .iter()
//!     .map(|d| d.code)
//!     .collect::<Vec<_>>();
//! assert!(codes.contains(&"missing-signature"), "{codes:?}");
//! assert!(codes.contains(&"malformed-timestamp"), "{codes:?}");
//! ```
//!
//! The host adapters add the format-specific wire kinds, and the semantic layer
//! reads the document rather than the token: it tells a cue's start timestamp
//! from its end timestamp, a cue's first text line from its continuations, and
//! a `00:00:24.000` that is a cue's own timing from one that sits inside a
//! cue-text `<00:00:24.000>` tag — while every span stays identical to the
//! syntax layer's.
//!
//! ```
//! use themoretheless_tokenizer_srt::{ENGINE, VTT_ENGINE};
//! use themoretheless_tokenizer_core::HostAnalysisOptions;
//!
//! let opts = HostAnalysisOptions::default();
//! let kinds = |engine: &dyn themoretheless_tokenizer_core::HostLanguage, source: &str| {
//!     engine
//!         .semantic_tokens(source, &opts)
//!         .unwrap()
//!         .tokens
//!         .iter()
//!         .map(|token| token.kind.to_string())
//!         .collect::<Vec<_>>()
//! };
//! assert!(kinds(&ENGINE, "1\n00:00:01,000 --> 00:00:04,000\nHi\nthere\n").contains(
//!     &"cue-text-continuation".to_string()
//! ));
//! assert!(kinds(&VTT_ENGINE, "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n").contains(&"cue-end".to_string()));
//! assert!(!kinds(&ENGINE, "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n").contains(&"signature".to_string()));
//! ```

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{Dialect, LexToken, Lexed, Options, SyntaxKind, TokenFlags, lex};

pub use parser::{Cue, DiagnosticKind, HeaderField, Parse, Setting, Timestamp, parse, validate};

pub use themoretheless_tokenizer_core::{Diagnostic, Span};

// ─── Host adapters ───────────────────────────────────────────────────────────

use std::borrow::Cow;
use std::ops::Range;

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

/// Semantic-layer state that no single token holds: where in its cue this line
/// sits, which timestamp of the timing line is being read, and which setting or
/// header field a value belongs to.
struct Semantic<'source> {
    /// Lines of cue text the current cue has had so far.
    text_line: usize,
    /// Which timestamp of the current timing line the reader has reached.
    stamp: usize,
    /// Name of the WebVTT header field whose value is being read.
    header: Option<&'source str>,
    /// Name of the cue setting whose value is being read.
    setting: Option<&'source str>,
}

impl Semantic<'_> {
    const fn fresh() -> Self {
        Self {
            text_line: 0,
            stamp: 0,
            header: None,
            setting: None,
        }
    }
}

/// What one token means in its document, given everything the lexer had to read
/// around it to say so.
fn semantic_kind<'source>(
    token: LexToken,
    source: &'source str,
    state: &mut Semantic<'source>,
) -> &'static str {
    let text = || token.text(source).unwrap_or_default();
    if token.has_error() {
        return "error";
    }
    if token.in_markup() && token.in_timestamp() {
        return "timing-tag";
    }
    match token.kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::RecordBreak => "record-break",
        SyntaxKind::Error => "error",
        SyntaxKind::Signature => "signature",
        SyntaxKind::SignatureComment => "signature-note",
        SyntaxKind::HeaderName => {
            state.header = Some(text().trim());
            "header-name"
        }
        SyntaxKind::HeaderSeparator => "header-separator",
        SyntaxKind::HeaderValue => match state.header {
            Some("Region") => "region-reference",
            Some("Style") => "style-reference",
            _ => "header-value",
        },
        SyntaxKind::BlockMarker => "block-marker",
        SyntaxKind::Comment => "comment",
        SyntaxKind::StyleContent => "style-rule",
        SyntaxKind::RegionProperty => "region-property",
        SyntaxKind::RegionSeparator => "region-separator",
        SyntaxKind::RegionValue => "region-value",
        SyntaxKind::CueIndex => "cue-index",
        SyntaxKind::CueIdentifier => "cue-identifier",
        SyntaxKind::CueId => "cue-id",
        SyntaxKind::CueText => {
            if state.text_line == 0 {
                "cue-text"
            } else {
                "cue-text-continuation"
            }
        }
        SyntaxKind::TimingArrow => "timing-arrow",
        // Every field of a timing line's timestamp means the same thing: the
        // instant the cue opens at, or the one it closes at.
        _ if token.is_timing_field() => match state.stamp {
            1 => "cue-start",
            2 => "cue-end",
            _ => "cue-timing",
        },
        SyntaxKind::SettingName => {
            state.setting = Some(text().trim());
            "setting-name"
        }
        SyntaxKind::SettingSeparator => "setting-separator",
        SyntaxKind::SettingValue => match state.setting {
            Some("align") => "alignment-value",
            Some("position") => "position-value",
            Some("size") => "size-value",
            Some("line") => "line-value",
            Some("vertical") => "vertical-value",
            Some("region") => "region-reference",
            _ => "setting-value",
        },
        SyntaxKind::MarkupPunctuation => "markup-punctuation",
        SyntaxKind::MarkupName => {
            let name = text();
            if name.starts_with('/') {
                "closing-tag"
            } else {
                // A class tag writes its class into the name: `<c.highlight>`.
                match name.split('.').next().unwrap_or_default() {
                    "v" => "voice-tag",
                    "c" => "class-tag",
                    "i" | "b" | "u" => "emphasis-tag",
                    _ => "inline-tag",
                }
            }
        }
        SyntaxKind::MarkupValue => "markup-value",
        // A kind a future dialect adds: read it as written.
        _ => token.kind.host_kind(),
    }
}

/// The semantic layer: the syntax vocabulary re-read as a document, so a host
/// gets `cue-start` for the instant a cue opens at and `cue-timing` for the
/// identical bytes inside a `<00:00:24.000>` tag. Every token keeps its span,
/// so the two layers agree byte for byte.
fn semantic_kinds(source: &str, tokens: &[LexToken], lines: &[Range<usize>]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::with_capacity(tokens.len());
    let mut state = Semantic::fresh();
    for range in lines {
        let line = &tokens[range.clone()];
        let blank = line.iter().all(|token| token.kind.is_trivia());
        let timing = !blank && parser::is_timing_line(line);
        if blank {
            state = Semantic::fresh();
        } else if timing {
            state.stamp = 0;
            state.text_line = 0;
            state.setting = None;
        }
        for token in line {
            if token.is_timing_field() && token.is_timestamp_head() {
                state.stamp += 1;
            }
            out.push(semantic_kind(*token, source, &mut state));
        }
        if !timing {
            state.text_line += 1;
        }
    }
    debug_assert_eq!(out.len(), tokens.len(), "one semantic kind per token");
    out
}

fn tokenization(parsed: &Parse<'_>, semantic: bool) -> HostTokenization {
    let source = parsed.lexed().source();
    let tokens = parsed.lexed().tokens();
    let kinds: Vec<&'static str> = if semantic {
        semantic_kinds(source, tokens, &parser::line_ranges(tokens))
    } else {
        tokens.iter().map(|token| token.kind.host_kind()).collect()
    };
    let host_tokens = tokens
        .iter()
        .zip(kinds)
        .map(|(token, kind)| HostToken {
            kind: Cow::Borrowed(kind),
            span: HostSpan::from(token.span),
            error: token.has_error(),
        })
        .collect();
    let diagnostics = parsed
        .diagnostics()
        .iter()
        .map(|diagnostic| HostDiagnostic {
            code: Cow::Borrowed(diagnostic.code),
            message: Cow::Borrowed(diagnostic.message),
            span: HostSpan::from(diagnostic.span),
            severity: DiagnosticKind::from_code(diagnostic.code)
                .map_or(Severity::Error, |kind| kind.severity()),
        })
        .collect();
    HostTokenization {
        tokens: host_tokens,
        diagnostics,
        valid: parsed.is_valid(),
    }
}

fn analyze(
    descriptor: &LanguageDescriptor,
    options: Options,
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
    Ok(tokenization(&parse(source, options), semantic))
}

fn diagnose(
    descriptor: &LanguageDescriptor,
    options: Options,
    source: &str,
    host: &HostAnalysisOptions,
) -> Result<Vec<HostDiagnostic>, HostError> {
    Ok(analyze(descriptor, options, source, host, true)?.diagnostics)
}

/// SubRip host adapter: index lines, `hh:mm:ss,mmm --> hh:mm:ss,mmm`, prose.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

/// WebVTT host adapter: the `WEBVTT` header, block types, cue settings, tags.
#[derive(Debug, Default, Clone, Copy)]
pub struct VttHost;

pub static ENGINE: Host = Host;

pub static VTT_ENGINE: VttHost = VttHost;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity tree,
/// cursor, or visitor here, so none of those capabilities are advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::SRT,
    "SubRip",
    &["srt", "subrip"],
    &[".srt"],
    &["application/x-subrip"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

/// The same capability surface as SubRip, under the WebVTT identity.
pub static VTT_DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::VTT,
    "WebVTT",
    &["vtt", "webvtt"],
    &[".vtt"],
    &["text/vtt"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, Options::SRT, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, Options::SRT, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&DESCRIPTOR, Options::SRT, source, opts)
    }
}

impl HostLanguage for VttHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &VTT_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&VTT_DESCRIPTOR, Options::VTT, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&VTT_DESCRIPTOR, Options::VTT, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&VTT_DESCRIPTOR, Options::VTT, source, opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SubRip's representative document: a BOM, an ordinal index line, a
    /// non-integer identity line, two text lines in one cue, em dashes and an
    /// emoji, and markup that SubRip has no grammar for.
    const SRT_SAMPLE: &str = concat!(
        "\u{FEFF}1\n",
        "00:00:01,000 --> 00:00:04,000\n",
        "Hello, world \u{2014} bonjour!\n",
        "Second line of the first cue.\n",
        "\n",
        "2\n",
        "00:00:04,500 --> 00:00:09,000\n",
        "\u{1F3AC} Cue two: <i>tags stay text</i> here.\n",
        "\n",
        "coda\n",
        "00:00:09,000 --> 00:00:12,000\n",
        "Final cue \u{2014} the end.\n",
    );

    /// WebVTT's representative document: the signature with its note, both
    /// header field kinds, all three block types, a voice tag, a class tag, an
    /// emphasis pair, a cue-text timing tag, five cue settings, and a cue whose
    /// hours are left out.
    const VTT_SAMPLE: &str = concat!(
        "WEBVTT - Adventures in timed text\n",
        "Region: overlay\n",
        "Style: highlight\n",
        "\n",
        "NOTE Transcribed by the tokenizer project.\n",
        "\n",
        "STYLE\n",
        "::cue(.highlight) { color: #4afa9b }\n",
        "::cue { background: transparent }\n",
        "\n",
        "REGION\n",
        "id:overlay\n",
        "width:100.0%\n",
        "lines:3\n",
        "viewportanchor:10.0%,90.0%\n",
        "scroll:up\n",
        "\n",
        "1\n",
        "00:00:20.000 --> 00:00:24.000 align:start position:50%\n",
        "Hello <v Roger>are you there?\n",
        "\n",
        "00:00:25.000 --> 00:00:30.000\n",
        "An <i>italic</i> word.\n",
        "\n",
        "2\n",
        "00:00:31.000 --> 00:00:34.000 size:60% line:-1 region:overlay\n",
        "Highlight: <c.highlight>class</c> and time.\n",
        "Later at <00:00:33.500> the plot thickens.\n",
        "\n",
        "01:00.000 --> 01:02.000 vertical:rl\n",
        "A cue that leaves its hours out.\n",
    );

    /// Empty input, a BOM alone and with content, LF versus CRLF, non-ASCII and
    /// emoji cue text, malformed cues, and blocks that never terminated.
    const CORPUS: [&str; 18] = [
        "",
        "\u{FEFF}",
        SRT_SAMPLE,
        VTT_SAMPLE,
        "1\n00:00:01,000 --> 00:00:02,000\r\nHi\r\n",
        "1\n00:00:01,000 --> 00:00:02,000\nHi\n",
        "00:00:01,000 --> 00:00:02,000",
        "\r",
        "\n\n\n",
        "-->",
        "--> -->\n",
        "1\n00:00:01,00 --> \n",
        "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<i>unclosed\n",
        "WEBVTT junk\n",
        "1\n00:00:01,000 -> 00:00:02,000\n",
        "STYLE",
        "1999: a line that opens like a timestamp\n",
        "\u{FEFF}NOTE\r\nmulti\r\nline\r\n",
    ];

    /// Inputs a hostile file can hold: each one must still lex, diagnose and
    /// finish.
    const PATHOLOGICAL: [&str; 14] = [
        "-",
        ">",
        "<",
        "<i",
        "--",
        "00:",
        "0:,0",
        ",,,\n",
        ":::\n",
        "1\n-->\n",
        "WEBVTT\n\n-->\n",
        "00:00:00,000 -->00:00:01,000\n",
        "00:00:00,000--> 00:00:01,000\n",
        "\u{FEFF}\r\n\r\n\u{FEFF}\n",
    ];

    /// Kinds that belong to every language's vocabulary, so they prove nothing
    /// about how a format models its own content.
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

    /// Every document the suite measures, both fixtures and every broken one.
    fn corpus() -> impl Iterator<Item = &'static str> {
        CORPUS.iter().chain(PATHOLOGICAL.iter()).copied()
    }

    fn pairs() -> Vec<(&'static str, &'static dyn HostLanguage, &'static str)> {
        vec![
            ("srt", &ENGINE, SRT_SAMPLE),
            ("vtt", &VTT_ENGINE, VTT_SAMPLE),
        ]
    }

    fn kinds(engine: &dyn HostLanguage, source: &str, semantic: bool) -> Vec<String> {
        let opts = HostAnalysisOptions::default();
        let tokenization = if semantic {
            engine.semantic_tokens(source, &opts)
        } else {
            engine.lex(source, &opts)
        }
        .unwrap();
        tokenization
            .tokens
            .iter()
            .map(|token| token.kind.to_string())
            .collect()
    }

    fn codes(engine: &dyn HostLanguage, source: &str) -> Vec<String> {
        let found = engine
            .diagnose(source, &HostAnalysisOptions::default())
            .unwrap()
            .iter()
            .map(|diagnostic| diagnostic.code.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            found,
            validate(source, dialect_of(engine))
                .iter()
                .map(|diagnostic| diagnostic.code.to_string())
                .collect::<Vec<_>>(),
            "`diagnose` and `validate` must agree for {source:?}"
        );
        found
    }

    /// Which dialect an engine reads with: the two engines differ only here.
    fn dialect_of(engine: &dyn HostLanguage) -> Options {
        if std::ptr::eq(engine.descriptor(), &DESCRIPTOR) {
            Options::SRT
        } else {
            Options::VTT
        }
    }

    /// The sorted set of kinds a fixture produced that are not the vocabulary
    /// every editor already has.
    fn non_generic(found: &[String]) -> Vec<String> {
        let mut set: Vec<String> = found
            .iter()
            .filter(|kind| !GENERIC_KINDS.contains(&kind.as_str()))
            .cloned()
            .collect();
        set.sort();
        set.dedup();
        set
    }

    fn assert_lossless_stream(source: &str, options: Options) {
        let lexed = lex(source, options);
        assert_eq!(
            lexed.verify_lossless(),
            Ok(()),
            "the token stream must cover {source:?}"
        );
        assert_eq!(lexed.joined(), source, "token text must be the source");
        for token in lexed.tokens() {
            assert!(token.span.end > token.span.start, "zero-length span");
            assert!(source.is_char_boundary(token.span.start));
            assert!(source.is_char_boundary(token.span.end));
        }
        assert!(
            lexed
                .significant_tokens()
                .all(|token| !token.kind.is_trivia()),
            "the significant filter must be exact"
        );
    }

    // ─── Measured vocabularies ───────────────────────────────────────────────

    #[test]
    fn srt_lex_emits_exactly_the_subrip_kinds() {
        assert_eq!(
            non_generic(&kinds(&ENGINE, SRT_SAMPLE, false)),
            [
                "bom",
                "cue-identifier",
                "cue-index",
                "cue-text",
                "millisecond",
                "millisecond-separator",
                "record-break",
                "time-hour",
                "time-minute",
                "time-second",
                "time-separator",
                "timing-arrow",
            ]
        );
    }

    #[test]
    fn vtt_lex_emits_exactly_the_webvtt_kinds() {
        assert_eq!(
            non_generic(&kinds(&VTT_ENGINE, VTT_SAMPLE, false)),
            [
                "block-marker",
                "cue-id",
                "cue-text",
                "header-name",
                "header-separator",
                "header-value",
                "markup-name",
                "markup-punctuation",
                "markup-value",
                "millisecond",
                "millisecond-separator",
                "record-break",
                "region-property",
                "region-separator",
                "region-value",
                "setting-name",
                "setting-separator",
                "setting-value",
                "signature",
                "signature-comment",
                "style-content",
                "time-hour",
                "time-minute",
                "time-second",
                "time-separator",
                "timing-arrow",
            ]
        );
    }

    #[test]
    fn srt_semantic_emits_exactly_the_subrip_roles() {
        assert_eq!(
            non_generic(&kinds(&ENGINE, SRT_SAMPLE, true)),
            [
                "bom",
                "cue-end",
                "cue-identifier",
                "cue-index",
                "cue-start",
                "cue-text",
                "cue-text-continuation",
                "record-break",
                "timing-arrow",
            ]
        );
    }

    #[test]
    fn vtt_semantic_emits_exactly_the_webvtt_roles() {
        assert_eq!(
            non_generic(&kinds(&VTT_ENGINE, VTT_SAMPLE, true)),
            [
                "alignment-value",
                "block-marker",
                "class-tag",
                "closing-tag",
                "cue-end",
                "cue-id",
                "cue-start",
                "cue-text",
                "cue-text-continuation",
                "emphasis-tag",
                "header-name",
                "header-separator",
                "line-value",
                "markup-punctuation",
                "markup-value",
                "position-value",
                "record-break",
                "region-property",
                "region-reference",
                "region-separator",
                "region-value",
                "setting-name",
                "setting-separator",
                "signature",
                "signature-note",
                "size-value",
                "style-reference",
                "style-rule",
                "timing-arrow",
                "timing-tag",
                "vertical-value",
                "voice-tag",
            ]
        );
    }

    #[test]
    fn both_semantic_layers_clear_the_depth_bar() {
        for (name, engine, source) in pairs() {
            let found = non_generic(&kinds(engine, source, true));
            assert!(
                found.len() >= 8,
                "{name} semantic layer has only {}: {found:?}",
                found.len()
            );
        }
    }

    #[test]
    fn neither_layer_borrows_a_generic_vocabulary() {
        for (name, engine, source) in pairs() {
            for semantic in [false, true] {
                for kind in kinds(engine, source, semantic) {
                    assert!(
                        !matches!(
                            kind.as_str(),
                            "identifier"
                                | "number"
                                | "string"
                                | "punctuation"
                                | "keyword"
                                | "type"
                                | "variable"
                                | "function"
                                | "class"
                        ),
                        "{name} (semantic={semantic}) laundered the generic kind {kind}"
                    );
                }
            }
        }
    }

    // ─── Losslessness ────────────────────────────────────────────────────────

    #[test]
    fn every_corpus_entry_lexes_to_a_lossless_stream_for_both_ids() {
        for source in corpus() {
            for options in [Options::SRT, Options::VTT] {
                assert_lossless_stream(source, options);
            }
        }
    }

    #[test]
    fn both_host_layers_reconstruct_the_source() {
        for source in corpus() {
            for engine in [&ENGINE as &dyn HostLanguage, &VTT_ENGINE] {
                for semantic in [false, true] {
                    let tokenization = engine.lex(source, &HostAnalysisOptions::default()).unwrap();
                    let tokenization = if semantic {
                        engine
                            .semantic_tokens(source, &HostAnalysisOptions::default())
                            .unwrap()
                    } else {
                        tokenization
                    };
                    let mut joined = String::new();
                    let mut previous = 0usize;
                    for token in &tokenization.tokens {
                        assert!(
                            token.span.end > token.span.start && token.span.start >= previous,
                            "gap, overlap or zero-width span in {source:?} semantic={semantic}"
                        );
                        assert!(
                            source.is_char_boundary(token.span.start)
                                && source.is_char_boundary(token.span.end),
                            "span off a char boundary in {source:?}"
                        );
                        joined.push_str(&source[token.span.start..token.span.end]);
                        previous = token.span.end;
                    }
                    assert_eq!(joined, source, "{source:?} semantic={semantic}");
                }
            }
        }
    }

    #[test]
    fn the_semantic_layer_never_moves_a_span() {
        for source in corpus() {
            for engine in [&ENGINE as &dyn HostLanguage, &VTT_ENGINE] {
                let opts = HostAnalysisOptions::default();
                let syntax = engine.lex(source, &opts).unwrap();
                let semantic = engine.semantic_tokens(source, &opts).unwrap();
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
                    "semantic spans must match syntax spans for {source:?}"
                );
            }
        }
    }

    #[test]
    fn malformed_input_never_panics_or_stalls() {
        for source in PATHOLOGICAL {
            for options in [Options::SRT, Options::VTT] {
                let parsed = parse(source, options);
                assert!(
                    parsed.lexed().is_lossless(),
                    "{source:?} lost bytes under {options:?}"
                );
                let _ = parsed.is_valid();
                let _ = parsed.cues().len();
            }
        }
    }

    // ─── Identity retagging ─────────────────────────────────────────────────

    fn kinds_of(source: &str, options: Options) -> Vec<SyntaxKind> {
        lex(source, options)
            .tokens()
            .iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn a_digits_only_identity_line_is_an_index_only_in_subrip() {
        let source = "7\n00:00:01,000 --> 00:00:02,000\nHi\n";
        assert!(kinds_of(source, Options::SRT).contains(&SyntaxKind::CueIndex));
        let webvtt = "WEBVTT\n\n7\n00:00:01.000 --> 00:00:02.000\nHi\n";
        let vtt_kinds = kinds_of(webvtt, Options::VTT);
        assert!(vtt_kinds.contains(&SyntaxKind::CueId), "{vtt_kinds:?}");
        assert!(!vtt_kinds.contains(&SyntaxKind::CueIndex));
        assert!(!kinds_of(source, Options::SRT).contains(&SyntaxKind::CueId));
    }

    #[test]
    fn a_non_integer_identity_line_stays_a_cue_identifier() {
        let source = "opening-credits\n00:00:01,000 --> 00:00:02,000\nHi\n";
        let kinds = kinds_of(source, Options::SRT);
        assert!(kinds.contains(&SyntaxKind::CueIdentifier), "{kinds:?}");
        assert!(!kinds.contains(&SyntaxKind::CueIndex));
        assert!(codes(&ENGINE, source).is_empty());
    }

    #[test]
    fn only_the_line_a_timing_line_follows_is_an_identity() {
        let source = "1\nprose\n00:00:01,000 --> 00:00:02,000\n";
        let kinds = kinds_of(source, Options::SRT);
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::CueText)
                .count(),
            1,
            "the earlier candidate is text written too early: {kinds:?}"
        );
        assert!(kinds.contains(&SyntaxKind::CueIdentifier));
        assert_eq!(codes(&ENGINE, source), ["text-before-timing"]);
    }

    #[test]
    fn an_unterminated_block_keeps_its_identity_reading() {
        let source = "5\nno timing line here\n";
        let kinds = kinds_of(source, Options::SRT);
        assert!(kinds.contains(&SyntaxKind::CueIdentifier), "{kinds:?}");
        assert_eq!(codes(&ENGINE, source), ["missing-timing-line"]);
        assert_eq!(
            codes(&VTT_ENGINE, "5\nno timing line here\n"),
            ["missing-signature"],
            "the same bytes with no header to open them are a second fault"
        );
        assert_eq!(
            codes(&VTT_ENGINE, "WEBVTT\n\n5\nno timing line here\n"),
            ["missing-timing-line"]
        );
        assert!(
            kinds_of("WEBVTT\n\n5\nno timing line here\n", Options::VTT)
                .contains(&SyntaxKind::CueId)
        );
    }

    // ─── Diagnostics ────────────────────────────────────────────────────────

    #[test]
    fn missing_webvtt_signature_is_reported_only_for_vtt() {
        let source = "1\n00:00:01.000 --> 00:00:02.000\nHi\n";
        assert_eq!(codes(&VTT_ENGINE, source), ["missing-signature"]);
        assert!(codes(&ENGINE, "1\n00:00:01,000 --> 00:00:02,000\nHi\n").is_empty());
        // SubRip has no signature to miss.
        assert!(!codes(&ENGINE, source).contains(&"missing-signature".to_string()));
    }

    #[test]
    fn header_text_after_the_signature_must_be_a_note() {
        assert_eq!(
            codes(&VTT_ENGINE, "WEBVTT junk\n"),
            ["unexpected-header-text"]
        );
        assert!(codes(&VTT_ENGINE, "WEBVTT - a fine note\n").is_empty());
        assert!(codes(&VTT_ENGINE, "WEBVTT\n").is_empty());
    }

    #[test]
    fn a_comma_is_a_timestamp_only_in_subrip() {
        let dot = "00:00:01.000 --> 00:00:02.000\n";
        let comma = "00:00:01,000 --> 00:00:02,000\n";
        assert_eq!(
            codes(&VTT_ENGINE, &format!("WEBVTT\n\n{dot}")),
            Vec::<String>::new()
        );
        assert_eq!(
            codes(&ENGINE, comma),
            Vec::<String>::new(),
            "SubRip requires hours, a comma and three digits"
        );
        assert_eq!(
            codes(&ENGINE, dot),
            ["malformed-timestamp", "malformed-timestamp"]
        );
        assert_eq!(
            codes(&VTT_ENGINE, &format!("WEBVTT\n\n{comma}")),
            ["malformed-timestamp", "malformed-timestamp"]
        );
    }

    #[test]
    fn subrip_requires_its_fractional_digits() {
        assert_eq!(
            codes(&ENGINE, "00:00:01,00 --> 00:00:02,000\n"),
            ["malformed-timestamp"]
        );
        // WebVTT may leave the milliseconds out, and shorten them.
        assert!(codes(&VTT_ENGINE, "WEBVTT\n\n00:01 --> 00:02.5\n").is_empty());
        assert!(codes(&VTT_ENGINE, "WEBVTT\n\n01:00.000 --> 01:02.000\n").is_empty());
    }

    #[test]
    fn a_timing_line_needs_its_arrow() {
        assert_eq!(
            codes(&ENGINE, "00:00:01,000 -> 00:00:02,000\n"),
            ["missing-arrow"]
        );
        assert_eq!(
            codes(&ENGINE, "00:00:01,000 00:00:02,000\n"),
            ["missing-arrow"]
        );
    }

    #[test]
    fn an_arrow_needs_exactly_two_timestamps() {
        assert_eq!(codes(&ENGINE, "-->"), ["missing-timestamp"]);
        assert_eq!(codes(&ENGINE, "00:00:01,000 -->\n"), ["missing-timestamp"]);
        assert_eq!(
            codes(&ENGINE, "00:00:01,000 --> 00:00:02,000 --> 00:00:03,000\n"),
            ["missing-timestamp"]
        );
    }

    #[test]
    fn a_cue_without_a_timing_line_is_named() {
        assert_eq!(codes(&ENGINE, "1\nJust prose.\n"), ["missing-timing-line"]);
        assert_eq!(
            codes(&VTT_ENGINE, "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn text_written_before_the_timing_line_is_a_fault() {
        assert_eq!(
            codes(&ENGINE, "1\nprose\n00:00:01,000 --> 00:00:02,000\nHello\n"),
            ["text-before-timing"]
        );
    }

    #[test]
    fn a_cue_must_end_after_it_starts() {
        assert_eq!(
            codes(&ENGINE, "00:00:04,000 --> 00:00:01,000\n"),
            ["start-after-end"]
        );
        // A framing cue, start equal to end, is legal.
        assert!(codes(&ENGINE, "00:00:04,000 --> 00:00:04,000\n").is_empty());
    }

    #[test]
    fn overlapping_cues_warn_without_losing_either() {
        let source = concat!(
            "1\n00:00:01,000 --> 00:00:05,000\nFirst\n",
            "\n2\n00:00:04,000 --> 00:00:06,000\nSecond\n"
        );
        assert_eq!(codes(&ENGINE, source), ["overlapping-cues"]);
        let parsed = parse(source, Options::SRT);
        assert_eq!(parsed.cues().len(), 2);
        assert!(parsed.is_valid(), "an overlap is a warning");
    }

    #[test]
    fn two_cues_must_be_separated_by_an_empty_line() {
        let source = concat!(
            "1\n00:00:01,000 --> 00:00:02,000\nFirst\n",
            "2\n00:00:03,000 --> 00:00:04,000\nSecond\n"
        );
        assert_eq!(codes(&ENGINE, source), ["missing-blank-line"]);
        assert_eq!(parse(source, Options::SRT).cues().len(), 2);
    }

    #[test]
    fn subrip_indices_must_rise() {
        let source = concat!(
            "1\n00:00:01,000 --> 00:00:02,000\nFirst\n",
            "\n3\n00:00:03,000 --> 00:00:04,000\nSecond\n",
            "\n2\n00:00:05,000 --> 00:00:06,000\nThird\n"
        );
        assert_eq!(codes(&ENGINE, source), ["non-monotonic-index"]);
        assert!(
            codes(
                &ENGINE,
                "1\n00:00:01,000 --> 00:00:02,000\n\n4\n00:00:03,000 --> 00:00:04,000\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn an_unknown_webvtt_block_type_warns() {
        assert_eq!(
            codes(&VTT_ENGINE, "WEBVTT\n\nBOGUS\nsome text\n"),
            ["unknown-block-type"]
        );
        // SubRip has no block types, so the same bytes are an unfinished cue.
        assert_eq!(
            codes(&ENGINE, "BOGUS\nsome text\n"),
            ["missing-timing-line"]
        );
        // A real marker never warns.
        assert!(codes(&VTT_ENGINE, "WEBVTT\n\nNOTE a comment\ntwo lines\n").is_empty());
    }

    #[test]
    fn an_unknown_cue_setting_warns_only_in_webvtt() {
        assert_eq!(
            codes(
                &VTT_ENGINE,
                "WEBVTT\n\n00:00:01.000 --> 00:00:02.000 bogus:1\n"
            ),
            ["unknown-cue-setting"]
        );
        for known in [
            "align:start",
            "position:50%",
            "size:60%",
            "line:-1",
            "vertical:rl",
        ] {
            assert!(
                codes(
                    &VTT_ENGINE,
                    &format!("WEBVTT\n\n00:00:01.000 --> 00:00:02.000 {known}\n")
                )
                .is_empty(),
                "{known} is a documented setting"
            );
        }
    }

    #[test]
    fn subrip_timing_lines_carry_no_settings() {
        assert_eq!(
            codes(&ENGINE, "00:00:01,000 --> 00:00:02,000 align:start\n"),
            ["unexpected-timing-text"]
        );
        assert_eq!(
            codes(&ENGINE, "00:00:01,000 --> 00:00:02,000 wobble tail\n"),
            ["unexpected-timing-text"]
        );
    }

    #[test]
    fn an_unclosed_tag_is_a_webvtt_fault() {
        assert_eq!(
            codes(
                &VTT_ENGINE,
                "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHello <i\n"
            ),
            ["unterminated-markup"]
        );
        // SubRip has no tags, so the same bytes are prose.
        assert!(codes(&ENGINE, "00:00:01,000 --> 00:00:02,000\nHello <i\n").is_empty());
    }

    #[test]
    fn a_lone_arrow_and_a_truncated_timestamp_keep_reporting_one_fault_each() {
        assert_eq!(codes(&ENGINE, "-->\n"), ["missing-timestamp"]);
        // Two timing lines in one block: each arrow is named, and so is the
        // empty line that never separates them.
        assert_eq!(
            codes(&ENGINE, "-->\n--> \n"),
            [
                "missing-timestamp",
                "missing-blank-line",
                "missing-timestamp"
            ]
        );
        // A clock cut off in the middle still opens where a timing line opens,
        // so its one fault is the timestamp itself.
        assert_eq!(codes(&ENGINE, "00:0\n"), ["malformed-timestamp"]);
        // A block that never has a timing line at all is the other fault.
        assert_eq!(codes(&ENGINE, "1\nprose\n"), ["missing-timing-line"]);
    }

    #[test]
    fn every_diagnostic_code_round_trips_and_has_a_severity() {
        let expected = [
            ("malformed-timestamp", Severity::Error),
            ("missing-arrow", Severity::Error),
            ("missing-blank-line", Severity::Warning),
            ("missing-signature", Severity::Error),
            ("missing-timing-line", Severity::Error),
            ("missing-timestamp", Severity::Error),
            ("non-monotonic-index", Severity::Warning),
            ("overlapping-cues", Severity::Warning),
            ("start-after-end", Severity::Error),
            ("text-before-timing", Severity::Error),
            ("unexpected-header-text", Severity::Error),
            ("unexpected-timing-text", Severity::Warning),
            ("unknown-block-type", Severity::Warning),
            ("unknown-cue-setting", Severity::Warning),
            ("unterminated-markup", Severity::Error),
        ];
        assert_eq!(expected.len(), DiagnosticKind::ALL.len());
        for (code, severity) in expected {
            let kind =
                DiagnosticKind::from_code(code).unwrap_or_else(|| panic!("no kind for {code}"));
            assert_eq!(kind.code(), code);
            assert_eq!(kind.severity(), severity, "{code}");
            assert!(!code.contains('_'), "{code} must stay kebab-case");
            assert!(!kind.message().is_empty());
            let diagnostic = Diagnostic {
                span: Span::new(0, 1),
                code: kind.code(),
                message: kind.message(),
            };
            assert_eq!(diagnostic.code, code);
            assert_eq!(diagnostic.message, kind.message());
        }
        assert_eq!(
            DiagnosticKind::ALL
                .iter()
                .map(|kind| kind.code())
                .collect::<Vec<_>>()
                .as_slice(),
            expected
                .iter()
                .map(|(code, _)| *code)
                .collect::<Vec<_>>()
                .as_slice(),
            "ALL must be ordered by code"
        );
    }

    #[test]
    fn dialect_only_codes_are_declared_as_such() {
        for kind in DiagnosticKind::ALL {
            match kind {
                DiagnosticKind::NonMonotonicIndex | DiagnosticKind::UnexpectedTimingText => {
                    assert!(kind.only_srt() && !kind.only_vtt(), "{kind:?}");
                }
                DiagnosticKind::MissingSignature
                | DiagnosticKind::UnexpectedHeaderText
                | DiagnosticKind::UnknownBlockType
                | DiagnosticKind::UnknownCueSetting
                | DiagnosticKind::UnterminatedMarkup => {
                    assert!(kind.only_vtt() && !kind.only_srt(), "{kind:?}");
                }
                other => assert!(!other.only_srt() && !other.only_vtt(), "{other:?}"),
            }
        }
        // And the declaration holds in practice: a SubRip-only code can never
        // appear for WebVTT, nor the other way round.
        for source in CORPUS {
            let srt: Vec<String> = codes(&ENGINE, source)
                .iter()
                .filter_map(|code| DiagnosticKind::from_code(code))
                .filter(|kind| kind.only_vtt())
                .map(|kind| kind.code().to_owned())
                .collect();
            assert!(srt.is_empty(), "SubRip reported vtt-only codes {srt:?}");
            let vtt: Vec<String> = codes(&VTT_ENGINE, source)
                .iter()
                .filter_map(|code| DiagnosticKind::from_code(code))
                .filter(|kind| kind.only_srt())
                .map(|kind| kind.code().to_owned())
                .collect();
            assert!(vtt.is_empty(), "WebVTT reported srt-only codes {vtt:?}");
        }
    }

    // ─── The fixtures themselves ─────────────────────────────────────────────

    #[test]
    fn both_representative_fixtures_are_clean() {
        for (name, engine, source) in pairs() {
            assert_eq!(codes(engine, source), Vec::<String>::new(), "{name}");
            let parsed = parse(source, dialect_of(engine));
            assert!(parsed.is_valid(), "{name} must validate");
            assert!(!parsed.lexed().has_errors(), "{name} has flagged spans");
            for token in parsed.lexed().tokens() {
                assert!(
                    !token.has_error(),
                    "{name}'s fixture should need no error flags, got {token:?}"
                );
            }
        }
    }

    #[test]
    fn the_subrip_fixture_parses_to_three_cues() {
        let parsed = parse(SRT_SAMPLE, Options::SRT);
        let cues = parsed.cues();
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].identity, Some(Span::new(3, 4)));
        assert_eq!(parsed.text(cues[0].identity.unwrap()), "1");
        assert_eq!(cues[0].start.map(|stamp| stamp.millis), Some(1_000));
        assert_eq!(cues[0].end.map(|stamp| stamp.millis), Some(4_000));
        assert_eq!(cues[0].text.len(), 2, "the first cue has two text lines");
        assert_eq!(cues[1].start.map(|stamp| stamp.millis), Some(4_500));
        assert_eq!(cues[2].identity.map(|span| parsed.text(span)), Some("coda"));
        assert_eq!(cues[2].end.map(|stamp| stamp.millis), Some(12_000));
        assert!(cues.iter().all(|cue| cue.settings.is_empty()));
        assert_eq!(parsed.headers().len(), 0);
    }

    #[test]
    fn the_webvtt_fixture_parses_to_four_cues_and_two_header_fields() {
        let parsed = parse(VTT_SAMPLE, Options::VTT);
        assert_eq!(parsed.headers().len(), 2);
        assert_eq!(parsed.text(parsed.headers()[0].name), "Region");
        assert_eq!(
            parsed.headers()[0]
                .value
                .map(|span| parsed.text(span).trim()),
            Some("overlay")
        );
        let cues = parsed.cues();
        assert_eq!(cues.len(), 4);
        assert_eq!(cues[0].identity.map(|span| parsed.text(span)), Some("1"));
        assert_eq!(cues[0].settings.len(), 2);
        assert_eq!(cues[0].start.map(|stamp| stamp.millis), Some(20_000));
        assert_eq!(cues[1].identity, None, "this cue has no id line");
        assert_eq!(cues[2].settings.len(), 3);
        assert_eq!(cues[2].text.len(), 2);
        assert_eq!(
            cues[3].start.map(|stamp| stamp.millis),
            Some(60_000),
            "a cue with its hours left out still reads its minutes"
        );
        assert!(
            cues.iter()
                .flat_map(|cue| cue.settings.iter())
                .all(|setting| setting.known)
        );
    }

    // ─── Dialect disagreement ───────────────────────────────────────────────

    #[test]
    fn the_dialects_disagree_in_both_directions() {
        // SubRip bytes under WebVTT: the signature is missing and every
        // comma-separated timestamp is malformed.
        let srt_as_vtt = codes(&VTT_ENGINE, SRT_SAMPLE);
        assert!(srt_as_vtt.contains(&"missing-signature".to_string()));
        assert!(srt_as_vtt.contains(&"malformed-timestamp".to_string()));
        // WebVTT bytes under SubRip: the header and the block markers are
        // unfinished cues, and the dot-separated timestamps are malformed.
        let vtt_as_srt = codes(&ENGINE, VTT_SAMPLE);
        assert!(vtt_as_srt.contains(&"missing-timing-line".to_string()));
        assert!(vtt_as_srt.contains(&"malformed-timestamp".to_string()));
        assert!(vtt_as_srt.contains(&"unexpected-timing-text".to_string()));
        // Neither reads the other's identity line as its own.
        assert_eq!(
            kinds(&ENGINE, "1\n00:00:01,000 --> 00:00:02,000\n", false)
                .iter()
                .filter(|kind| **kind == "cue-index" || **kind == "cue-id")
                .collect::<Vec<_>>(),
            vec!["cue-index"]
        );
        assert_eq!(
            kinds(
                &VTT_ENGINE,
                "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n",
                false
            )
            .iter()
            .filter(|kind| **kind == "cue-index" || **kind == "cue-id")
            .collect::<Vec<_>>(),
            vec!["cue-id"]
        );
    }

    #[test]
    fn each_id_emits_only_kinds_its_own_format_has() {
        for (name, engine, forbidden) in [
            (
                "srt",
                &ENGINE as &dyn HostLanguage,
                &[
                    "cue-id",
                    "signature",
                    "signature-comment",
                    "header-name",
                    "block-marker",
                    "style-content",
                    "region-property",
                    "region-separator",
                    "markup-name",
                ][..],
            ),
            ("vtt", &VTT_ENGINE, &["cue-index"][..]),
        ] {
            for source in corpus() {
                let found = kinds(engine, source, false);
                for kind in forbidden {
                    assert!(
                        !found.iter().any(|found| found == kind),
                        "{name} emitted its dialect's forbidden kind {kind} for {source:?}"
                    );
                }
            }
        }
    }

    // ─── Descriptors and the host surface ───────────────────────────────────

    #[test]
    fn the_two_descriptors_carry_their_own_ids_and_the_real_surface() {
        assert_eq!(DESCRIPTOR.language, LanguageId::SRT);
        assert_eq!(VTT_DESCRIPTOR.language, LanguageId::VTT);
        assert_eq!(DESCRIPTOR.display_name, "SubRip");
        assert_eq!(VTT_DESCRIPTOR.display_name, "WebVTT");
        assert_eq!(DESCRIPTOR.aliases, ["srt", "subrip"]);
        assert_eq!(VTT_DESCRIPTOR.aliases, ["vtt", "webvtt"]);
        assert_eq!(DESCRIPTOR.extensions, [".srt"]);
        assert_eq!(VTT_DESCRIPTOR.extensions, [".vtt"]);
        assert_eq!(DESCRIPTOR.mime_types, ["application/x-subrip"]);
        assert_eq!(VTT_DESCRIPTOR.mime_types, ["text/vtt"]);
        for descriptor in [&DESCRIPTOR, &VTT_DESCRIPTOR] {
            assert_eq!(descriptor.capabilities, CAPABILITIES);
            assert_eq!(descriptor.engine_version, env!("CARGO_PKG_VERSION"));
            assert!(descriptor.capabilities.contains(
                Capabilities::LEX
                    | Capabilities::PARSE
                    | Capabilities::SEMANTIC
                    | Capabilities::VALIDATE
            ));
            assert!(
                !descriptor
                    .capabilities
                    .contains(Capabilities::CST | Capabilities::NAVIGATE | Capabilities::VISITOR)
            );
            assert_eq!(
                descriptor.capabilities.close_prerequisites(),
                descriptor.capabilities,
                "the advertised surface must be closable"
            );
        }
        for engine in [&ENGINE as &dyn HostLanguage, &VTT_ENGINE] {
            assert!(engine.require(Capabilities::LEX).is_ok());
            assert!(engine.require(Capabilities::CST).is_err());
            assert!(engine.resolve_dialect("default").is_ok());
            assert!(engine.resolve_dialect("not-a-dialect").is_err());
        }
        assert_eq!(ENGINE.id(), LanguageId::SRT);
        assert_eq!(VTT_ENGINE.id(), LanguageId::VTT);
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default().with_limits(
            themoretheless_tokenizer_core::InputLimits::conservative().max_input_bytes(3),
        );
        for engine in [&ENGINE as &dyn HostLanguage, &VTT_ENGINE] {
            assert!(matches!(
                engine.lex("WEBVTT\n", &opts),
                Err(HostError::InputTooLarge { .. })
            ));
            assert!(engine.semantic_tokens("WEBVTT\n", &opts).is_err());
            assert!(engine.diagnose("WEBVTT\n", &opts).is_err());
        }
    }

    #[test]
    fn tokenize_layer_reaches_both_layers() {
        let opts = HostAnalysisOptions::default();
        for (engine, source) in [
            (&ENGINE as &dyn HostLanguage, SRT_SAMPLE),
            (&VTT_ENGINE, VTT_SAMPLE),
        ] {
            let syntax = engine
                .tokenize_layer(
                    source,
                    &opts,
                    themoretheless_tokenizer_core::TokenLayer::Syntax,
                )
                .unwrap();
            let semantic = engine
                .tokenize_layer(
                    source,
                    &opts,
                    themoretheless_tokenizer_core::TokenLayer::Semantic,
                )
                .unwrap();
            assert_eq!(syntax.tokens.len(), semantic.tokens.len());
            assert!(syntax.valid && semantic.valid);
        }
    }

    #[test]
    fn validate_and_parse_agree_with_the_host() {
        for source in corpus() {
            for options in [Options::SRT, Options::VTT] {
                let parsed = parse(source, options);
                let engine: &dyn HostLanguage = if options == Options::SRT {
                    &ENGINE
                } else {
                    &VTT_ENGINE
                };
                let valid = engine
                    .lex(source, &HostAnalysisOptions::default())
                    .unwrap()
                    .valid;
                assert_eq!(valid, parsed.is_valid(), "{source:?} {options:?}");
                assert_eq!(
                    validate(source, options).len(),
                    parsed.diagnostics().len(),
                    "{source:?} {options:?}"
                );
                let errors = validate(source, options)
                    .iter()
                    .filter(|diagnostic| {
                        DiagnosticKind::from_code(diagnostic.code)
                            .is_some_and(|kind| kind.severity() == Severity::Error)
                    })
                    .count();
                assert_eq!(errors == 0, parsed.is_valid(), "{source:?} {options:?}");
            }
        }
    }

    #[test]
    fn diagnostics_never_point_off_a_char_boundary() {
        for source in corpus() {
            for options in [Options::SRT, Options::VTT] {
                for diagnostic in validate(source, options) {
                    assert!(
                        source.is_char_boundary(diagnostic.span.start)
                            && source.is_char_boundary(diagnostic.span.end),
                        "{:?} for {source:?}",
                        diagnostic.code
                    );
                    assert!(diagnostic.span.end > diagnostic.span.start, "empty span");
                }
            }
        }
    }

    #[test]
    fn the_lexers_option_axes_are_independent_of_each_other() {
        // `Dialect` names the two parameter sets, and each option is one axis a
        // third timed-text dialect could pick on its own: moving one axis must
        // not drag another with it.
        assert_eq!(Dialect::Srt.options(), Options::SRT);
        assert_eq!(Dialect::Vtt.options(), Options::VTT);
        assert_eq!(Options::default(), Options::SRT);
        assert_ne!(Options::SRT, Options::VTT);

        let codes_of = |source: &str, options: Options| -> Vec<&'static str> {
            parse(source, options)
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect()
        };
        let timing = "00:01,000 --> 00:02,000\n";
        assert_eq!(
            codes_of(timing, Options::SRT),
            ["malformed-timestamp", "malformed-timestamp"]
        );
        let only_hours = Options {
            hours_optional: true,
            ..Options::SRT
        };
        assert!(
            codes_of(timing, only_hours).is_empty(),
            "hours are their own axis: the comma stays SubRip's"
        );
        let dotted = "00:00:01.000 --> 00:00:02.000\n";
        assert_eq!(
            codes_of(dotted, Options::SRT),
            ["malformed-timestamp", "malformed-timestamp"]
        );
        let only_dot = Options {
            millisecond_separator: b'.',
            ..Options::SRT
        };
        assert!(
            codes_of(dotted, only_dot).is_empty(),
            "the separator is its own axis: hours stay required"
        );
    }

    #[test]
    fn every_kind_names_itself_on_the_wire() {
        for kind in [
            SyntaxKind::Bom,
            SyntaxKind::Whitespace,
            SyntaxKind::RecordBreak,
            SyntaxKind::Error,
            SyntaxKind::Signature,
            SyntaxKind::SignatureComment,
            SyntaxKind::HeaderName,
            SyntaxKind::HeaderSeparator,
            SyntaxKind::HeaderValue,
            SyntaxKind::BlockMarker,
            SyntaxKind::Comment,
            SyntaxKind::StyleContent,
            SyntaxKind::RegionProperty,
            SyntaxKind::RegionValue,
            SyntaxKind::CueIndex,
            SyntaxKind::CueIdentifier,
            SyntaxKind::CueId,
            SyntaxKind::CueText,
            SyntaxKind::TimeHour,
            SyntaxKind::TimeSeparator,
            SyntaxKind::TimeMinute,
            SyntaxKind::TimeSecond,
            SyntaxKind::MillisecondSeparator,
            SyntaxKind::Millisecond,
            SyntaxKind::TimingArrow,
            SyntaxKind::SettingName,
            SyntaxKind::SettingSeparator,
            SyntaxKind::SettingValue,
            SyntaxKind::MarkupPunctuation,
            SyntaxKind::MarkupName,
            SyntaxKind::MarkupValue,
        ] {
            assert!(!kind.host_kind().is_empty());
            assert_eq!(
                kind.is_trivia(),
                matches!(
                    kind,
                    SyntaxKind::Bom | SyntaxKind::Whitespace | SyntaxKind::RecordBreak
                )
            );
        }
        for kind in [
            SyntaxKind::Signature,
            SyntaxKind::CueId,
            SyntaxKind::MarkupName,
        ] {
            assert!(kind.is_vtt_only() && !kind.is_srt_only());
        }
        assert!(SyntaxKind::CueIndex.is_srt_only());
        assert!(!SyntaxKind::CueText.is_srt_only());
        assert!(!SyntaxKind::CueText.is_vtt_only());
    }
}
