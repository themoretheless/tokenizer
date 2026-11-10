//! Lossless lexical scanner and bracket-balance verifier for profiled languages.

use std::borrow::Cow;

use crate::{
    Diagnostic, HostDiagnostic, HostSpan, HostToken, HostTokenization, Span,
    scan::match_common_operator, verify_lossless_spans,
};

use super::{
    lookup::{KeywordLookup, classify_linear},
    profile::FullProfile,
};

/// Exact syntax kinds for the shared full pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    Whitespace,
    LineComment,
    BlockComment,
    Keyword,
    TypeIdent,
    Identifier,
    StringLit,
    NumberLit,
    Punctuation,
    Operator,
    Error,
}

impl SyntaxKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::LineComment => "comment",
            Self::BlockComment => "comment",
            Self::Keyword => "keyword",
            Self::TypeIdent => "type",
            Self::Identifier => "identifier",
            Self::StringLit => "string",
            Self::NumberLit => "number",
            Self::Punctuation => "punctuation",
            Self::Operator => "operator",
            Self::Error => "error",
        }
    }

    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexToken {
    pub kind: SyntaxKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lexed {
    pub tokens: Vec<LexToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    #[must_use]
    pub fn is_lossless(&self, source: &str) -> bool {
        verify_lossless_spans(source, self.tokens.iter().map(|t| t.span)).is_ok()
    }
}

#[must_use]
pub fn lex_full(source: &str, profile: &FullProfile) -> Lexed {
    if source.len() >= 96 {
        let lookup = KeywordLookup::from_profile(profile);
        lex_full_with_lookup(source, profile, Some(&lookup))
    } else {
        lex_full_with_lookup(source, profile, None)
    }
}

#[must_use]
pub fn lex_full_with_lookup(
    source: &str,
    profile: &FullProfile,
    lookup: Option<&KeywordLookup>,
) -> Lexed {
    let bytes = source.as_bytes();
    let mut out = Lexed {
        tokens: Vec::with_capacity(bytes.len() / 4),
        diagnostics: Vec::new(),
    };
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::Whitespace, start, i);
            continue;
        }
        if let Some(m) = profile.line_comment {
            let mb = m.as_bytes();
            if bytes[i..].starts_with(mb) {
                let start = i;
                i += mb.len();
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                push_lex(&mut out, SyntaxKind::LineComment, start, i);
                continue;
            }
        }
        if profile.hash_line_comment && b == b'#' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::LineComment, start, i);
            continue;
        }
        if let Some((open, close)) = profile.block_comment {
            let o = open.as_bytes();
            let c = close.as_bytes();
            if bytes[i..].starts_with(o) {
                let start = i;
                i += o.len();
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i..].starts_with(c) {
                        i += c.len();
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                push_lex(&mut out, SyntaxKind::BlockComment, start, i);
                if !closed {
                    out.diagnostics.push(Diagnostic::new(
                        Span::new(start, i),
                        "unclosed-block-comment",
                        "Unclosed block comment",
                    ));
                }
                continue;
            }
        }
        if profile.triple_strings
            && (bytes[i..].starts_with(b"'''") || bytes[i..].starts_with(b"\"\"\""))
        {
            let q = if bytes[i] == b'\'' { b"'''" } else { b"\"\"\"" };
            let start = i;
            i += 3;
            let mut closed = false;
            while i + 2 < bytes.len() {
                if bytes[i..].starts_with(q) {
                    i += 3;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if !closed {
                i = bytes.len();
                out.diagnostics.push(Diagnostic::new(
                    Span::new(start, i),
                    "unclosed-string",
                    "Unclosed triple-quoted string",
                ));
            }
            push_lex(&mut out, SyntaxKind::StringLit, start, i);
            continue;
        }
        // Paired backticks shield embedded delimiters in templates, SQL names,
        // and shell commands. Haskell infix names keep their token structure.
        if (profile.dollar_ident || profile.backtick_strings)
            && b == b'`'
            && let Some(close) = find_backtick_end(bytes, i + 1)
        {
            push_lex(&mut out, SyntaxKind::StringLit, i, close);
            i = close;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let q = b;
            let start = i;
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == q {
                    i += 1;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'\n' {
                    break;
                }
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::StringLit, start, i);
            if !closed {
                out.diagnostics.push(Diagnostic::new(
                    Span::new(start, i),
                    "unclosed-string",
                    "Unclosed string",
                ));
            }
            continue;
        }
        if b.is_ascii_digit() || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'.'
                    || ((bytes[i] == b'+' || bytes[i] == b'-')
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P')))
            {
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::NumberLit, start, i);
            continue;
        }
        if is_ident_start(b, profile.dollar_ident) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i], profile.dollar_ident) {
                i += 1;
            }
            let text = &source[start..i];
            let kind = match lookup {
                Some(table) => table.classify(profile, text),
                None => classify_linear(profile, text),
            };
            push_lex(&mut out, kind, start, i);
            continue;
        }
        if let Some(len) = match_common_operator(bytes, i) {
            push_lex(&mut out, SyntaxKind::Operator, i, i + len);
            i += len;
            continue;
        }
        if b.is_ascii_punctuation() {
            push_lex(&mut out, SyntaxKind::Punctuation, i, i + 1);
            i += 1;
            continue;
        }
        let start = i;
        i += source[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        push_lex(&mut out, SyntaxKind::Identifier, start, i);
    }
    check_delimiters(source, &mut out);
    out
}

/// Bracket balance over the token stream.
///
/// The one structural claim the shared pipeline can make for every profiled
/// language without modeling its grammar: a `{`, `(` or `[` that is never
/// closed, or a closer with no opener, is broken in all of them. Strings and
/// comments are single tokens by the time this runs, so a bracket character
/// inside one cannot reach here.
fn check_delimiters(source: &str, out: &mut Lexed) {
    // `(group, opener span)`; a mismatched closer drops its opener so one
    // mistake does not cascade into a diagnostic per remaining token.
    let mut stack: Vec<(usize, Span)> = Vec::new();
    let mut found: Vec<Diagnostic> = Vec::new();
    for token in &out.tokens {
        if token.kind != SyntaxKind::Punctuation {
            continue;
        }
        let Some(text) = token.span.slice(source) else {
            continue;
        };
        let Some((group, is_open)) = delimiter_role(text) else {
            continue;
        };
        if is_open {
            stack.push((group, token.span));
            continue;
        }
        if stack.last().is_some_and(|(open, _)| *open == group) {
            stack.pop();
        } else {
            found.push(Diagnostic::new(
                token.span,
                "stray-delimiter",
                "Closing delimiter with no matching opener",
            ));
            stack.pop();
        }
    }
    for (_, span) in stack {
        found.push(Diagnostic::new(
            span,
            "unclosed-delimiter",
            "Delimiter opened here is never closed",
        ));
    }
    found.sort_by_key(|diagnostic| diagnostic.span.start);
    out.diagnostics.extend(found);
}

#[inline]
fn delimiter_role(text: &str) -> Option<(usize, bool)> {
    match text.as_bytes() {
        b"{" => Some((0, true)),
        b"(" => Some((1, true)),
        b"[" => Some((2, true)),
        b"}" => Some((0, false)),
        b")" => Some((1, false)),
        b"]" => Some((2, false)),
        _ => None,
    }
}

fn find_backtick_end(bytes: &[u8], mut i: usize) -> Option<usize> {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

#[inline]
fn push_lex(out: &mut Lexed, kind: SyntaxKind, start: usize, end: usize) {
    if end > start {
        out.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
        });
    }
}

#[inline]
fn is_ident_start(b: u8, dollar: bool) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || (dollar && b == b'$')
}

#[inline]
fn is_ident_continue(b: u8, dollar: bool) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || (dollar && b == b'$')
}

/// Host-facing syntax layer from full lex.
#[must_use]
pub fn lex_to_host(lexed: Lexed) -> HostTokenization {
    let valid = lexed.diagnostics.is_empty();
    HostTokenization {
        tokens: lexed
            .tokens
            .into_iter()
            .map(|t| HostToken {
                kind: Cow::Borrowed(t.kind.as_str()),
                span: HostSpan::from(t.span),
                error: t.kind == SyntaxKind::Error,
            })
            .collect(),
        diagnostics: lexed
            .diagnostics
            .into_iter()
            .map(HostDiagnostic::from_diagnostic)
            .collect(),
        valid,
    }
}
