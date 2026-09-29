//! Recovering EDN structure over the lossless token stream.
//!
//! The pass reads tokens only: every diagnostic points at bytes the lexer
//! already emitted, so recovery never consumes or drops input. Even the most
//! broken document keeps a lossless token stream while [`Parse::is_valid`]
//! reports `false`.
//!
//! Structure is deliberately shallow — a form table and a set of
//! parser_aware readings, not a node_identity tree: what an editor needs from
//! this format is *which* form a `#_` discards, *which* token carries a tag's
//! value, *which* map key repeats, and *which* keywords a `#:ns` prefix
//! namespaces. Those four facts are computed here and surfaced as [`Retag`]
//! marks the semantic layer applies without moving a single span.
//!
//! Top level: the EDN spec defines no enclosing element ("EDN is suitable
//! for streaming"), so a document is a *sequence* of top_level forms and
//! more than one is accepted without warning. An empty document is valid.
//!
//! Equality for duplicate detection follows the spec's typed equality:
//! symbols, keywords, strings, characters and literals compare within their
//! own kind; integers (decimal, `N`-suffixed and radix alike) canonicalize to
//! one value class; floats and `M`-decimals canonicalize through their
//! 64-bit value; ratios reduce. Compound keys (vectors, tagged forms,
//! discarded forms, error spans) are not compared.

use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::fmt;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Span};

use crate::lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};

/// An EDN structural violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A `#_` had no readable element after it before its collection or the
    /// end of input closed.
    DiscardWithoutValue,
    /// A map literal contained the same key twice.
    DuplicateKey,
    /// A set literal contained the same element twice.
    DuplicateSetElement,
    /// A `#` appeared with nothing it can dispatch on.
    IncompleteDispatch,
    /// A `\x` in a string is not one of the escapes EDN documents.
    InvalidEscape,
    /// A token started like a number but is none (`1e`, `01`, `1abc`, `##nan`).
    InvalidNumber,
    /// A radix prefix is not a decimal integer in `2..=36` (`1r11`).
    InvalidRadix,
    /// A radix digit is not valid for its declared radix (`2r1020`, `2r`).
    InvalidRadixDigit,
    /// A symbol, keyword or tag shape the spec disallows (`a//b`, `/a`, `:`).
    InvalidSymbol,
    /// `#inst`, `#uuid` or `#b` was followed by something other than a string.
    InvalidTaggedValue,
    /// A character literal is malformed (`\` at end, `\ `, `\nbsp`, `\u12`).
    MalformedChar,
    /// A closing delimiter does not match what is open, or closes nothing.
    MismatchedClose,
    /// A `#:ns` prefix was not followed by a map.
    NamespacedMapWithoutMap,
    /// A Clojure reader macro that is not data (`@`, `^`, `'`, `` ` ``, `#(`).
    NonEdnConstruct,
    /// A map literal held an odd number of forms, leaving a key without a value.
    OddMapEntries,
    /// A `#tag` had no element to tag.
    TagWithoutValue,
    /// A `(`, `[`, `{` or `#{` never reached its closing delimiter.
    UnterminatedCollection,
    /// A string's opening quote never reached its closing quote.
    UnterminatedString,
}

impl DiagnosticKind {
    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DiscardWithoutValue => "discard-without-value",
            Self::DuplicateKey => "duplicate-key",
            Self::DuplicateSetElement => "duplicate-set-element",
            Self::IncompleteDispatch => "incomplete-dispatch",
            Self::InvalidEscape => "invalid-escape",
            Self::InvalidNumber => "invalid-number",
            Self::InvalidRadix => "invalid-radix",
            Self::InvalidRadixDigit => "invalid-radix-digit",
            Self::InvalidSymbol => "invalid-symbol",
            Self::InvalidTaggedValue => "invalid-tagged-value",
            Self::MalformedChar => "malformed-char",
            Self::MismatchedClose => "mismatched-close",
            Self::NamespacedMapWithoutMap => "namespaced-map-without-map",
            Self::NonEdnConstruct => "non-edn-construct",
            Self::OddMapEntries => "odd-map-entries",
            Self::TagWithoutValue => "tag-without-value",
            Self::UnterminatedCollection => "unterminated-collection",
            Self::UnterminatedString => "unterminated-string",
        }
    }

    /// Human-readable one_liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::DiscardWithoutValue => "discard sequence has no following element",
            Self::DuplicateKey => "map key appears more than once",
            Self::DuplicateSetElement => "set element appears more than once",
            Self::IncompleteDispatch => "hash has nothing it can dispatch on",
            Self::InvalidEscape => "string escape is not one EDN documents",
            Self::InvalidNumber => "token looks numeric but is not an EDN number",
            Self::InvalidRadix => "radix prefix is not an integer between 2 and 36",
            Self::InvalidRadixDigit => "digit is invalid for the declared radix",
            Self::InvalidSymbol => "symbol or keyword shape is not legal EDN",
            Self::InvalidTaggedValue => "this built-in tag requires a following string",
            Self::MalformedChar => "character literal is malformed",
            Self::MismatchedClose => "closing delimiter does not match what is open",
            Self::NamespacedMapWithoutMap => "namespaced-map prefix is not followed by a map",
            Self::NonEdnConstruct => "Clojure reader construct is not part of the EDN data subset",
            Self::OddMapEntries => "map literal holds an odd number of forms",
            Self::TagWithoutValue => "tag has no following element",
            Self::UnterminatedCollection => "collection is never closed",
            Self::UnterminatedString => "string is never closed",
        }
    }
}

impl themoretheless_tokenizer_core::DiagnosticKind for DiagnosticKind {
    fn code(self) -> &'static str {
        DiagnosticKind::code(self)
    }

    fn message(self) -> &'static str {
        DiagnosticKind::message(self)
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// A parser_aware reading the semantic layer applies to a token without
/// moving its span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Retag {
    /// The token is part of a form a `#_` discards.
    Discarded,
    /// The first token of a `#inst` tag's string value.
    InstantValue,
    /// The first token of a `#uuid` tag's string value.
    UuidValue,
    /// The first token of a `#b` tag's string value.
    ByteValue,
    /// The first token of an arbitrary tag's value form.
    TaggedValue,
    /// A map key repeating an earlier key of the same map.
    DuplicateKey,
    /// A set element repeating an earlier element of the same set.
    DuplicateSetElement,
    /// A plain keyword that a `#:ns` prefix will namespace.
    NamespacedMapKey,
}

impl Retag {
    /// Wins-lowest ordering: when two readings land on one token, the mark
    /// with the smaller rank is the one kept.
    const fn rank(self) -> u8 {
        match self {
            Self::Discarded => 0,
            Self::DuplicateKey | Self::DuplicateSetElement => 1,
            Self::NamespacedMapKey => 2,
            Self::InstantValue | Self::UuidValue | Self::ByteValue | Self::TaggedValue => 3,
        }
    }
}

/// Lossless tokens plus EDN diagnostics, the semantic retag table and the
/// top_level form spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    retags: Vec<(usize, Retag)>,
    top_level: Vec<Span>,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    /// Lexical and structural diagnostics, in source order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// `(token index, reading)` pairs in lexed_token_index order.
    #[must_use]
    pub fn retags(&self) -> &[(usize, Retag)] {
        &self.retags
    }

    /// The parser_aware reading for one lexed token, if the structure pass
    /// has one.
    #[must_use]
    pub fn retag(&self, token_index: usize) -> Option<Retag> {
        self.retags
            .binary_search_by_key(&token_index, |(index, _)| *index)
            .ok()
            .map(|position| self.retags[position].1)
    }

    /// Spans of the top_level forms, in order. A document of several forms
    /// is an EDN stream, which the spec permits; see the module docs.
    #[must_use]
    pub fn top_level_forms(&self) -> &[Span] {
        &self.top_level
    }

    /// Whether the document is structurally clean. An error_flagged token is
    /// always paired with a diagnostic, so both must be empty.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && !self.lexed.has_errors()
    }

    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

/// Runs the lossless lexer and the recovering structure pass.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    let lexed = lex(source);
    let mut walker = Walker {
        source: lexed.source(),
        sigs: Vec::new(),
        stack: Vec::new(),
        diagnostics: Vec::new(),
        retags: HashMap::new(),
        top_level: Vec::new(),
    };
    walker.run(&lexed);
    let mut diagnostics: Vec<Diagnostic> = lexed
        .diagnostics()
        .iter()
        .copied()
        .chain(walker.diagnostics)
        .collect();
    diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
    let mut retags: Vec<(usize, Retag)> = walker.retags.into_iter().collect();
    retags.sort_unstable_by_key(|(index, _)| *index);
    Parse {
        lexed,
        diagnostics,
        retags,
        top_level: walker.top_level,
    }
}

/// Structural diagnostics only (lex and structure combined).
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).into_diagnostics()
}

struct Sig {
    real: usize,
    token: LexToken,
    /// The lexer emitted this token as part of the previous element
    /// (a string region's escape or tail).
    continues: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Seq,
    Map,
    Set,
}

#[derive(Debug, Clone, Copy)]
enum Dispatch {
    Tag { sig: usize, kind: SyntaxKind },
    Discard { sig: usize },
}

struct Frame {
    /// `None` for the implicit root level.
    open: Option<SyntaxKind>,
    open_sig: usize,
    mode: Mode,
    ns_prefixed: bool,
    dispatch: Vec<Dispatch>,
    /// In a map: the previous form was a key and still wants a value.
    expecting_value: bool,
    last_key_sig: usize,
    seen: HashSet<String>,
}

impl Frame {
    fn root() -> Self {
        Self {
            open: None,
            open_sig: 0,
            mode: Mode::Seq,
            ns_prefixed: false,
            dispatch: Vec::new(),
            expecting_value: false,
            last_key_sig: 0,
            seen: HashSet::new(),
        }
    }

    fn new(token: LexToken, open_sig: usize) -> Self {
        let mode = match token.kind {
            SyntaxKind::MapOpen => Mode::Map,
            SyntaxKind::SetOpen => Mode::Set,
            _ => Mode::Seq,
        };
        Self {
            open: Some(token.kind),
            open_sig,
            mode,
            ns_prefixed: false,
            dispatch: Vec::new(),
            expecting_value: false,
            last_key_sig: 0,
            seen: HashSet::new(),
        }
    }
}

struct Walker<'source> {
    source: &'source str,
    sigs: Vec<Sig>,
    stack: Vec<Frame>,
    diagnostics: Vec<Diagnostic>,
    retags: HashMap<usize, Retag>,
    top_level: Vec<Span>,
}

impl<'source> Walker<'source> {
    fn run(&mut self, lexed: &Lexed<'source>) {
        self.sigs = lexed
            .tokens()
            .iter()
            .enumerate()
            .filter(|(_, token)| !token.kind.is_trivia())
            .map(|(real, token)| Sig {
                real,
                token: *token,
                continues: token.flags.contains(TokenFlags::CONTINUES),
            })
            .collect();
        self.stack.push(Frame::root());
        let mut i = 0;
        let mut ns_attach = false;
        while i < self.sigs.len() {
            let token = self.sigs[i].token;
            if token.kind.is_open() {
                let mut frame = Frame::new(token, i);
                frame.ns_prefixed = ns_attach;
                ns_attach = false;
                self.stack.push(frame);
                i += 1;
                continue;
            }
            if token.kind.is_close() {
                self.close_frame(i, token);
                i += 1;
                continue;
            }
            match token.kind {
                SyntaxKind::Discard => {
                    if let Some(frame) = self.stack.last_mut() {
                        frame.dispatch.push(Dispatch::Discard { sig: i });
                    }
                }
                kind if kind.is_tag() => {
                    if let Some(frame) = self.stack.last_mut() {
                        frame.dispatch.push(Dispatch::Tag { sig: i, kind });
                    }
                }
                SyntaxKind::NamespacedMapPrefix => {
                    // `#:ns` namespaces the keys of the map that follows.
                    let followed_by_map = matches!(
                        self.sigs.get(i + 1).map(|sig| sig.token.kind),
                        Some(SyntaxKind::MapOpen)
                    );
                    if followed_by_map {
                        ns_attach = true;
                    } else {
                        self.push_diagnostic(DiagnosticKind::NamespacedMapWithoutMap, token.span);
                    }
                }
                _ => {
                    let mut end = i;
                    while self.sigs.get(end + 1).is_some_and(|sig| sig.continues) {
                        end += 1;
                    }
                    self.complete_form(i, end);
                    i = end;
                }
            }
            i += 1;
        }
        self.finish();
    }

    fn close_frame(&mut self, close_sig: usize, close: LexToken) {
        let frame = self.stack.pop().expect("the root frame is always present");
        let Some(open) = frame.open else {
            // A closer with nothing open under it: blame it, keep the root.
            self.stack.push(frame);
            self.push_diagnostic(DiagnosticKind::MismatchedClose, close.span);
            return;
        };
        if SyntaxKind::expected_close(open) != close.kind {
            self.push_diagnostic(DiagnosticKind::MismatchedClose, close.span);
        }
        let open_sig = frame.open_sig;
        self.flush_dispatches(&frame);
        self.check_odd_map(&frame);
        self.complete_form(open_sig, close_sig);
    }

    /// EOF: unwind from the innermost frame outward.
    fn finish(&mut self) {
        while self.stack.len() > 1 {
            let frame = self.stack.pop().expect("a frame above the root");
            self.flush_dispatches(&frame);
            if let Some(span) = self.open_span(&frame) {
                self.push_diagnostic(DiagnosticKind::UnterminatedCollection, span);
            }
        }
        let root = self.stack.pop().expect("the root frame");
        self.flush_dispatches(&root);
    }

    /// Reports dispatches that never received a form, then drops them.
    fn flush_dispatches(&mut self, frame: &Frame) {
        let pending = frame.dispatch.clone();
        for item in pending {
            let (kind, span) = match item {
                Dispatch::Tag { sig, .. } => {
                    (DiagnosticKind::TagWithoutValue, self.sigs[sig].token.span)
                }
                Dispatch::Discard { sig } => (
                    DiagnosticKind::DiscardWithoutValue,
                    self.sigs[sig].token.span,
                ),
            };
            self.push_diagnostic(kind, span);
        }
    }

    /// A closed map whose last form was a key is odd; at EOF the
    /// unterminated collection says it better, so this runs on close only.
    fn check_odd_map(&mut self, frame: &Frame) {
        if frame.mode == Mode::Map && frame.expecting_value && frame.open.is_some() {
            let span = self.sigs[frame.last_key_sig].token.span;
            self.push_diagnostic(DiagnosticKind::OddMapEntries, span);
        }
    }

    fn open_span(&self, frame: &Frame) -> Option<Span> {
        frame.open.map(|_| self.sigs[frame.open_sig].token.span)
    }

    /// A form spanning `sigs[start..=end]` is finished: resolve the level's
    /// pending dispatches against it (LIFO — each `#tag` or `#_` consumes
    /// exactly one element), then commit it to its collection.
    fn complete_form(&mut self, start: usize, end: usize) {
        let mut start = start;
        loop {
            let pending = self
                .stack
                .last()
                .and_then(|frame| frame.dispatch.last().copied());
            match pending {
                Some(Dispatch::Tag { sig, kind }) => {
                    self.stack
                        .last_mut()
                        .expect("frame with a pending dispatch")
                        .dispatch
                        .pop();
                    let head = self.sigs[start].token;
                    if matches!(
                        kind,
                        SyntaxKind::InstantTag | SyntaxKind::UuidTag | SyntaxKind::ByteTag
                    ) {
                        if head.kind == SyntaxKind::String {
                            let reading = match kind {
                                SyntaxKind::InstantTag => Retag::InstantValue,
                                SyntaxKind::UuidTag => Retag::UuidValue,
                                _ => Retag::ByteValue,
                            };
                            self.add_retag(self.sigs[start].real, reading);
                        } else {
                            self.push_diagnostic(
                                DiagnosticKind::InvalidTaggedValue,
                                self.sigs[sig].token.span,
                            );
                        }
                    } else {
                        self.add_retag(self.sigs[start].real, Retag::TaggedValue);
                    }
                    // The tag and its value are now one element starting at
                    // the tag token; an outer dispatch may still consume it.
                    start = sig;
                }
                Some(Dispatch::Discard { sig: _ }) => {
                    self.stack
                        .last_mut()
                        .expect("frame with a pending dispatch")
                        .dispatch
                        .pop();
                    for index in start..=end {
                        let real = self.sigs[index].real;
                        self.add_retag(real, Retag::Discarded);
                    }
                    // A discarded form is consumed and commits nothing.
                    return;
                }
                None => {
                    self.commit(start, end);
                    return;
                }
            }
        }
    }

    fn commit(&mut self, start: usize, end: usize) {
        let region = Span::new(
            self.sigs[start].token.span.start,
            self.sigs[end].token.span.end,
        );
        let head = self.sigs[start].token;
        // One element: a single token, or a string region whose every piece
        // continues its head token.
        let one_element = start == end
            || (head.kind == SyntaxKind::String
                && (start + 1..=end).all(|index| self.sigs[index].continues));
        let identity = if one_element && !head.has_error() {
            self.source
                .get(region.range())
                .and_then(|text| key_identity(head.kind, text))
        } else {
            None
        };
        let mut ns_key = false;
        let mut duplicate: Option<Retag> = None;
        {
            let Some(frame) = self.stack.last_mut() else {
                self.top_level.push(region);
                return;
            };
            if frame.open.is_none() {
                // A completed top_level form: the stream EDN is designed for.
                self.top_level.push(region);
                return;
            }
            match frame.mode {
                Mode::Seq => {}
                Mode::Map => {
                    if frame.expecting_value {
                        frame.expecting_value = false;
                    } else {
                        frame.expecting_value = true;
                        frame.last_key_sig = start;
                        ns_key =
                            frame.ns_prefixed && one_element && head.kind == SyntaxKind::Keyword;
                        if let Some(id) = identity
                            && !frame.seen.insert(id)
                        {
                            duplicate = Some(Retag::DuplicateKey);
                        }
                    }
                }
                Mode::Set => {
                    if let Some(id) = identity
                        && !frame.seen.insert(id)
                    {
                        duplicate = Some(Retag::DuplicateSetElement);
                    }
                }
            }
        }
        if ns_key {
            self.add_retag(self.sigs[start].real, Retag::NamespacedMapKey);
        }
        if let Some(reading) = duplicate {
            let kind = match reading {
                Retag::DuplicateSetElement => DiagnosticKind::DuplicateSetElement,
                _ => DiagnosticKind::DuplicateKey,
            };
            self.push_diagnostic(kind, region);
            self.add_retag(self.sigs[start].real, reading);
        }
    }

    fn add_retag(&mut self, real: usize, reading: Retag) {
        match self.retags.entry(real) {
            Entry::Occupied(mut entry) => {
                if reading.rank() < entry.get().rank() {
                    entry.insert(reading);
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(reading);
            }
        }
    }

    fn push_diagnostic(&mut self, kind: DiagnosticKind, span: Span) {
        self.diagnostics.push(kind.to_diagnostic(span));
    }
}

/// A comparable identity for a scalar map key or set element, typed by the
/// spec's equality rules. `None` means "not compared": compounds and error
/// spans included.
fn key_identity(kind: SyntaxKind, text: &str) -> Option<String> {
    let tagged = match kind {
        SyntaxKind::Symbol | SyntaxKind::NamespacedSymbol => format!("sym {text}"),
        SyntaxKind::Keyword | SyntaxKind::NamespacedKeyword => format!("kw {text}"),
        SyntaxKind::BooleanLiteral | SyntaxKind::NilLiteral => format!("lit {text}"),
        SyntaxKind::Character => format!("chr {text}"),
        // Raw bytes with quotes and escapes as written: `"\t"` and a literal
        // tab inside quotes are read as different keys (documented).
        SyntaxKind::String => format!("str {text}"),
        SyntaxKind::Integer | SyntaxKind::BigInteger | SyntaxKind::RadixInteger => {
            format!("int {}", canonical_int(text)?)
        }
        SyntaxKind::Ratio => canonical_ratio(text)?,
        SyntaxKind::Float | SyntaxKind::Decimal | SyntaxKind::SpecialNumber => {
            format!("num {}", canonical_float(text)?)
        }
        _ => return None,
    };
    Some(tagged)
}

/// Canonical decimal value of an integer, bigint or radix integer. Values too
/// large for the working width are not compared (they return `None`).
fn canonical_int(text: &str) -> Option<String> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(stripped) => (true, stripped),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let rest = rest.strip_suffix('N').unwrap_or(rest);
    let value: u128 = if let Some(slash) = rest.find(['r', 'R']) {
        let radix: u32 = rest[..slash].parse().ok()?;
        let mut value: u128 = 0;
        for byte in rest[slash + 1..].bytes() {
            let digit = crate::lexer::radix_digit_value(byte)?;
            value = value
                .checked_mul(u128::from(radix))?
                .checked_add(u128::from(digit))?;
        }
        value
    } else {
        rest.parse().ok()?
    };
    if value == 0 {
        return Some("0".to_owned());
    }
    Some(if negative {
        format!("-{value}")
    } else {
        value.to_string()
    })
}

/// Reduced ratio; one whose denominator divides collapses into the integer
/// class, matching what EDN readers yield.
fn canonical_ratio(text: &str) -> Option<String> {
    let (numerator, denominator) = text.split_once('/')?;
    let mut n: i128 = numerator.parse().ok()?;
    let mut d: i128 = denominator.parse().ok()?;
    if d == 0 {
        return None;
    }
    if d < 0 {
        n = -n;
        d = -d;
    }
    let gcd = gcd128(n.unsigned_abs(), d.unsigned_abs());
    n /= i128::try_from(gcd).ok()?;
    d /= i128::try_from(gcd).ok()?;
    if d == 1 {
        Some(format!("int {n}"))
    } else {
        Some(format!("rat {n} {d}"))
    }
}

fn gcd128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Canonical value of a float, decimal or special numeric, through its
/// 64-bit reading.
fn canonical_float(text: &str) -> Option<String> {
    let value = match text {
        "##Inf" => f64::INFINITY,
        "##-Inf" => f64::NEG_INFINITY,
        "##NaN" => return Some("nan".to_owned()),
        other => other.strip_suffix('M').unwrap_or(other).parse().ok()?,
    };
    if value == 0.0 {
        return Some("0".to_owned());
    }
    Some(format!("{value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(source: &str) -> Vec<&'static str> {
        validate(source).iter().map(|d| d.code).collect()
    }

    fn retags_at<'a>(parsed: &Parse<'_>, source: &'a str) -> Vec<(&'a str, Retag)> {
        parsed
            .retags()
            .iter()
            .map(|(index, reading)| {
                (
                    parsed.lexed().tokens()[*index].text(source).unwrap(),
                    *reading,
                )
            })
            .collect()
    }

    #[test]
    fn spec_shaped_documents_have_no_diagnostics() {
        for source in [
            "(a b 42)",
            "[a b 42]",
            "{:a 1, \"foo\" :bar, [1 2 3] four}",
            "#{a b [1 2 3]}",
            "nil true false",
            "42 -7 +5 1000N",
            "1.5 6.022e23 -0.5 1E5 3.25M 7M",
            "2r1010 16rFF 8r777",
            "3/4 -3/4",
            "##Inf ##-Inf ##NaN",
            "\\c \\newline \\space \\u00e9",
            "\"multi\nline\"",
            "#inst \"1985-04-12T23:20:50.52Z\"",
            "#uuid \"f81d4fae-7dec-11d0-a765-00a0c91e6bf6\"",
            "#myapp/Person {:first \"Fred\" :last \"Mertz\"}",
            "[a b #_foo 42]",
            "{:a 1 #_[1 2] :b 3}",
            "#:app{:level :info}",
            "; just a comment",
            "",
            "   \n,;",
            "\u{FEFF}nil",
            "{} [] () #{}",
            "a/b / clojure.string/join",
            ":kw :ns/kw",
        ] {
            let parsed = parse(source);
            assert!(
                parsed.diagnostics().is_empty() && parsed.is_valid(),
                "{source:?}: {:?}",
                parsed.diagnostics()
            );
        }
    }

    #[test]
    fn multiple_top_level_forms_are_an_edn_stream() {
        // The spec defines no enclosing element, so several forms are
        // accepted without warning and each gets a recorded span.
        let parsed = parse("1 [2] {:a 3}");
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        assert_eq!(parsed.top_level_forms().len(), 3);
        assert_eq!(parsed.top_level_forms()[2], Span::new(6, 12));
        assert!(parse("").top_level_forms().is_empty());
    }

    #[test]
    fn discard_marks_every_significant_token_of_its_form() {
        let source = "[a #_[1 {:x y}] b]";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let marks = retags_at(&parsed, source);
        assert_eq!(
            marks,
            vec![
                ("[", Retag::Discarded),
                ("1", Retag::Discarded),
                ("{", Retag::Discarded),
                (":x", Retag::Discarded),
                ("y", Retag::Discarded),
                ("}", Retag::Discarded),
                ("]", Retag::Discarded),
            ]
        );
        assert!(
            parsed
                .retags()
                .iter()
                .all(|(_, reading)| *reading == Retag::Discarded)
        );
    }

    #[test]
    fn consecutive_discards_consume_one_form_each() {
        let source = "[#_#_1 2 3]";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let texts: Vec<&str> = parsed
            .retags()
            .iter()
            .map(|(index, _)| parsed.lexed().tokens()[*index].text(source).unwrap())
            .collect();
        assert_eq!(texts, vec!["1", "2"], "k discards skip k forms");
    }

    #[test]
    fn tag_marks_the_first_token_of_its_value() {
        let source = "#inst \"2026\" #uuid \"u\" #b \"b\" #me/Thing [1 2]";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        assert_eq!(
            retags_at(&parsed, source),
            vec![
                ("\"2026\"", Retag::InstantValue),
                ("\"u\"", Retag::UuidValue),
                ("\"b\"", Retag::ByteValue),
                ("[", Retag::TaggedValue),
            ]
        );
    }

    #[test]
    fn reserved_tags_demand_a_string_payload() {
        for source in ["#b 5", "#inst [1]", "#uuid :nope", "#inst \"ok\" #b \"ok\""] {
            let parsed = parse(source);
            let invalid = parsed
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid-tagged-value")
                .count();
            let expected = usize::from(
                source.starts_with("#b 5") || source.contains("[1]") || source.contains(":nope"),
            );
            assert_eq!(invalid, expected, "{source:?}");
        }
    }

    #[test]
    fn duplicate_map_keys_are_reported_once_per_repeat() {
        let source = "{:a 1 \"s\" 2 \"s\" 3 :a 4}";
        assert_eq!(codes(source), vec!["duplicate-key", "duplicate-key"]);
        let parsed = parse(source);
        assert_eq!(
            retags_at(&parsed, source)
                .into_iter()
                .map(|(text, _)| text)
                .collect::<Vec<_>>(),
            vec!["\"s\"", ":a"]
        );
        assert_eq!(codes("{:a 1 :b 2}"), Vec::<&str>::new());
    }

    #[test]
    fn duplicate_detection_is_typed_and_canonical() {
        // Same value in another notation is still a duplicate.
        assert_eq!(codes("#{255 16rFF}"), vec!["duplicate-set-element"]);
        assert_eq!(codes("#{1 1N}"), vec!["duplicate-set-element"]);
        assert_eq!(codes("#{4/2 2}"), vec!["duplicate-set-element"]);
        // The spec keeps numeric types apart: magnitude alone is not enough.
        assert_eq!(codes("#{1 1.0}"), Vec::<&str>::new());
        assert_eq!(codes("{\"a\" 1 a 2 :a 3}"), Vec::<&str>::new());
        // Compounds are not compared.
        assert_eq!(codes("{[1] :a [1] :b}"), Vec::<&str>::new());
    }

    #[test]
    fn set_duplicates_have_their_own_code() {
        assert_eq!(codes("#{a a}"), vec!["duplicate-set-element"]);
        assert_eq!(
            codes("#{a b b b}"),
            vec!["duplicate-set-element", "duplicate-set-element"]
        );
    }

    #[test]
    fn odd_map_entries_are_reported_on_the_dangling_key() {
        let source = "{:a 1 :b}";
        assert_eq!(codes(source), vec!["odd-map-entries"]);
        let parsed = parse(source);
        assert_eq!(parsed.diagnostics()[0].span, Span::new(6, 8));
        assert_eq!(codes("{:a [1] :b 2}"), Vec::<&str>::new());
    }

    #[test]
    fn mismatched_and_stray_closers_are_reported() {
        assert_eq!(codes("[1)"), vec!["mismatched-close"]);
        assert_eq!(codes("{:a]"), vec!["odd-map-entries", "mismatched-close"]);
        assert_eq!(codes(")"), vec!["mismatched-close"]);
        assert_eq!(codes("#{1]"), vec!["mismatched-close"]);
        // A vector inside a map still closes its own kind cleanly.
        assert_eq!(codes("{[1] :b}"), Vec::<&str>::new());
    }

    #[test]
    fn unclosed_collections_are_reported_at_their_opener() {
        let source = "[1 (2";
        assert_eq!(
            codes(source),
            vec!["unterminated-collection", "unterminated-collection"]
        );
        let parsed = parse(source);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.diagnostics()[0].span, Span::new(0, 1));
        assert_eq!(parsed.diagnostics()[1].span, Span::new(3, 4));
        // The discard is satisfied by `2`; only the open bracket is unclosed.
        assert_eq!(codes("(1 #_2"), vec!["unterminated-collection"]);
    }

    #[test]
    fn tags_and_discards_without_a_value_are_reported() {
        assert_eq!(codes("#inst"), vec!["tag-without-value"]);
        assert_eq!(
            codes("#my/tag ["),
            vec!["tag-without-value", "unterminated-collection"]
        );
        assert_eq!(codes("[#uuid]"), vec!["tag-without-value"]);
        assert_eq!(codes("#_"), vec!["discard-without-value"]);
        assert_eq!(codes("[1 #_]"), vec!["discard-without-value"]);
        assert_eq!(codes("#_ ;c"), vec!["discard-without-value"]);
        // A discard consumed by the end of its own collection.
        assert_eq!(codes("{:a 1 #_}"), vec!["discard-without-value"]);
    }

    #[test]
    fn namespaced_map_prefix_requires_a_map() {
        assert_eq!(codes("#:app{:a 1}"), Vec::<&str>::new());
        assert_eq!(codes("#:app [1]"), vec!["namespaced-map-without-map"]);
        assert_eq!(codes("#:app"), vec!["namespaced-map-without-map"]);
    }

    #[test]
    fn namespaced_map_keys_are_marked_only_where_the_prefix_reaches() {
        let source = "#:app{:level :info :meta {:level 2}}";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let marked = retags_at(&parsed, source);
        assert_eq!(marked.len(), 2, "only the direct plain keys of the map");
        assert_eq!(marked[0], (":level", Retag::NamespacedMapKey));
        assert_eq!(marked[1], (":meta", Retag::NamespacedMapKey));
        // Namespaced keywords already carry their ns and are never marked.
        let parsed = parse("#:app{:a/b 1}");
        assert!(parsed.retags().is_empty());
    }

    #[test]
    fn discarded_and_tagged_constructs_compose() {
        // The discard eats the whole tagged construct, not just its string.
        let source = "[#_#b \"bytes\" 1]";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let texts: Vec<(&str, Retag)> = retags_at(&parsed, source);
        assert!(texts.contains(&("#b", Retag::Discarded)));
        assert!(texts.contains(&("\"bytes\"", Retag::Discarded)));
        assert!(
            !texts
                .iter()
                .any(|(_, reading)| *reading == Retag::ByteValue),
            "the discarded construct keeps no value reading"
        );
    }

    #[test]
    fn every_diagnostic_code_is_reachable_and_unique() {
        let all = [
            DiagnosticKind::DiscardWithoutValue,
            DiagnosticKind::DuplicateKey,
            DiagnosticKind::DuplicateSetElement,
            DiagnosticKind::IncompleteDispatch,
            DiagnosticKind::InvalidEscape,
            DiagnosticKind::InvalidNumber,
            DiagnosticKind::InvalidRadix,
            DiagnosticKind::InvalidRadixDigit,
            DiagnosticKind::InvalidSymbol,
            DiagnosticKind::InvalidTaggedValue,
            DiagnosticKind::MalformedChar,
            DiagnosticKind::MismatchedClose,
            DiagnosticKind::NamespacedMapWithoutMap,
            DiagnosticKind::NonEdnConstruct,
            DiagnosticKind::OddMapEntries,
            DiagnosticKind::TagWithoutValue,
            DiagnosticKind::UnterminatedCollection,
            DiagnosticKind::UnterminatedString,
        ];
        let mut wire: Vec<&str> = all.iter().map(|kind| kind.code()).collect();
        assert_eq!(wire.len(), all.len(), "no variant listed twice");
        wire.sort_unstable();
        let before = wire.len();
        wire.dedup();
        assert_eq!(wire.len(), before, "codes are unique");
        for kind in all {
            assert!(!kind.message().is_empty());
            assert!(kind.code().is_ascii());
            assert!(!kind.code().contains(' '), "{}", kind.code());
            assert_eq!(kind.to_diagnostic(Span::new(0, 1)).code, kind.code());
            assert_eq!(format!("{kind}"), kind.message());
        }
    }

    #[test]
    fn each_required_fault_carries_its_stable_code() {
        // One table: source → exact codes, proving coverage of every code
        // the contract requires, at the position of a real token.
        for (source, expected) in [
            ("\"oops", &["unterminated-string"][..]),
            ("[1 2", &["unterminated-collection"]),
            ("(a]", &["mismatched-close"]),
            ("2r1020", &["invalid-radix-digit"]),
            ("1r11", &["invalid-radix"]),
            ("\\nbsp", &["malformed-char"]),
            ("{:a 1 :a 2}", &["duplicate-key"]),
            ("#inst", &["tag-without-value"]),
            ("#_", &["discard-without-value"]),
            ("@a", &["non-edn-construct"]),
            ("^:m {}", &["non-edn-construct"]),
            ("#", &["incomplete-dispatch"]),
            ("a//b", &["invalid-symbol"]),
            ("\"a \\q\"", &["invalid-escape"]),
            ("1e", &["invalid-number"]),
            ("#{a a}", &["duplicate-set-element"]),
            ("#b 1", &["invalid-tagged-value"]),
            ("#:a [1]", &["namespaced-map-without-map"]),
            ("{:a}", &["odd-map-entries"]),
            ("\"x", &["unterminated-string"]),
        ] {
            let observed: Vec<&str> = codes(source);
            assert_eq!(observed, expected, "for {source:?}");
            for diagnostic in validate(source) {
                assert!(!diagnostic.span.is_empty(), "{diagnostic:?} is zero-width");
                assert!(
                    diagnostic.span.is_valid_for(source),
                    "{diagnostic:?} escapes the source"
                );
            }
        }
    }

    #[test]
    fn diagnostics_are_in_source_order() {
        let source = "] [1 \"oops #_";
        let diagnostics = validate(source);
        assert!(
            diagnostics
                .windows(2)
                .all(|pair| pair[0].span.start <= pair[1].span.start),
            "{diagnostics:?}"
        );
        let observed: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(
            observed,
            vec![
                "mismatched-close",
                "unterminated-collection",
                "unterminated-string",
            ]
        );
    }

    #[test]
    fn recovery_keeps_the_stream_lossless() {
        let broken = concat!(
            "\u{FEFF}{:a [1 2, #_}\n",
            "#inst 5 #:x [1] \"unterminated\n",
        );
        let parsed = parse(broken);
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), broken);
        assert!(!parsed.is_valid());
    }

    #[test]
    fn empty_and_whitespace_only_input_parse_clean() {
        for source in ["", "  \n,", "\u{FEFF}"] {
            let parsed = parse(source);
            assert!(parsed.is_valid(), "{source:?}");
            assert!(parsed.top_level_forms().is_empty());
        }
    }

    #[test]
    fn every_truncation_stays_lossless_and_progresses() {
        let sample = concat!(
            "#:app{:a #_(1 [2 #{\\a \"s\\\"t\" :k #inst \"2026\"}])\n",
            " :b #b \"\\u0000\" :c [2r10 16rFF ##NaN 3/4]} ; done\n",
        );
        for cut in 0..=sample.len() {
            let source = &sample[..cut];
            let parsed = parse(source);
            assert!(parsed.lexed().is_lossless(), "{source:?}");
            assert_eq!(parsed.lexed().joined(), source, "{source:?}");
            for (index, reading) in parsed.retags() {
                assert!(*index < parsed.lexed().tokens().len(), "{source:?}");
                let _ = reading;
            }
            for form in parsed.top_level_forms() {
                assert!(!form.is_empty(), "{source:?}");
                assert!(form.is_valid_for(source), "{source:?}");
            }
        }
    }
}
