//! The iCalendar structure pass: components, parameters, value types.
//!
//! The lexer hands over physical lines; this pass re-joins them. A logical
//! content line is its name, its parameters and the value bytes that follow the
//! `:` — including the bytes a fold pushed onto the next physical line — and it
//! is that re-joining, and nothing else, that lets a value be *typed*. Not one
//! span moves: every diagnostic and every semantic reading points at bytes the
//! lexer already emitted, so a broken document still round-trips byte-for-byte
//! while [`Parse::is_valid`] reports `false`.
//!
//! Three things are decided here rather than in the lexer, because each needs
//! the line or the component around it:
//!
//! * **Component structure.** `BEGIN`/`END` are matched, nested and closed, so
//!   an unclosed `VEVENT` and an `END:VTODO` that answers a `BEGIN:VEVENT` can
//!   each be named. A property is placed in the component that owns it, which is
//!   how a required property can be reported missing.
//! * **Value type.** RFC 5545 types a value by its property name, overridden by
//!   a `VALUE=` parameter: the same bytes are a `DATE` in one line and a
//!   `DURATION` in another. The lexer therefore keeps values opaque.
//! * **Case.** Names are matched case-insensitively, so a lowercase name is
//!   still the name it is — and only here does it become a warning.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use themoretheless_tokenizer_core::{Severity, Span};

use crate::lexer::{LexToken, Lexed, SyntaxKind, lex};

/// An iCalendar violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A `BEGIN` component was never closed by a matching `END`.
    UnclosedComponent,
    /// An `END` appeared with no open component to close.
    UnexpectedEnd,
    /// A component was closed by an `END` naming a different component.
    ComponentMismatch,
    /// The document has content but no `VCALENDAR` wrapped around it.
    MissingVcalendarWrapper,
    /// A property line appeared outside any component.
    PropertyBeforeBegin,
    /// A `BEGIN` or `END` line whose value — the component name — is empty.
    MissingComponentName,
    /// Bytes the content-line grammar has no place for.
    InvalidContentLine,
    /// A content line with a name that never reached its `:`.
    MissingValueDelimiter,
    /// Whitespace at the start of a line whose predecessor had already ended.
    NothingToFold,
    /// A quoted parameter value that never reached its closing quote.
    UnterminatedQuotedParam,
    /// A backslash sequence that is not one of RFC 5545's text escapes.
    InvalidEscape,
    /// A line ending that is not the `CRLF` the specification demands.
    InvalidLineEnding,
    /// A `DATE` value that is not eight digits naming a real calendar day.
    MalformedDate,
    /// A `DATE-TIME` value that is not `YYYYMMDDTHHMMSS` with optional `Z`.
    MalformedDateTime,
    /// A `DURATION` value that is not `P…D`, `P…W` or `PT…H…M…S`.
    MalformedDuration,
    /// A `PERIOD` value that is not `<date-time>/<date-time|duration>`.
    MalformedPeriod,
    /// A component lacked a property its specification marks as required.
    MissingRequiredProperty,
    /// A name was written with lowercase letters.
    NonUppercaseName,
}

impl DiagnosticKind {
    /// Every kind the engine can raise.
    pub const ALL: [Self; 18] = [
        Self::ComponentMismatch,
        Self::InvalidContentLine,
        Self::InvalidEscape,
        Self::InvalidLineEnding,
        Self::MalformedDate,
        Self::MalformedDateTime,
        Self::MalformedDuration,
        Self::MalformedPeriod,
        Self::MissingComponentName,
        Self::MissingRequiredProperty,
        Self::MissingValueDelimiter,
        Self::MissingVcalendarWrapper,
        Self::NonUppercaseName,
        Self::NothingToFold,
        Self::PropertyBeforeBegin,
        Self::UnexpectedEnd,
        Self::UnterminatedQuotedParam,
        Self::UnclosedComponent,
    ];

    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnclosedComponent => "unclosed-component",
            Self::UnexpectedEnd => "unexpected-end",
            Self::ComponentMismatch => "component-mismatch",
            Self::MissingVcalendarWrapper => "missing-vcalendar-wrapper",
            Self::PropertyBeforeBegin => "property-before-begin",
            Self::MissingComponentName => "missing-component-name",
            Self::InvalidContentLine => "invalid-content-line",
            Self::MissingValueDelimiter => "missing-value-delimiter",
            Self::NothingToFold => "nothing-to-fold",
            Self::UnterminatedQuotedParam => "unterminated-quoted-param",
            Self::InvalidEscape => "invalid-escape",
            Self::InvalidLineEnding => "invalid-line-ending",
            Self::MalformedDate => "malformed-date",
            Self::MalformedDateTime => "malformed-date-time",
            Self::MalformedDuration => "malformed-duration",
            Self::MalformedPeriod => "malformed-period",
            Self::MissingRequiredProperty => "missing-required-property",
            Self::NonUppercaseName => "non-uppercase-name",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnclosedComponent => "component opened by BEGIN is never closed by END",
            Self::UnexpectedEnd => "END has no open component to close",
            Self::ComponentMismatch => "END names a different component than BEGIN",
            Self::MissingVcalendarWrapper => "content is not wrapped in a VCALENDAR component",
            Self::PropertyBeforeBegin => "property appears outside any component",
            Self::MissingComponentName => "BEGIN or END names no component",
            Self::InvalidContentLine => "line does not match name *(\";\" param) \":\" value",
            Self::MissingValueDelimiter => "content line has a name but no value delimiter",
            Self::NothingToFold => "fold continuation has no content line to join",
            Self::UnterminatedQuotedParam => "quoted parameter value is never closed",
            Self::InvalidEscape => "backslash does not begin an RFC 5545 text escape",
            Self::InvalidLineEnding => "line is not terminated by CRLF",
            Self::MalformedDate => "value is not a DATE of the form YYYYMMDD",
            Self::MalformedDateTime => "value is not a DATE-TIME of the form YYYYMMDDTHHMMSS[Z]",
            Self::MalformedDuration => "value is not a DURATION of the form P… or PT…",
            Self::MalformedPeriod => "value is not a PERIOD of the form start/end",
            Self::MissingRequiredProperty => {
                "component lacks a property its RFC 5545 conformance rule requires"
            }
            Self::NonUppercaseName => "name uses lowercase letters",
        }
    }

    /// How serious the violation is.
    ///
    /// RFC 5545 states every rule above with MUST, so each is an error. Casing
    /// alone is a [`Severity::Warning`]: the specification matches names
    /// case-insensitively and only asks writers to emit upper case, so a
    /// lowercase name is non-conformant output rather than unreadable input.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::NonUppercaseName => Severity::Warning,
            _ => Severity::Error,
        }
    }

    /// The diagnostic this kind raises at `span`.
    #[must_use]
    pub const fn issue(self, span: Span) -> Diagnostic {
        Diagnostic {
            span,
            code: self.code(),
            message: self.message(),
            severity: self.severity(),
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

    fn severity(self) -> Severity {
        DiagnosticKind::severity(self)
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// A source-attached diagnostic with the severity the host should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub span: Span,
    pub code: &'static str,
    pub message: &'static str,
    pub severity: Severity,
}

impl Diagnostic {
    #[must_use]
    pub const fn span(self) -> Span {
        self.span
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        self.message
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// The value type RFC 5545 assigns a property, given its name and the `VALUE=`
/// parameter that may override that default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueType {
    /// `TEXT`, the default every property not listed below falls back to.
    Text,
    /// `URI`.
    Uri,
    /// `DATE`: a calendar day, no time.
    Date,
    /// `DATE-TIME`: a day and a time, optionally in UTC.
    DateTime,
    /// `DURATION`: a signed amount of elapsed time.
    Duration,
    /// `DURATION` or `DATE-TIME`, which `TRIGGER` allows.
    DurationOrDateTime,
    /// `PERIOD`: a start with an end or a duration, optionally a list.
    Period,
    /// `RECUR`: a recurrence rule.
    Recurrence,
}

/// One component instance, from its `BEGIN` to its `END` when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// The component name's bytes, e.g. `VEVENT`.
    pub name: Span,
    /// The `BEGIN` marker's span.
    pub marker: Span,
    /// The `END` marker's span, when the component was closed.
    pub end: Option<Span>,
    /// Nesting depth, with the outermost component at depth 0.
    pub depth: usize,
    /// Spans of the property names this component owns directly.
    pub properties: Vec<Span>,
}

/// One logical content line, folded back together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentLine {
    /// First byte of the line through its line break, or through the last byte
    /// the input has when the line is unterminated.
    pub span: Span,
    /// Index range of this line's tokens in the lexer's stream.
    pub tokens: Range<usize>,
    /// The name token: a property name or a `BEGIN`/`END` marker.
    pub name: Option<Span>,
    /// The `:` token, when the line reached one.
    pub delimiter: Option<Span>,
    /// The value's byte spans, one per physical line it was folded over.
    pub value: Vec<Span>,
    /// The value type the specification gives this property.
    pub value_type: ValueType,
    /// Whether the line opened or closed a component.
    pub is_structure: bool,
}

/// Lossless tokens plus iCalendar diagnostics, structure and semantic readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    components: Vec<Component>,
    lines: Vec<ContentLine>,
    retags: HashMap<usize, &'static str>,
    valid: bool,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Error-severity diagnostics only.
    pub fn errors(&self) -> impl Iterator<Item = Diagnostic> + '_ {
        self.diagnostics
            .iter()
            .copied()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// [`Severity::Warning`] diagnostics only.
    pub fn warnings(&self) -> impl Iterator<Item = Diagnostic> + '_ {
        self.diagnostics
            .iter()
            .copied()
            .filter(|diagnostic| diagnostic.severity == Severity::Warning)
    }

    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components
    }

    #[must_use]
    pub fn lines(&self) -> &[ContentLine] {
        &self.lines
    }

    /// The semantic reading of a token's span: what the structure says the bytes
    /// are, which is where a `DATE` differs from the text next to it.
    #[must_use]
    pub fn semantic_kind(&self, token: LexToken) -> &'static str {
        if token.has_error() {
            return "error";
        }
        if let Some(kind) = self.retags.get(&token.span.start) {
            return kind;
        }
        crate::lex_host_kind(token.kind)
    }

    /// Whether the document carries no error-severity violation.
    ///
    /// A [`DiagnosticKind::NonUppercaseName`] warning alone does not make a
    /// document invalid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.valid
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
    let mut build = Builder {
        source,
        lexed: &lexed,
        diagnostics: Vec::new(),
        components: Vec::new(),
        open: Vec::new(),
        lines: Vec::new(),
        retags: HashMap::new(),
        saw_content_line: false,
        saw_root_calendar: false,
    };
    build.run();
    let Builder {
        mut diagnostics,
        mut open,
        components,
        lines,
        retags,
        saw_content_line,
        saw_root_calendar,
        ..
    } = build;
    // An unclosed component still owns the properties it collected, so those are
    // checked before the fact that it never closed is reported. Innermost first.
    while let Some(component) = open.pop() {
        check_required(source, &mut diagnostics, &component);
        diagnostics.push(DiagnosticKind::UnclosedComponent.issue(component.marker));
    }
    if saw_content_line
        && !saw_root_calendar
        && let Some(first) = lines.first()
    {
        let span = first.name.unwrap_or(first.span);
        if !span.is_empty() {
            diagnostics.push(DiagnosticKind::MissingVcalendarWrapper.issue(span));
        }
    }
    diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
    diagnostics.dedup_by(|a, b| a.code == b.code && a.span == b.span);
    let valid = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error);
    Parse {
        lexed,
        diagnostics,
        components,
        lines,
        retags,
        valid,
    }
}

/// Structural diagnostics only.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).into_diagnostics()
}

/// A component that has been opened and not yet closed.
#[derive(Debug)]
struct OpenComponent {
    name: String,
    name_span: Span,
    marker: Span,
    depth: usize,
    properties: Vec<Span>,
}

struct Builder<'a, 'source> {
    source: &'source str,
    lexed: &'a Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    components: Vec<Component>,
    open: Vec<OpenComponent>,
    lines: Vec<ContentLine>,
    retags: HashMap<usize, &'static str>,
    saw_content_line: bool,
    saw_root_calendar: bool,
}

impl<'lexed, 'source> Builder<'lexed, 'source> {
    fn run(&mut self) {
        let tokens = self.lexed.tokens();
        let mut index = 0usize;
        while index < tokens.len() {
            match tokens[index].kind {
                // A fold marker only joins lines from *inside* a line. Reaching
                // one here means no content line was open, which is the whole
                // of what `nothing-to-fold` claims.
                SyntaxKind::FoldMarker => {
                    self.push(DiagnosticKind::NothingToFold, tokens[index].span);
                    index += 1;
                }
                SyntaxKind::Bom | SyntaxKind::LineBreak => index += 1,
                _ => index = self.lex_logical_line(tokens, index),
            }
        }
    }

    /// Consumes one logical line — its tokens through the `LineBreak` that ends
    /// it, or through the end of the stream — and returns the next index. The
    /// index always advances by at least one token.
    fn lex_logical_line(&mut self, tokens: &[LexToken], start: usize) -> usize {
        let mut end = start;
        loop {
            let closed = tokens[end].kind == SyntaxKind::LineBreak;
            end += 1;
            if closed || end >= tokens.len() {
                break;
            }
        }
        self.analyze(start..end);
        end
    }

    fn analyze(&mut self, range: Range<usize>) {
        let tokens = self.lexed.tokens();
        let line: &[LexToken] = &tokens[range.clone()];
        let span = Span::new(line[0].span.start, line[line.len() - 1].span.end);
        self.saw_content_line = true;

        let name = line.iter().copied().find(|token| {
            matches!(
                token.kind,
                SyntaxKind::PropertyName | SyntaxKind::StructureMarker
            )
        });
        let delimiter = line
            .iter()
            .copied()
            .find(|token| token.kind == SyntaxKind::ValueColon);
        let value: Vec<LexToken> = delimiter.map_or_else(Vec::new, |colon| {
            line.iter()
                .copied()
                .filter(|token| token.span.start > colon.span.start)
                .filter(|token| matches!(token.kind, SyntaxKind::Value | SyntaxKind::Escape))
                .collect()
        });
        // The lexer flagged what it could not place; here each flag becomes the
        // code that names the fault.
        for token in line {
            if token.has_error() {
                self.flagged(*token);
            }
        }
        self.check_case(line);

        let is_structure = name.is_some_and(|token| token.kind == SyntaxKind::StructureMarker);
        let name_text = name.map_or_else(String::new, |token| self.text(token.span).to_owned());
        let value_type = if is_structure {
            ValueType::Text
        } else {
            value_type_for(&name_text, self.declared_value_type(line))
        };
        if let Some(token) = name
            && delimiter.is_none()
        {
            self.push(DiagnosticKind::MissingValueDelimiter, token.span);
        }

        if let Some(marker) = name.filter(|_| is_structure) {
            self.structure_line(marker, &name_text, delimiter, &value);
        } else if let Some(name_token) = name {
            self.property_line(name_token, &value, value_type);
            if let Some(opened) = self.open.last_mut() {
                opened.properties.push(name_token.span);
            }
        }

        self.lines.push(ContentLine {
            span,
            tokens: range,
            name: name.map(|token| token.span),
            delimiter: delimiter.map(|token| token.span),
            value: value.iter().map(|token| token.span).collect(),
            value_type,
            is_structure,
        });
    }

    /// A `BEGIN` or `END` line. The value's bytes name the component, which is
    /// why the semantic layer reads them differently from an ordinary value.
    fn structure_line(
        &mut self,
        marker: LexToken,
        marker_text: &str,
        delimiter: Option<LexToken>,
        value: &[LexToken],
    ) {
        let name = joined(self.source, value);
        let name_span = value
            .iter()
            .find(|token| token.kind == SyntaxKind::Value)
            .map_or(marker.span, |token| token.span);
        if name.is_empty() {
            let span = delimiter.map_or(marker.span, |token| token.span);
            self.push(DiagnosticKind::MissingComponentName, span);
        } else {
            for token in value {
                if token.kind == SyntaxKind::Value {
                    self.retags.insert(token.span.start, "component-name");
                }
            }
        }
        if marker_text.eq_ignore_ascii_case("BEGIN") {
            let depth = self.open.len();
            if depth == 0 && name.eq_ignore_ascii_case("VCALENDAR") {
                self.saw_root_calendar = true;
            }
            self.open.push(OpenComponent {
                name,
                name_span,
                marker: marker.span,
                depth,
                properties: Vec::new(),
            });
        } else {
            self.close_component(&name, marker.span);
        }
    }

    /// An `END` line: the open component it answers, if any, is finished.
    fn close_component(&mut self, name: &str, marker: Span) {
        let Some(opened) = self.open.pop() else {
            self.push(DiagnosticKind::UnexpectedEnd, marker);
            return;
        };
        if !name.is_empty() && !opened.name.eq_ignore_ascii_case(name) {
            self.push(DiagnosticKind::ComponentMismatch, marker);
        }
        check_required(self.source, &mut self.diagnostics, &opened);
        self.components.push(Component {
            name: opened.name_span,
            marker: opened.marker,
            end: Some(marker),
            depth: opened.depth,
            properties: opened.properties,
        });
    }

    fn property_line(&mut self, name: LexToken, value: &[LexToken], value_type: ValueType) {
        if self.open.is_empty() {
            self.push(DiagnosticKind::PropertyBeforeBegin, name.span);
        }
        let text = joined(self.source, value);
        if text.is_empty() {
            // RFC 5545 lets a value be empty. An empty value has no type to
            // check and no bytes to retag, so nothing is invented for it.
            return;
        }
        let runs: Vec<Span> = value
            .iter()
            .filter(|token| token.kind == SyntaxKind::Value)
            .map(|token| token.span)
            .collect();
        let Some(first) = runs.first().copied() else {
            return;
        };
        match check_value(&text, value_type) {
            Some(kind) => {
                for span in runs {
                    self.retags.insert(span.start, kind);
                }
            }
            None => {
                let diagnostic = match value_type {
                    ValueType::Date => DiagnosticKind::MalformedDate,
                    ValueType::DateTime | ValueType::DurationOrDateTime => {
                        DiagnosticKind::MalformedDateTime
                    }
                    ValueType::Duration => DiagnosticKind::MalformedDuration,
                    ValueType::Period => DiagnosticKind::MalformedPeriod,
                    ValueType::Text | ValueType::Uri | ValueType::Recurrence => return,
                };
                self.push(diagnostic, first);
            }
        }
    }

    /// Names match case-insensitively, so a lowercase name is still its name;
    /// RFC 5545 §3.1 only asks writers to emit upper case, so it is a warning.
    fn check_case(&mut self, line: &[LexToken]) {
        for token in line.iter().copied().filter(|token| {
            matches!(
                token.kind,
                SyntaxKind::PropertyName | SyntaxKind::StructureMarker | SyntaxKind::ParameterName
            )
        }) {
            if !is_uppercase_name(self.text(token.span)) {
                self.push(DiagnosticKind::NonUppercaseName, token.span);
            }
        }
    }

    /// The `VALUE=` parameter that overrides a property's default type, with the
    /// surrounding quotes of a quoted value removed.
    fn declared_value_type(&self, line: &[LexToken]) -> Option<&'source str> {
        let mut cursor = 0usize;
        while cursor + 2 < line.len() {
            if line[cursor].kind == SyntaxKind::ParameterName
                && self.text(line[cursor].span).eq_ignore_ascii_case("VALUE")
                && line[cursor + 1].kind == SyntaxKind::ParameterEquals
                && matches!(
                    line[cursor + 2].kind,
                    SyntaxKind::BareParam | SyntaxKind::QuotedParam
                )
            {
                let text = self.text(line[cursor + 2].span);
                return Some(
                    text.strip_prefix('"')
                        .unwrap_or(text)
                        .strip_suffix('"')
                        .unwrap_or(text),
                );
            }
            cursor += 1;
        }
        None
    }

    /// Turns a token the lexer already flagged into the code that names it.
    fn flagged(&mut self, token: LexToken) {
        let kind = match token.kind {
            SyntaxKind::QuotedParam => DiagnosticKind::UnterminatedQuotedParam,
            SyntaxKind::Escape => DiagnosticKind::InvalidEscape,
            SyntaxKind::LineBreak => DiagnosticKind::InvalidLineEnding,
            _ => DiagnosticKind::InvalidContentLine,
        };
        self.push(kind, token.span);
    }

    fn push(&mut self, kind: DiagnosticKind, span: Span) {
        // Every diagnostic reports bytes that exist: no zero-width span, and no
        // offset outside the source.
        if span.is_empty() || !span.is_valid_for(self.source) {
            return;
        }
        self.diagnostics.push(kind.issue(span));
    }

    fn text(&self, span: Span) -> &'source str {
        let source: &'source str = self.source;
        span.slice(source).unwrap_or_default()
    }
}

/// The logical bytes of a value region: its physical runs joined in order,
/// which is exactly what dropping each fold's `CRLF` plus one whitespace leaves.
fn joined(source: &str, tokens: &[LexToken]) -> String {
    let mut joined = String::new();
    for token in tokens {
        if let Some(text) = token.span.slice(source) {
            joined.push_str(text);
        }
    }
    joined
}

/// Properties every instance of a component must carry, per RFC 5545 §3.6.
fn required_properties(name: &str) -> &'static [&'static str] {
    match name.to_ascii_uppercase().as_str() {
        "VCALENDAR" => &["VERSION", "PRODID"],
        "VEVENT" | "VTODO" | "VJOURNAL" | "VFREEBUSY" => &["UID", "DTSTAMP"],
        "VALARM" => &["ACTION", "TRIGGER"],
        "VTIMEZONE" => &["TZID"],
        "STANDARD" | "DAYLIGHT" => &["DTSTART", "TZOFFSETFROM", "TZOFFSETTO", "TZNAME"],
        _ => &[],
    }
}

/// One diagnostic per absent required property, all at the component's `BEGIN`
/// marker. `parse` collapses identical `(code, span)` pairs, so a component
/// missing two properties is reported once: the second copy would name the same
/// bytes with the same message and say nothing more.
fn check_required(source: &str, diagnostics: &mut Vec<Diagnostic>, opened: &OpenComponent) {
    for required in required_properties(&opened.name) {
        let present = opened
            .properties
            .iter()
            .any(|span| property_matches(source, *span, required));
        if !present {
            diagnostics.push(DiagnosticKind::MissingRequiredProperty.issue(opened.marker));
        }
    }
}

/// Whether `name` is a property whose text equals `required`, ignoring case.
fn property_matches(source: &str, span: Span, required: &str) -> bool {
    span.slice(source)
        .is_some_and(|text| text.eq_ignore_ascii_case(required))
}

/// The value type a property name declares. RFC 5545 §3.3.10 through §3.3.15 fix
/// these defaults; everything the specification leaves open is `TEXT`, which is
/// also what an unknown `VALUE=` falls back to.
fn value_type_for(name: &str, declared: Option<&str>) -> ValueType {
    if let Some(declared) = declared {
        match declared.to_ascii_uppercase().as_str() {
            "DATE" => return ValueType::Date,
            "DATE-TIME" => return ValueType::DateTime,
            "DURATION" => return ValueType::Duration,
            "PERIOD" => return ValueType::Period,
            "RECUR" => return ValueType::Recurrence,
            "URI" => return ValueType::Uri,
            "TEXT" => return ValueType::Text,
            _ => {}
        }
    }
    match name.to_ascii_uppercase().as_str() {
        "DTSTART" | "DTEND" | "DUE" | "COMPLETED" | "CREATED" | "DTSTAMP" | "LAST-MODIFIED"
        | "EXDATE" | "RDATE" => ValueType::DateTime,
        "DURATION" => ValueType::Duration,
        "TRIGGER" => ValueType::DurationOrDateTime,
        "FREEBUSY" => ValueType::Period,
        "RRULE" | "EXRULE" => ValueType::Recurrence,
        "URL" | "ATTACH" | "ALTREP" => ValueType::Uri,
        _ => ValueType::Text,
    }
}

/// Whether the value really is of its type. `None` says it is not. Recurrence
/// rules, URIs and text are typed but not shape-checked: the specification gives
/// them no closed grammar for this engine to enforce, and inventing one would
/// turn valid documents into flagged ones.
fn check_value(value_text: &str, value_type: ValueType) -> Option<&'static str> {
    let ok = match value_type {
        ValueType::Date => value_text.split(',').all(is_date),
        ValueType::DateTime => value_text.split(',').all(is_date_time),
        ValueType::Duration => value_text.split(',').all(is_duration),
        ValueType::Period => value_text.split(',').all(is_period),
        ValueType::DurationOrDateTime => {
            if value_text.split(',').all(is_duration) {
                return Some(ValueType::Duration.kind_name());
            }
            value_text.split(',').all(is_date_time)
        }
        ValueType::Text | ValueType::Uri | ValueType::Recurrence => true,
    };
    if ok {
        Some(value_type.kind_name())
    } else {
        None
    }
}

impl ValueType {
    /// The kind name the semantic layer reads a value of this type as.
    #[must_use]
    pub const fn kind_name(self) -> &'static str {
        match self {
            Self::Date => "date-value",
            Self::DateTime => "date-time-value",
            Self::Duration => "duration-value",
            Self::DurationOrDateTime => "date-time-value",
            Self::Period => "period-value",
            Self::Recurrence => "recurrence-value",
            Self::Uri => "uri-value",
            Self::Text => "text-value",
        }
    }
}

fn is_uppercase_name(text: &str) -> bool {
    !text.bytes().any(|byte| byte.is_ascii_lowercase())
}

/// `YYYYMMDD` for a day that exists.
fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 8 || !bytes.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let month = slice_number(text, 4, 6);
    let day = slice_number(text, 6, 8);
    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(slice_number(text, 0, 4), month)
}

/// `YYYYMMDDTHHMMSS`, optionally suffixed with `Z` for UTC. A floating local
/// time — the same shape without `Z` — is legal, with or without a `TZID`.
fn is_date_time(text: &str) -> bool {
    let local = text.strip_suffix('Z').unwrap_or(text);
    if local.len() != 15 || !local.is_ascii() {
        return false;
    }
    local.as_bytes()[8] == b'T' && is_date(&local[..8]) && is_time(&local[9..])
}

/// `HHMMSS`, with seconds up to 60 so a leap second is not a violation.
fn is_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 6
        && bytes.iter().all(u8::is_ascii_digit)
        && (0..=23).contains(&slice_number(text, 0, 2))
        && (0..=59).contains(&slice_number(text, 2, 4))
        && (0..=60).contains(&slice_number(text, 4, 6))
}

/// `P`, then either a week count, or a day count with an optional `T` time part,
/// or a bare `T` time part — and at least one unit.
fn is_duration(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let Some(rest) = body.strip_prefix('P') else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    if let Some(weeks) = rest.strip_suffix('W') {
        return !weeks.is_empty()
            && weeks.bytes().all(|byte| byte.is_ascii_digit())
            && !weeks.contains('-');
    }
    let (date_part, time_part) = match rest.split_once('T') {
        Some((date_part, time_part)) => (date_part, Some(time_part)),
        None => (rest, None),
    };
    if let Some(days) = date_part.strip_suffix('D') {
        if days.is_empty() || !days.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
    } else if !date_part.is_empty() {
        return false;
    }
    match time_part {
        // A date part alone is a duration; `P` on its own was rejected above.
        None => !date_part.is_empty(),
        Some(time_part) => !time_part.is_empty() && is_time_units(time_part),
    }
}

/// `H`, `M` and `S` amounts in that order, each at most once, each preceded by
/// digits. The unit bytes ascend, so ordering them is enough.
fn is_time_units(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut last: Option<u8> = None;
    let mut saw_any = false;
    while index < bytes.len() {
        let digits = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == digits || index >= bytes.len() {
            return false;
        }
        let unit = bytes[index];
        if !matches!(unit, b'H' | b'M' | b'S') || last.is_some_and(|last| unit <= last) {
            return false;
        }
        last = Some(unit);
        index += 1;
        saw_any = true;
    }
    saw_any
}

/// `<date-time>/<date-time>` or `<date-time>/<duration>`.
fn is_period(text: &str) -> bool {
    let Some((start, finish)) = text.split_once('/') else {
        return false;
    };
    is_date_time(start) && (is_date_time(finish) || is_duration(finish))
}

fn slice_number(text: &str, from: usize, to: usize) -> u32 {
    text.get(from..to)
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(u32::MAX)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_unique_stable_code_and_message() {
        let mut seen: Vec<&str> = DiagnosticKind::ALL.iter().map(|kind| kind.code()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "codes are unique");
        assert_eq!(before, 18);
        for kind in DiagnosticKind::ALL {
            assert!(!kind.message().is_empty(), "{kind:?}");
            let issue = kind.issue(Span::new(0, 1));
            assert_eq!(issue.code(), kind.code());
            assert_eq!(issue.message(), kind.message());
            assert_eq!(issue.severity, kind.severity());
            assert_eq!(format!("{kind}"), kind.message());
        }
    }

    #[test]
    fn value_shapes_are_checked_by_type_alone() {
        for text in ["20260924", "20240229", "19000101", "00010101", "20261231"] {
            assert!(is_date(text), "{text}");
        }
        for text in [
            "20260931",
            "20261301",
            "20260230",
            "2026092",
            "2026092A",
            "20260924T",
        ] {
            assert!(!is_date(text), "{text}");
        }
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);

        for text in ["20260924T090000", "20260924T090000Z", "20260924T235960Z"] {
            assert!(is_date_time(text), "{text}");
        }
        for text in [
            "20260924T09000",
            "20260924 090000",
            "20260924T0900000Z",
            "20260924T090000z",
            "20260924T240000",
            "20261324T090000Z",
            "ééééééééT090000",
        ] {
            assert!(!is_date_time(text), "{text}");
        }

        for text in [
            "PT1H30M",
            "P1D",
            "-PT15M",
            "PT9H",
            "P2W",
            "PT0S",
            "+PT1H",
            "PT1H30M45S",
        ] {
            assert!(is_duration(text), "{text}");
        }
        for text in [
            "P",
            "PT",
            "P1Y",
            "P1D2H",
            "PT30M1H",
            "1H",
            "PT1H30M45S60S",
            "P1W2D",
            "PT1S2S",
            "PT",
            "P-1D",
        ] {
            assert!(!is_duration(text), "{text}");
        }

        assert!(is_period("20260924T090000Z/20260924T100000Z"));
        assert!(is_period("20260924T090000Z/PT1H"));
        assert!(!is_period("20260924T090000Z"));
        assert!(!is_period("20260924/20260925"));
        assert!(!is_period("PT1H/20260924T100000Z"));
    }

    #[test]
    fn value_type_follows_the_name_and_the_value_param() {
        assert_eq!(value_type_for("DTSTART", None), ValueType::DateTime);
        assert_eq!(value_type_for("DTSTART", Some("DATE")), ValueType::Date);
        assert_eq!(value_type_for("dtstart", Some("date")), ValueType::Date);
        assert_eq!(value_type_for("SUMMARY", None), ValueType::Text);
        assert_eq!(value_type_for("X-WR-CALNAME", None), ValueType::Text);
        assert_eq!(
            value_type_for("TRIGGER", None),
            ValueType::DurationOrDateTime
        );
        assert_eq!(value_type_for("FREEBUSY", None), ValueType::Period);
        assert_eq!(value_type_for("URL", None), ValueType::Uri);
        assert_eq!(value_type_for("DURATION", Some("TEXT")), ValueType::Text);
        assert_eq!(value_type_for("X-THINKPAD", None), ValueType::Text);

        // A DATE is not a DATE-TIME: the shape is checked, not guessed.
        assert_eq!(check_value("20260925", ValueType::DateTime), None);
        assert_eq!(check_value("20260925", ValueType::Date), Some("date-value"));
        assert_eq!(
            check_value("PT1H", ValueType::Duration),
            Some("duration-value")
        );
        assert_eq!(check_value("PT1H", ValueType::DateTime), None);
        assert_eq!(
            check_value("20260924T090000Z/PT1H", ValueType::Period),
            Some("period-value")
        );
        assert_eq!(check_value("nonsense", ValueType::Period), None);
        assert_eq!(
            check_value("-PT15M", ValueType::DurationOrDateTime),
            Some("duration-value")
        );
        assert_eq!(
            check_value("20260924T090000Z", ValueType::DurationOrDateTime),
            Some("date-time-value")
        );
        assert_eq!(check_value("anything", ValueType::Text), Some("text-value"));
        assert_eq!(
            check_value("FREQ=WEEKLY;INTERVAL=2", ValueType::Recurrence),
            Some("recurrence-value")
        );
    }

    #[test]
    fn comma_lists_are_validated_item_by_item() {
        // RDATE and EXDATE are date-time lists; a single DTSTART is not.
        assert_eq!(
            check_value("20260924T090000Z,20260925T090000Z", ValueType::DateTime),
            Some("date-time-value")
        );
        assert_eq!(
            check_value("20260924T090000Z,nonsense", ValueType::DateTime),
            None
        );
        assert_eq!(
            check_value(
                "20260924T090000Z/PT1H,20260925T090000Z/PT2H",
                ValueType::Period
            ),
            Some("period-value")
        );
    }

    #[test]
    fn required_properties_come_from_the_component() {
        assert_eq!(required_properties("VEVENT"), ["UID", "DTSTAMP"]);
        assert_eq!(required_properties("vevent"), ["UID", "DTSTAMP"]);
        assert_eq!(required_properties("VALARM"), ["ACTION", "TRIGGER"]);
        assert_eq!(required_properties("STANDARD").len(), 4);
        assert!(required_properties("VTIMEZONE").contains(&"TZID"));
        assert!(required_properties("X-CUSTOM").is_empty());
        assert!(property_matches("UID", Span::new(0, 3), "uid"));
        assert!(!property_matches("UID", Span::new(0, 3), "dtstamp"));
    }
}
