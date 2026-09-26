//! A lossless, dialect-parameterised SubRip/WebVTT lexer.
//!
//! Concatenating token text reconstructs the source byte-for-byte, including
//! for malformed input: a truncated timestamp, a `->` where `-->` belongs, an
//! unclosed `<i` and a dash run the grammar has no room for each keep their own
//! span — flagged [`LexToken::has_error`] — and are never dropped or replaced by
//! a synthesized token. Nothing here borrows a programming-language vocabulary:
//! the milliseconds are `millisecond`, the `-->` is a `timing-arrow`, a line of
//! subtitle prose is `cue-text`, and a newline is a `record-break`.
//!
//! SubRip and WebVTT share this lexer and disagree through [`Options`] alone,
//! on rules that change what the bytes mean:
//!
//! * WebVTT opens with a `WEBVTT` signature (SubRip has none), accepts the
//!   `NOTE`/`STYLE`/`REGION` blocks (SubRip reads those words as cue text),
//!   lets a timestamp leave its hours out (`mm:ss.SSS`), separates milliseconds
//!   with `.` and may leave them out entirely, allows `name:value` cue settings
//!   on a timing line, and marks cue text up with `<v Voice>`, `<c.class>`,
//!   `<i>` and `<00:00:24.000>` timing tags.
//! * SubRip requires `hh:mm:ss,mmm` with a `,` and exactly three fractional
//!   digits, has no settings after its end timestamp, has no blocks and no
//!   markup, and reads a digits-only identity line as the cue's ordinal index —
//!   where WebVTT, which has no index concept, keeps it an ordinary `cue-id`.
//!
//! Whether a line before a timing line is the cue's identity or text written
//! too early depends on the *next* line, so an identity candidate is lexed as
//! [`SyntaxKind::CueIdentifier`] and retagged by [`Lexer::resolve_identity`]
//! once its timing line — or the end of its block — has been seen.

use themoretheless_tokenizer_core::{LosslessViolation, Span, verify_lossless_spans};

/// Which timed-text dialect a document is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dialect {
    /// SubRip: ordinal index lines, `hh:mm:ss,mmm --> hh:mm:ss,mmm`, prose.
    Srt,
    /// WebVTT: `WEBVTT` header, `NOTE`/`STYLE`/`REGION` blocks, cue settings,
    /// inline tags, `mm:ss[.SSS]` timestamps.
    Vtt,
}

impl Dialect {
    /// The parameter set this dialect is defined by.
    #[must_use]
    pub const fn options(self) -> Options {
        match self {
            Self::Srt => Options::SRT,
            Self::Vtt => Options::VTT,
        }
    }
}

/// Engine options: every axis on which the two dialects part company, exposed
/// individually so a third timed-text dialect can pick its own combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Options {
    /// The document opens with a `WEBVTT` signature, and the first block's
    /// `Name: value` lines are programmatic headers.
    pub programmatic_header: bool,
    /// A `NOTE` line at the start of a block opens a comment block.
    pub comment_blocks: bool,
    /// A `STYLE` line at the start of a block opens a stylesheet block.
    pub style_blocks: bool,
    /// A `REGION` line at the start of a block opens a region block.
    pub region_blocks: bool,
    /// A timestamp may leave its hours out (`mm:ss[.SSS]`).
    pub hours_optional: bool,
    /// The byte between seconds and fractional seconds: `,` for SubRip.
    pub millisecond_separator: u8,
    /// A timestamp must state its fractional seconds.
    pub milliseconds_required: bool,
    /// A timing line may carry `name:value` cue settings after its end
    /// timestamp.
    pub cue_settings: bool,
    /// A cue text line may carry voice, class, style and timing tags.
    pub inline_markup: bool,
    /// A digits-only cue identity is an ordinal index, not an opaque id.
    pub ordinal_index: bool,
}

impl Options {
    /// SubRip: no header, no blocks, `hh:mm:ss,mmm` only, no settings, no
    /// markup, and a digits-only identity line is the cue index.
    pub const SRT: Self = Self {
        programmatic_header: false,
        comment_blocks: false,
        style_blocks: false,
        region_blocks: false,
        hours_optional: false,
        millisecond_separator: b',',
        milliseconds_required: true,
        cue_settings: false,
        inline_markup: false,
        ordinal_index: true,
    };

    /// WebVTT: the `WEBVTT` signature, the three block markers, optional hours,
    /// `.` before the milliseconds, cue settings and inline tags, and a
    /// digits-only identity line that stays an ordinary cue id.
    pub const VTT: Self = Self {
        programmatic_header: true,
        comment_blocks: true,
        style_blocks: true,
        region_blocks: true,
        hours_optional: true,
        millisecond_separator: b'.',
        milliseconds_required: false,
        cue_settings: true,
        inline_markup: true,
        ordinal_index: false,
    };
}

impl Default for Options {
    fn default() -> Self {
        Self::SRT
    }
}

/// Exact lexical categories emitted by the SubRip/WebVTT lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// A run of spaces and tabs that carries no content.
    Whitespace,
    /// The `\r\n`, `\n` or `\r` ending a line.
    RecordBreak,
    /// A span the lexer flagged but kept: a broken timestamp, an unterminated
    /// tag, a dash run that should have been `-->`.
    Error,
    /// The `WEBVTT` token that opens a WebVTT file.
    Signature,
    /// The `- description` run a WebVTT signature line may carry.
    SignatureComment,
    /// A programmatic header name (`Region`, `Style`) before its `:`.
    HeaderName,
    /// The `:` after a programmatic header name.
    HeaderSeparator,
    /// The rest of a header line: a header value or the media file identifier.
    HeaderValue,
    /// A `NOTE`, `STYLE` or `REGION` line opener.
    BlockMarker,
    /// A comment: the text after a `NOTE` marker and the lines below it.
    Comment,
    /// A line inside a `STYLE` block.
    StyleContent,
    /// A `name` before the `:` of a `REGION` block line.
    RegionProperty,
    /// The `:` that separates a `REGION` property from its value.
    RegionSeparator,
    /// The rest of a `REGION` block line after its `name:`.
    RegionValue,
    /// A digits-only cue identity line: the SubRip ordinal index.
    CueIndex,
    /// A cue identity line that is not a plain integer.
    CueIdentifier,
    /// A cue identity line: WebVTT's opaque cue id.
    CueId,
    /// A line of cue text.
    CueText,
    /// The hours field of a timestamp.
    TimeHour,
    /// A `:` between timestamp fields.
    TimeSeparator,
    /// The minutes field of a timestamp.
    TimeMinute,
    /// The seconds field of a timestamp.
    TimeSecond,
    /// The `,` or `.` before the fractional seconds.
    MillisecondSeparator,
    /// The fractional seconds of a timestamp.
    Millisecond,
    /// The `-->` between a cue's start and end timestamps.
    TimingArrow,
    /// A cue setting name, or other text after a timing line's end timestamp.
    SettingName,
    /// The `:` after a cue setting name.
    SettingSeparator,
    /// A cue setting value, up to whitespace or the line break.
    SettingValue,
    /// A `<` or `>` of an inline WebVTT tag.
    MarkupPunctuation,
    /// A tag name inside inline markup: `v`, `/c`, `i`.
    MarkupName,
    /// The voice or class an inline tag carries: ` Roger`, `.warning`.
    MarkupValue,
}

impl SyntaxKind {
    /// The host wire name for this kind before any semantic reading.
    #[must_use]
    pub const fn host_kind(self) -> &'static str {
        match self {
            Self::Bom => "bom",
            Self::Whitespace => "whitespace",
            Self::RecordBreak => "record-break",
            Self::Error => "error",
            Self::Signature => "signature",
            Self::SignatureComment => "signature-comment",
            Self::HeaderName => "header-name",
            Self::HeaderSeparator => "header-separator",
            Self::HeaderValue => "header-value",
            Self::BlockMarker => "block-marker",
            Self::Comment => "comment",
            Self::StyleContent => "style-content",
            Self::RegionProperty => "region-property",
            Self::RegionSeparator => "region-separator",
            Self::RegionValue => "region-value",
            Self::CueIndex => "cue-index",
            Self::CueIdentifier => "cue-identifier",
            Self::CueId => "cue-id",
            Self::CueText => "cue-text",
            Self::TimeHour => "time-hour",
            Self::TimeSeparator => "time-separator",
            Self::TimeMinute => "time-minute",
            Self::TimeSecond => "time-second",
            Self::MillisecondSeparator => "millisecond-separator",
            Self::Millisecond => "millisecond",
            Self::TimingArrow => "timing-arrow",
            Self::SettingName => "setting-name",
            Self::SettingSeparator => "setting-separator",
            Self::SettingValue => "setting-value",
            Self::MarkupPunctuation => "markup-punctuation",
            Self::MarkupName => "markup-name",
            Self::MarkupValue => "markup-value",
        }
    }

    /// Whitespace, line breaks and the BOM carry no cue structure.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Bom | Self::Whitespace | Self::RecordBreak)
    }

    /// Kinds holding the text of a cue, a comment or a style declaration.
    #[must_use]
    pub const fn is_line_text(self) -> bool {
        matches!(
            self,
            Self::CueText
                | Self::Comment
                | Self::StyleContent
                | Self::RegionValue
                | Self::RegionProperty
                | Self::HeaderValue
                | Self::SignatureComment
        )
    }

    /// Kinds a cue's identity line may take.
    #[must_use]
    pub const fn is_identity(self) -> bool {
        matches!(self, Self::CueIndex | Self::CueIdentifier | Self::CueId)
    }

    /// Whether this kind can only exist in a WebVTT document.
    #[must_use]
    pub const fn is_vtt_only(self) -> bool {
        matches!(
            self,
            Self::Signature
                | Self::SignatureComment
                | Self::HeaderName
                | Self::HeaderSeparator
                | Self::HeaderValue
                | Self::BlockMarker
                | Self::StyleContent
                | Self::RegionProperty
                | Self::RegionSeparator
                | Self::RegionValue
                | Self::CueId
                | Self::MarkupPunctuation
                | Self::MarkupName
                | Self::MarkupValue
        )
    }

    /// Whether this kind can only exist in a SubRip document.
    #[must_use]
    pub const fn is_srt_only(self) -> bool {
        matches!(self, Self::CueIndex)
    }
}

/// Compact per-token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    pub const EMPTY: Self = Self(0);
    /// The span is part of, or abandoned by, a malformed construct.
    pub const HAS_ERROR: Self = Self(1);
    /// The span is one field of a timestamp. A reader walks the fields this bit
    /// links to recover the whole timestamp it belongs to.
    pub const TIMESTAMP: Self = Self(2);
    /// The first field of its timestamp.
    pub const TIMESTAMP_HEAD: Self = Self(4);
    /// The timestamp this field belongs to is not the shape the dialect
    /// documents.
    pub const BAD_TIME: Self = Self(8);
    /// A dash run sits where a timing arrow belongs and is not `-->`.
    pub const BAD_ARROW: Self = Self(16);
    /// An inline tag was opened and never reached its `>` on this line.
    pub const UNCLOSED: Self = Self(32);
    /// A cue setting name WebVTT does not document.
    pub const UNKNOWN_SETTING: Self = Self(64);
    /// The span sits inside a `<…>` tag of a WebVTT cue text line, so a
    /// reader knows a timestamp here is a cue-text timing tag, not a cue's
    /// own timing line.
    pub const IN_MARKUP: Self = Self(128);

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for TokenFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for TokenFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A lexical token. Spans are non-empty UTF-8 byte ranges, and every source
/// byte sits in exactly one of them, which [`Lexed::verify_lossless`] checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexToken {
    pub kind: SyntaxKind,
    pub span: Span,
    pub flags: TokenFlags,
}

impl LexToken {
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        self.span.slice(source)
    }

    /// Whether this token belongs to a malformed construct.
    #[must_use]
    pub const fn has_error(self) -> bool {
        self.flags.contains(TokenFlags::HAS_ERROR)
    }

    /// Whether this span is one field of a timestamp.
    #[must_use]
    pub const fn in_timestamp(self) -> bool {
        self.flags.contains(TokenFlags::TIMESTAMP)
    }

    /// Whether this span opens its timestamp.
    #[must_use]
    pub const fn is_timestamp_head(self) -> bool {
        self.flags.contains(TokenFlags::TIMESTAMP_HEAD)
    }

    /// Whether the timestamp this field belongs to is malformed.
    #[must_use]
    pub const fn has_bad_time(self) -> bool {
        self.flags.contains(TokenFlags::BAD_TIME)
    }

    /// Whether this dash run should have been a `-->`.
    #[must_use]
    pub const fn is_bad_arrow(self) -> bool {
        self.flags.contains(TokenFlags::BAD_ARROW)
    }

    /// Whether this token opens markup that never closed.
    #[must_use]
    pub const fn is_unclosed(self) -> bool {
        self.flags.contains(TokenFlags::UNCLOSED)
    }

    /// Whether this token is a setting name WebVTT does not document.
    #[must_use]
    pub const fn is_unknown_setting(self) -> bool {
        self.flags.contains(TokenFlags::UNKNOWN_SETTING)
    }

    /// Whether this token sits inside a `<…>` tag of a cue text line.
    #[must_use]
    pub const fn in_markup(self) -> bool {
        self.flags.contains(TokenFlags::IN_MARKUP)
    }

    /// Whether this token is a field of a cue's own timing line: a timestamp
    /// field that is not one of a cue text line's timing tags.
    #[must_use]
    pub const fn is_timing_field(self) -> bool {
        self.in_timestamp() && !self.in_markup()
    }
}

/// Lossless lexer output. Concatenating token text always reconstructs the
/// original source byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed<'source> {
    source: &'source str,
    tokens: Vec<LexToken>,
}

impl<'source> Lexed<'source> {
    #[must_use]
    pub const fn source(&self) -> &'source str {
        self.source
    }

    #[must_use]
    pub fn tokens(&self) -> &[LexToken] {
        &self.tokens
    }

    /// Tokens that are not whitespace, a line break or the BOM.
    pub fn significant_tokens(&self) -> impl Iterator<Item = LexToken> + '_ {
        self.tokens
            .iter()
            .copied()
            .filter(|token| !token.kind.is_trivia())
    }

    #[must_use]
    pub fn text(&self, token: LexToken) -> Option<&'source str> {
        token.text(self.source)
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.tokens.iter().any(|token| token.has_error())
    }

    /// Token text concatenated in span order.
    #[must_use]
    pub fn joined(&self) -> String {
        let mut joined = String::with_capacity(self.source.len());
        for token in &self.tokens {
            if let Some(text) = token.text(self.source) {
                joined.push_str(text);
            }
        }
        joined
    }

    /// Whether the token stream covers the source with no gaps or overlaps.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.verify_lossless().is_ok()
    }

    /// Named lossless violation. Streams from [`lex`] always pass; the check is
    /// public so a host can re-verify a transformed token list.
    pub fn verify_lossless(&self) -> Result<(), LosslessViolation> {
        verify_lossless_spans(self.source, self.tokens.iter().map(|token| token.span))
    }
}

/// Lex a SubRip or WebVTT document.
#[must_use]
pub fn lex(source: &str, options: Options) -> Lexed<'_> {
    let mut lexer = Lexer {
        bytes: source.as_bytes(),
        options,
        pos: 0,
        tokens: Vec::new(),
        context: Context::Body,
        saw_signature: false,
        timing_seen: false,
        pending: Vec::new(),
    };
    lexer.run();
    Lexed {
        source,
        tokens: lexer.tokens,
    }
}

/// Which kind of block the line being lexed belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// Between blocks: the start of the document, or just after a blank line.
    Body,
    /// The WebVTT file header, from its signature to the next blank line.
    Header,
    /// Inside a `NOTE` block.
    Comment,
    /// Inside a `STYLE` block.
    Style,
    /// Inside a `REGION` block.
    Region,
    /// A cue: its identity line, its timing line, or its text.
    Cue,
}

/// The role one physical line takes inside its block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Signature,
    Header,
    Marker,
    Comment,
    Style,
    Region,
    Timing,
    Identity,
    Text,
}

/// One physical line, its break kept separate from its content.
struct Line {
    start: usize,
    content_end: usize,
    break_start: usize,
    break_end: usize,
}

/// A cue identity line whose role is not settled yet: the index of the token
/// holding its text, and the bytes that text covers.
struct Pending {
    token: usize,
    text: Span,
}

/// UTF-8 encoding of U+FEFF.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// The WebVTT program token.
const SIGNATURE: &[u8] = b"WEBVTT";
/// The arrow between a cue's timestamps.
const ARROW: &[u8] = b"-->";
/// `STYLE` and `REGION` are longer than `NOTE`, so they are named apart.
const STYLE_MARKER: &[u8] = b"STYLE";
const REGION_MARKER: &[u8] = b"REGION";
/// The block markers WebVTT documents.
const MARKER_WORDS: [&[u8]; 3] = [b"NOTE", STYLE_MARKER, REGION_MARKER];

const fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

const fn is_break(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

const fn is_digit(byte: u8) -> bool {
    byte.is_ascii_digit()
}

/// Bytes a timestamp is written with. Every one is ASCII, so a UTF-8
/// continuation byte can never split a token in the middle of a character.
const fn is_time_byte(byte: u8) -> bool {
    is_digit(byte) || byte == b':' || byte == b',' || byte == b'.'
}

/// Whether `run` is written as a timestamp: only timestamp bytes, a colon
/// inside, and a digit at each end.
#[must_use]
fn is_timestamp_word(run: &[u8]) -> bool {
    run.first().is_some_and(|byte| is_digit(*byte))
        && run.last().is_some_and(|byte| is_digit(*byte))
        && run.contains(&b':')
        && run.iter().all(|byte| is_time_byte(*byte))
}

/// A non-empty run of digits.
#[must_use]
fn is_digit_run(text: &[u8]) -> bool {
    !text.is_empty() && text.iter().all(u8::is_ascii_digit)
}

/// A WebVTT tag name: an optional `/`, then letters, digits, `-`, `_` and `.`,
/// with at least one letter so a timing tag's digits are not read as one.
#[must_use]
fn is_tag_name(text: &[u8]) -> bool {
    let body = text.strip_prefix(b"/").unwrap_or(text);
    !body.is_empty()
        && body
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && body.iter().any(u8::is_ascii_alphabetic)
}

/// The cue settings WebVTT documents.
#[must_use]
fn is_known_setting(name: &[u8]) -> bool {
    matches!(
        name,
        b"vertical" | b"line" | b"position" | b"size" | b"align" | b"region"
    )
}

/// The line-at-a-time scanner. Each line is read in the block role the
/// [`Self::context`] gives it, and every byte the line holds is covered by a
/// token before the next line starts.
struct Lexer<'source> {
    /// The document, as bytes: every marker this grammar has is ASCII.
    bytes: &'source [u8],
    /// The dialect rules the document is read with.
    options: Options,
    /// The next byte to read. Every path through a line moves it forward.
    pos: usize,
    /// Tokens settled so far, in span order.
    tokens: Vec<LexToken>,
    /// Which kind of block the line being read belongs to.
    context: Context,
    /// Whether the WebVTT signature line has been read.
    saw_signature: bool,
    /// Whether the current block has had its timing line yet.
    timing_seen: bool,
    /// Identity candidates whose role the next line still has to settle.
    pending: Vec<Pending>,
}

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_bom();
        while let Some(line) = self.read_line() {
            if self.is_blank(&line) {
                self.close_block();
                self.lex_blank_line(&line);
            } else {
                let role = self.role_of(&line);
                self.lex_line(&line, role);
            }
        }
        self.close_block();
    }

    fn lex_bom(&mut self) {
        if self.bytes.starts_with(&BOM) {
            self.push(SyntaxKind::Bom, 0, BOM.len(), TokenFlags::EMPTY);
            self.pos = BOM.len();
        }
    }

    /// The next physical line, advancing `pos` past its break. `None` at EOF.
    fn read_line(&mut self) -> Option<Line> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && !is_break(self.bytes[end]) {
            end += 1;
        }
        let break_start = end;
        let mut break_end = end;
        if break_end < self.bytes.len() {
            break_end = self.break_end(break_end);
        }
        self.pos = break_end;
        Some(Line {
            start,
            content_end: end,
            break_start,
            break_end,
        })
    }

    /// End of the line break starting at `start`: `\r\n` is one break.
    fn break_end(&self, start: usize) -> usize {
        let mut end = start + 1;
        if self.bytes[start] == b'\r' && self.bytes.get(end) == Some(&b'\n') {
            end += 1;
        }
        end
    }

    /// A line holding spaces at most.
    fn is_blank(&self, line: &Line) -> bool {
        let mut index = line.start;
        while index < line.content_end {
            if !is_space(self.bytes[index]) {
                return false;
            }
            index += 1;
        }
        true
    }

    /// The blank line, or end of input, that closes a block.
    fn close_block(&mut self) {
        if self.context == Context::Cue && !self.timing_seen {
            self.resolve_identity(false);
        }
        self.context = Context::Body;
        self.timing_seen = false;
    }

    /// The role this line takes, and the block state it leaves behind.
    fn role_of(&mut self, line: &Line) -> Role {
        let content = &self.bytes[line.start..line.content_end];
        if self.context == Context::Body {
            if self.options.programmatic_header && !self.saw_signature {
                self.context = Context::Header;
                return Role::Signature;
            }
            if let Some(context) = self.marker_context(content) {
                self.context = context;
                return Role::Marker;
            }
            self.context = Context::Cue;
        }
        match self.context {
            Context::Comment => Role::Comment,
            Context::Style => Role::Style,
            Context::Region => Role::Region,
            Context::Header => {
                // A timing line with no blank line after the header: the header
                // ends here and the cue is lexed as the cue it is.
                if self.timing_prefix(content).is_some() {
                    self.context = Context::Cue;
                    self.timing_seen = false;
                    Role::Timing
                } else {
                    Role::Header
                }
            }
            Context::Cue => {
                if self.timing_prefix(content).is_some() {
                    Role::Timing
                } else if self.timing_seen {
                    Role::Text
                } else {
                    Role::Identity
                }
            }
            Context::Body => unreachable!("the body context always opens a block"),
        }
    }

    /// The block a marker line opens, if this line starts with one.
    fn marker_context(&self, content: &[u8]) -> Option<Context> {
        let text = trim_start_spaces(content);
        if self.options.comment_blocks && starts_with_word(text, b"NOTE") {
            return Some(Context::Comment);
        }
        if self.options.style_blocks && starts_with_word(text, STYLE_MARKER) {
            return Some(Context::Style);
        }
        if self.options.region_blocks && starts_with_word(text, REGION_MARKER) {
            return Some(Context::Region);
        }
        None
    }

    /// Whether this line opens where a timing line must: an optional indent,
    /// then either a `-` — the arrow a broken line still states — or a run of
    /// timestamp bytes that holds a `:`. A cue's index line is digits with no
    /// colon, so it stays the identity line it is, while cue text opening with
    /// `1999: a year` is malformed and reading it as a timing line names the
    /// fault exactly.
    fn timing_prefix(&self, content: &[u8]) -> Option<usize> {
        let index = trim_start_spaces_len(content);
        let byte = *content.get(index)?;
        if byte == b'-' {
            return Some(index);
        }
        if !is_digit(byte) {
            return None;
        }
        let run_end = content[index..]
            .iter()
            .position(|byte| !is_time_byte(*byte))
            .unwrap_or(content.len() - index);
        content[index..index + run_end]
            .contains(&b':')
            .then_some(index)
    }

    /// Give the pending identity candidates their final kind. With a timing
    /// line, the last candidate is the cue's identity and any earlier one was
    /// text written before its timing line. Without one, every line keeps the
    /// identity kind a reader of the broken file sees.
    fn resolve_identity(&mut self, timing_found: bool) {
        let pending = std::mem::take(&mut self.pending);
        let count = pending.len();
        for (position, entry) in pending.iter().enumerate() {
            let digits = is_digit_run(&self.bytes[entry.text.range()]);
            let wanted = if !timing_found {
                if self.options.programmatic_header {
                    SyntaxKind::CueId
                } else {
                    SyntaxKind::CueIdentifier
                }
            } else if position + 1 < count {
                SyntaxKind::CueText
            } else if digits && self.options.ordinal_index {
                SyntaxKind::CueIndex
            } else if self.options.programmatic_header {
                SyntaxKind::CueId
            } else {
                SyntaxKind::CueIdentifier
            };
            if let Some(token) = self.tokens.get_mut(entry.token) {
                token.kind = wanted;
            }
        }
        self.timing_seen = true;
    }

    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize, flags: TokenFlags) {
        debug_assert!(start < end, "a token span is never empty");
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags,
        });
    }

    /// A blank line: its optional spaces and its break.
    fn lex_blank_line(&mut self, line: &Line) {
        if line.content_end > line.start {
            self.push(
                SyntaxKind::Whitespace,
                line.start,
                line.content_end,
                TokenFlags::EMPTY,
            );
        }
        self.push_break(line);
    }

    fn push_break(&mut self, line: &Line) {
        if line.break_end > line.break_start {
            self.push(
                SyntaxKind::RecordBreak,
                line.break_start,
                line.break_end,
                TokenFlags::EMPTY,
            );
        }
    }

    /// Lex one non-blank line in the role [`role`] gives its content. A line's
    /// tokens go to a buffer first, because an identity candidate may still be
    /// retagged, and are appended afterwards — which is what leaves its index
    /// known for exactly that.
    fn lex_line(&mut self, line: &Line, role: Role) {
        let mut out: Vec<LexToken> = Vec::new();
        let mut identity: Option<(usize, Span)> = None;
        match role {
            Role::Signature => self.lex_signature_line(line, &mut out),
            Role::Header => self.lex_header_line(line, &mut out),
            Role::Marker => self.lex_marker_line(line, &mut out),
            Role::Comment => self.push_line_text(&mut out, SyntaxKind::Comment, line),
            Role::Style => self.push_line_text(&mut out, SyntaxKind::StyleContent, line),
            Role::Region => self.lex_region_line(line, &mut out),
            Role::Timing => self.lex_timing_line(line, &mut out),
            Role::Identity => {
                let end = line.content_end;
                let start = skip_spaces(self.bytes, line.start, end);
                push_flags(
                    &mut out,
                    SyntaxKind::Whitespace,
                    line.start,
                    start,
                    TokenFlags::EMPTY,
                );
                identity = Some((out.len(), Span::new(start, end)));
                push_flags(
                    &mut out,
                    SyntaxKind::CueIdentifier,
                    start,
                    end,
                    TokenFlags::EMPTY,
                );
            }
            Role::Text => self.lex_text_line(line, &mut out),
        }
        let base = self.tokens.len();
        if let Some((offset, text)) = identity {
            self.pending.push(Pending {
                token: base + offset,
                text,
            });
        }
        debug_assert!(!out.is_empty(), "a non-blank line always yields a token");
        self.tokens.append(&mut out);
        self.push_break(line);
    }

    /// A whole content line as one token of the given kind.
    fn push_line_text(&mut self, out: &mut Vec<LexToken>, kind: SyntaxKind, line: &Line) {
        push_flags(out, kind, line.start, line.content_end, TokenFlags::EMPTY);
    }

    /// The `WEBVTT` signature line: the program token, the optional
    /// `- description` run, and anything else the line holds.
    fn lex_signature_line(&mut self, line: &Line, out: &mut Vec<LexToken>) {
        self.saw_signature = true;
        let end = line.content_end;
        let pos = self.spaces(line.start, end, out);
        let name_end = pos + SIGNATURE.len();
        let signed = name_end <= end
            && &self.bytes[pos..name_end] == SIGNATURE
            && self.at_word_boundary(name_end);
        if !signed {
            // No signature at all: the line keeps the header kind it would have
            // had, and the structural pass reports the missing signature on it.
            push_flags(
                out,
                SyntaxKind::CueIdentifier,
                line.start,
                end,
                TokenFlags::EMPTY,
            );
            return;
        }
        push_flags(out, SyntaxKind::Signature, pos, name_end, TokenFlags::EMPTY);
        let rest = self.spaces(name_end, end, out);
        if rest >= end {
            return;
        }
        if self.bytes[rest] == b'-' {
            push_flags(
                out,
                SyntaxKind::SignatureComment,
                rest,
                end,
                TokenFlags::EMPTY,
            );
        } else {
            push_flags(out, SyntaxKind::Error, rest, end, TokenFlags::HAS_ERROR);
        }
    }

    /// Whether the byte at `pos` cannot continue the word just read. Every
    /// marker and tag name is ASCII, so a UTF-8 lead byte is a boundary.
    fn at_word_boundary(&self, pos: usize) -> bool {
        self.bytes
            .get(pos)
            .is_none_or(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_'))
    }

    /// A `Name: value` programmatic header, or the media file identifier.
    fn lex_header_line(&mut self, line: &Line, out: &mut Vec<LexToken>) {
        let end = line.content_end;
        let content = &self.bytes[line.start..end];
        let body_start = self.spaces(line.start, end, out);
        let named = colon_in(content, body_start - line.start);
        if let Some(at) = named {
            let name_end = line.start + at;
            push_flags(
                out,
                SyntaxKind::HeaderName,
                body_start,
                name_end,
                TokenFlags::EMPTY,
            );
            push_flags(
                out,
                SyntaxKind::HeaderSeparator,
                name_end,
                name_end + 1,
                TokenFlags::EMPTY,
            );
            push_flags(
                out,
                SyntaxKind::HeaderValue,
                name_end + 1,
                end,
                TokenFlags::EMPTY,
            );
            return;
        }
        push_flags(
            out,
            SyntaxKind::HeaderValue,
            body_start,
            end,
            TokenFlags::EMPTY,
        );
    }

    /// A `NOTE`, `STYLE` or `REGION` opening line, with whatever body shares it.
    fn lex_marker_line(&mut self, line: &Line, out: &mut Vec<LexToken>) {
        let end = line.content_end;
        let content = &self.bytes[line.start..end];
        let body_start = self.spaces(line.start, end, out);
        let indent = body_start - line.start;
        let word_len = marker_word_len(&content[indent..]).unwrap_or(end - body_start);
        let word_end = body_start + word_len;
        push_flags(
            out,
            SyntaxKind::BlockMarker,
            body_start,
            word_end,
            TokenFlags::EMPTY,
        );
        let value_start = self.spaces(word_end, end, out);
        if value_start >= end {
            return;
        }
        let kind = match self.context {
            Context::Style => SyntaxKind::StyleContent,
            Context::Region => SyntaxKind::RegionValue,
            _ => SyntaxKind::Comment,
        };
        push_flags(out, kind, value_start, end, TokenFlags::EMPTY);
    }

    /// A `REGION` block line: its property name, then everything after the `:`.
    fn lex_region_line(&mut self, line: &Line, out: &mut Vec<LexToken>) {
        let end = line.content_end;
        let content = &self.bytes[line.start..end];
        let body_start = self.spaces(line.start, end, out);
        let indent = body_start - line.start;
        if let Some(at) = colon_in(content, indent) {
            let name_end = line.start + at;
            push_flags(
                out,
                SyntaxKind::RegionProperty,
                body_start,
                name_end,
                TokenFlags::EMPTY,
            );
            push_flags(
                out,
                SyntaxKind::RegionSeparator,
                name_end,
                name_end + 1,
                TokenFlags::EMPTY,
            );
            push_flags(
                out,
                SyntaxKind::RegionValue,
                name_end + 1,
                end,
                TokenFlags::EMPTY,
            );
            return;
        }
        push_flags(
            out,
            SyntaxKind::RegionProperty,
            body_start,
            end,
            TokenFlags::EMPTY,
        );
    }

    /// A cue text line: plain prose in SubRip, and in WebVTT prose interleaved
    /// with voice, class, style and timing tags.
    fn lex_text_line(&mut self, line: &Line, out: &mut Vec<LexToken>) {
        let end = line.content_end;
        if !self.options.inline_markup {
            push_flags(out, SyntaxKind::CueText, line.start, end, TokenFlags::EMPTY);
            return;
        }
        let mut scratch: Vec<LexToken> = Vec::new();
        let mut pos = self.spaces(line.start, end, out);
        let mut run = pos;
        while pos < end {
            if self.bytes[pos] != b'<' {
                pos += 1;
                continue;
            }
            scratch.clear();
            if let Some(stop) = self.lex_markup(pos, end, &mut scratch) {
                push_flags(out, SyntaxKind::CueText, run, pos, TokenFlags::EMPTY);
                out.append(&mut scratch);
                pos = stop;
                run = stop;
            } else {
                // A `<` that opens no tag is prose; it stays in the run.
                pos += 1;
            }
        }
        push_flags(out, SyntaxKind::CueText, run, end, TokenFlags::EMPTY);
    }

    /// Lex the WebVTT tag at `pos`, which holds a `<`, into `out`, returning the
    /// offset just past it. `None` means the byte is text, which is what a bare
    /// `<` in prose is. A tag that never reaches its `>` flags the tokens it did
    /// produce, so the prose around it stays readable and every byte is covered.
    fn lex_markup(&self, pos: usize, end: usize, out: &mut Vec<LexToken>) -> Option<usize> {
        let body_start = pos + 1;
        if body_start >= end {
            return None;
        }
        let mut stop = body_start;
        while stop < end && !is_space(self.bytes[stop]) && self.bytes[stop] != b'>' {
            stop += 1;
        }
        let body = &self.bytes[body_start..stop];
        let timing = !body.is_empty()
            && body.iter().all(|byte| is_time_byte(*byte))
            && body.iter().any(|byte| is_digit(*byte));
        if !timing && !is_tag_name(body) {
            return None;
        }
        let base = out.len();
        push_flags(
            out,
            SyntaxKind::MarkupPunctuation,
            pos,
            body_start,
            TokenFlags::IN_MARKUP,
        );
        if timing {
            self.lex_timestamp(body_start, stop, out);
        } else {
            push_flags(
                out,
                SyntaxKind::MarkupName,
                body_start,
                stop,
                TokenFlags::IN_MARKUP,
            );
        }
        if stop < end && self.bytes[stop] != b'>' {
            // A voice name or a tag's attributes, up to the `>` closing it.
            let value_start = stop;
            while stop < end && self.bytes[stop] != b'>' {
                stop += 1;
            }
            push_flags(
                out,
                SyntaxKind::MarkupValue,
                value_start,
                stop,
                TokenFlags::IN_MARKUP,
            );
        }
        for token in &mut out[base..] {
            token.flags = token.flags.with(TokenFlags::IN_MARKUP);
        }
        if stop >= end {
            flag_unclosed(out, base);
            return Some(end);
        }
        push_flags(
            out,
            SyntaxKind::MarkupPunctuation,
            stop,
            stop + 1,
            TokenFlags::IN_MARKUP,
        );
        Some(stop + 1)
    }

    /// A cue's timing line, from its first timestamp to the end of the line.
    fn lex_timing_line(&mut self, line: &Line, out: &mut Vec<LexToken>) {
        self.resolve_identity(true);
        let end = line.content_end;
        let mut pos = self.spaces(line.start, end, out);
        pos = self.lex_timestamp(pos, end, out);
        pos = self.spaces(pos, end, out);
        pos = self.lex_arrow(pos, end, out);
        pos = self.spaces(pos, end, out);
        if pos < end {
            pos = self.lex_timestamp(pos, end, out);
            pos = self.spaces(pos, end, out);
        }
        if pos < end {
            self.lex_settings(pos, end, out);
        }
    }

    /// The space run at `pos`, pushed so no byte between two content tokens is
    /// ever left out of the stream.
    fn spaces(&self, pos: usize, end: usize, out: &mut Vec<LexToken>) -> usize {
        let stop = skip_spaces(self.bytes, pos, end);
        push_flags(out, SyntaxKind::Whitespace, pos, stop, TokenFlags::EMPTY);
        stop
    }

    /// One timestamp: every field of the digit-and-separator run at `pos`, or a
    /// single flagged span where a timestamp should have opened. A group is
    /// flagged as a whole, because a timestamp is one value written in fields
    /// and a reader wants its fault named once.
    fn lex_timestamp(&self, pos: usize, end: usize, out: &mut Vec<LexToken>) -> usize {
        if pos >= end {
            return pos;
        }
        let base = out.len();
        if self.bytes.get(pos..pos + ARROW.len()) == Some(ARROW) {
            return pos;
        }
        if !is_digit(self.bytes[pos]) {
            let stop = skip_word(self.bytes, pos, end);
            push_flags(
                out,
                SyntaxKind::Error,
                pos,
                stop,
                TokenFlags::HAS_ERROR
                    | TokenFlags::TIMESTAMP
                    | TokenFlags::TIMESTAMP_HEAD
                    | TokenFlags::BAD_TIME,
            );
            return stop;
        }
        let mut stop = pos;
        while stop < end && is_time_byte(self.bytes[stop]) {
            stop += 1;
        }
        let group = if timestamp_is_well_formed(&self.bytes[pos..stop], self.options) {
            TokenFlags::TIMESTAMP
        } else {
            TokenFlags::TIMESTAMP | TokenFlags::HAS_ERROR | TokenFlags::BAD_TIME
        };
        let fraction_at = self.bytes[pos..stop]
            .iter()
            .position(|byte| *byte == self.options.millisecond_separator);
        let clock_end = fraction_at.map_or(stop, |at| pos + at);
        let hours_present = self.bytes[pos..clock_end]
            .iter()
            .filter(|byte| **byte == b':')
            .count()
            >= 2;
        let mut index = pos;
        let mut colons = 0usize;
        let mut fraction = false;
        while index < stop {
            let field_start = index;
            while index < stop && is_digit(self.bytes[index]) {
                index += 1;
            }
            if index > field_start {
                let kind = if fraction {
                    SyntaxKind::Millisecond
                } else {
                    match colons {
                        0 if hours_present => SyntaxKind::TimeHour,
                        0 => SyntaxKind::TimeMinute,
                        1 if hours_present => SyntaxKind::TimeMinute,
                        _ => SyntaxKind::TimeSecond,
                    }
                };
                push_flags(out, kind, field_start, index, group);
            }
            if index >= stop {
                break;
            }
            let byte = self.bytes[index];
            let mut flags = group;
            let kind = if byte == b':' {
                colons += 1;
                SyntaxKind::TimeSeparator
            } else {
                fraction = true;
                if byte == self.options.millisecond_separator {
                    SyntaxKind::MillisecondSeparator
                } else {
                    // A separator this dialect does not use, kept and flagged.
                    flags = group | TokenFlags::BAD_TIME;
                    SyntaxKind::Error
                }
            };
            push_flags(out, kind, index, index + 1, flags);
            index += 1;
        }
        if let Some(head) = out.get_mut(base) {
            head.flags = head.flags.with(TokenFlags::TIMESTAMP_HEAD);
        }
        stop
    }

    /// The `-->` between a cue's timestamps, or the run standing where one
    /// belongs.
    fn lex_arrow(&self, pos: usize, end: usize, out: &mut Vec<LexToken>) -> usize {
        let broken = TokenFlags::HAS_ERROR | TokenFlags::BAD_ARROW;
        if self.bytes.get(pos..pos + ARROW.len()) == Some(ARROW) {
            push_flags(
                out,
                SyntaxKind::TimingArrow,
                pos,
                pos + ARROW.len(),
                TokenFlags::EMPTY,
            );
            return pos + ARROW.len();
        }
        if pos >= end {
            return pos;
        }
        let mut stop = pos;
        if self.bytes[pos] == b'-' {
            while stop < end && self.bytes[stop] == b'-' {
                stop += 1;
            }
            if self.bytes.get(stop) == Some(&b'>') {
                stop += 1;
            }
        } else {
            stop = skip_word(self.bytes, pos, end);
        }
        push_flags(out, SyntaxKind::Error, pos, stop, broken);
        stop
    }

    /// Everything after a timing line's end timestamp: WebVTT cue settings, and
    /// in SubRip the same bytes read as text the grammar has no room for.
    fn lex_settings(&self, pos: usize, end: usize, out: &mut Vec<LexToken>) {
        let mut index = pos;
        loop {
            if self.options.inline_markup && self.bytes[index] == b'<' {
                // A tag written in the settings column is still markup: it is
                // read as one so its bytes are named, and what follows it is
                // read as the word it is written as.
                if let Some(stop) = self.lex_markup(index, end, out) {
                    index = self.spaces(stop, end, out);
                    if index >= end {
                        return;
                    }
                    continue;
                }
            }
            if self.bytes[index] == b'-' {
                // A second arrow written on a line that already has one. It is
                // the arrow shape it is written in, not a setting name, so the
                // structural pass can count the groups on either side of it.
                index = self.lex_arrow(index, end, out);
                index = self.spaces(index, end, out);
                if index >= end {
                    return;
                }
                continue;
            }
            let word_start = index;
            let mut stop = index;
            let mut colon = None;
            while stop < end && !is_space(self.bytes[stop]) {
                if self.bytes[stop] == b':' && colon.is_none() {
                    colon = Some(stop);
                }
                stop += 1;
            }
            let name_end = colon.unwrap_or(stop);
            let mut flags = TokenFlags::EMPTY;
            if !is_known_setting(&self.bytes[word_start..name_end]) {
                flags = TokenFlags::UNKNOWN_SETTING;
            }
            if is_timestamp_word(&self.bytes[word_start..stop]) {
                // A whole timing group written where settings belong: lexed as
                // the timestamp it is written as, so the structural pass counts
                // it and names the line's fault once.
                self.lex_timestamp(word_start, stop, out);
            } else if let Some(at) = colon {
                push_flags(out, SyntaxKind::SettingName, word_start, at, flags);
                push_flags(out, SyntaxKind::SettingSeparator, at, at + 1, flags);
                push_flags(
                    out,
                    SyntaxKind::SettingValue,
                    at + 1,
                    stop,
                    TokenFlags::EMPTY,
                );
            } else {
                // A word after a timing line's end timestamp that is not a
                // `name:value` pair: kept, and unknown to the vocabulary.
                push_flags(out, SyntaxKind::SettingName, word_start, stop, flags);
            }
            index = self.spaces(stop, end, out);
            if index >= end {
                break;
            }
        }
    }
}

/// Flag the tokens of the tag being built, from `base`, as never closed.
fn flag_unclosed(out: &mut [LexToken], base: usize) {
    let flags = TokenFlags::HAS_ERROR | TokenFlags::UNCLOSED;
    for token in &mut out[base..] {
        token.flags = token.flags.with(flags);
    }
}

/// Whether `run` is a timestamp this dialect documents: two or three
/// colon-separated fields of up to two digits, then a fractional part after the
/// dialect's own separator.
#[must_use]
fn timestamp_is_well_formed(run: &[u8], options: Options) -> bool {
    let fraction_at = run
        .iter()
        .position(|byte| *byte == b'.' || *byte == b',' || *byte == options.millisecond_separator);
    let (clock, fraction) = match fraction_at {
        Some(at) => (&run[..at], Some((run[at], &run[at + 1..]))),
        None => (run, None),
    };
    let fields = clock.iter().filter(|byte| **byte == b':').count() + 1;
    let shape_ok = if options.hours_optional {
        (2..=3).contains(&fields)
    } else {
        fields == 3
    };
    if !shape_ok || !clock.iter().all(|byte| is_digit(*byte) || *byte == b':') {
        return false;
    }
    for field in clock.split(|byte| *byte == b':') {
        if field.len() > 2 || !is_digit_run(field) {
            return false;
        }
    }
    match fraction {
        Some((byte, digits)) => {
            byte == options.millisecond_separator
                && is_digit_run(digits)
                && digits.len() <= 3
                && (!options.milliseconds_required || digits.len() == 3)
        }
        None => !options.milliseconds_required,
    }
}

/// The offset of the first `:` in `content`, provided something follows it.
#[must_use]
fn colon_in(content: &[u8], indent: usize) -> Option<usize> {
    let at = content.iter().position(|byte| *byte == b':')?;
    (at > indent).then_some(at)
}

/// Append a token of the given kind and flags covering `start..end`, dropping a
/// range that holds no byte so a zero-length span can never reach a host.
fn push_flags(
    out: &mut Vec<LexToken>,
    kind: SyntaxKind,
    start: usize,
    end: usize,
    flags: TokenFlags,
) {
    debug_assert!(start <= end, "a token range is never inverted");
    if start < end {
        out.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags,
        });
    }
}

#[must_use]
fn skip_spaces(bytes: &[u8], mut pos: usize, end: usize) -> usize {
    while pos < end && is_space(bytes[pos]) {
        pos += 1;
    }
    pos
}

/// The end of the non-space run opening at `pos`, taking at least one byte.
#[must_use]
fn skip_word(bytes: &[u8], pos: usize, end: usize) -> usize {
    let mut stop = pos + 1;
    while stop < end && !is_space(bytes[stop]) {
        stop += 1;
    }
    stop
}

#[must_use]
fn trim_start_spaces_len(text: &[u8]) -> usize {
    text.iter()
        .position(|byte| !is_space(*byte))
        .unwrap_or(text.len())
}

#[must_use]
fn trim_start_spaces(text: &[u8]) -> &[u8] {
    &text[trim_start_spaces_len(text)..]
}

/// The length of the block marker opening `text`, if one is there.
#[must_use]
fn marker_word_len(text: &[u8]) -> Option<usize> {
    MARKER_WORDS
        .into_iter()
        .find(|word| starts_with_word(text, word))
        .map(<[u8]>::len)
}

/// Whether `text` opens with `word` and `word` is not merely the start of a
/// longer word: a cue id of `NOTICE` is not a `NOTE` marker.
#[must_use]
fn starts_with_word(text: &[u8], word: &[u8]) -> bool {
    text.len() >= word.len()
        && &text[..word.len()] == word
        && text
            .get(word.len())
            .is_none_or(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_'))
}
