//! Shared lossless highlighters for text/programming language plugins.
//!
//! These are **highlight-first** engines (LEX capability): keyword tables,
//! comments, strings, numbers, identifiers. Not full language parsers.

use crate::{
    Diagnostic, HostDiagnostic, HostSpan, HostToken, HostTokenization, Span,
    verify_lossless_spans,
};
use std::borrow::Cow;
use std::collections::HashSet;

/// One highlight run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HlToken {
    pub kind: &'static str,
    pub span: Span,
}

/// Result of a highlight pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Highlighted {
    pub tokens: Vec<HlToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Highlighted {
    #[must_use]
    pub fn is_lossless(&self, source: &str) -> bool {
        verify_lossless_spans(source, self.tokens.iter().map(|t| t.span)).is_ok()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn to_host(self) -> HostTokenization {
        let valid = self.is_valid();
        HostTokenization {
            tokens: self
                .tokens
                .into_iter()
                .map(|t| HostToken {
                    kind: Cow::Borrowed(t.kind),
                    span: HostSpan::from(t.span),
                    error: t.kind == "invalid" || t.kind == "error",
                })
                .collect(),
            diagnostics: self
                .diagnostics
                .into_iter()
                .map(HostDiagnostic::from_diagnostic)
                .collect(),
            valid,
        }
    }
}

/// Profile for C-family / scripting highlighters.
#[derive(Debug, Clone, Copy)]
pub struct CLikeProfile {
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub builtins: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub hash_line_comment: bool,
    pub strings: StringStyle,
    pub ident_continue: IdentStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringStyle {
    /// `"..."` and `'...'` with `\` escapes; optional raw/byte prefixes left to caller.
    CStyle,
    /// Python-like: also `'''` / `"""` triples.
    Python,
    /// Shell: `'...'` literal, `"..."` with `$`/`\` loosely.
    Shell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentStyle {
    Ascii,
    /// Allow `$` in identifiers (PHP, JS optional).
    AsciiDollar,
}

impl Default for CLikeProfile {
    fn default() -> Self {
        Self {
            keywords: &[],
            types: &[],
            builtins: &[],
            line_comment: Some("//"),
            block_comment: Some(("/*", "*/")),
            hash_line_comment: false,
            strings: StringStyle::CStyle,
            ident_continue: IdentStyle::Ascii,
        }
    }
}

/// Lossless C-like highlight of `source`.
#[must_use]
pub fn highlight_c_like(source: &str, profile: &CLikeProfile) -> Highlighted {
    let bytes = source.as_bytes();
    let mut out = Highlighted::default();
    let mut i = 0usize;
    let keywords: HashSet<&str> = profile.keywords.iter().copied().collect();
    let types: HashSet<&str> = profile.types.iter().copied().collect();
    let builtins: HashSet<&str> = profile.builtins.iter().copied().collect();

    while i < bytes.len() {
        let b = bytes[i];

        // Whitespace
        if b.is_ascii_whitespace() {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            push(&mut out, "whitespace", start, i);
            continue;
        }

        // Line comments
        if let Some(marker) = profile.line_comment {
            let m = marker.as_bytes();
            if bytes[i..].starts_with(m) {
                let start = i;
                i += m.len();
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                push(&mut out, "comment", start, i);
                continue;
            }
        }
        if profile.hash_line_comment && b == b'#' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push(&mut out, "comment", start, i);
            continue;
        }

        // Block comments
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
                push(&mut out, "comment", start, i);
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

        // Strings
        match profile.strings {
            StringStyle::Python if bytes[i..].starts_with(b"'''") || bytes[i..].starts_with(b"\"\"\"") =>
            {
                let quote = if bytes[i] == b'\'' { b"'''" } else { b"\"\"\"" };
                let start = i;
                i += 3;
                let mut closed = false;
                while i + 2 < bytes.len() {
                    if bytes[i..].starts_with(quote) {
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
                push(&mut out, "string", start, i);
                continue;
            }
            StringStyle::CStyle | StringStyle::Python | StringStyle::Shell
                if b == b'"' || b == b'\'' =>
            {
                let quote = b;
                let start = i;
                i += 1;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        i += 1;
                        closed = true;
                        break;
                    }
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if profile.strings != StringStyle::Shell && bytes[i] == b'\n' {
                        break;
                    }
                    i += 1;
                }
                push(&mut out, "string", start, i);
                if !closed {
                    out.diagnostics.push(Diagnostic::new(
                        Span::new(start, i),
                        "unclosed-string",
                        "Unclosed string",
                    ));
                }
                continue;
            }
            _ => {}
        }

        // Numbers
        if b.is_ascii_digit()
            || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'.'
                    || ((bytes[i] == b'+' || bytes[i] == b'-')
                        && i > start
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P')))
            {
                i += 1;
            }
            push(&mut out, "number", start, i);
            continue;
        }

        // Identifiers / keywords
        if is_ident_start(b, profile.ident_continue) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i], profile.ident_continue) {
                i += 1;
            }
            let text = &source[start..i];
            let kind = if keywords.contains(text) {
                "keyword"
            } else if types.contains(text) {
                "type"
            } else if builtins.contains(text) {
                "builtin"
            } else {
                "identifier"
            };
            push(&mut out, kind, start, i);
            continue;
        }

        // Operators (multi-char first)
        if let Some(len) = match_operator(bytes, i) {
            push(&mut out, "operator", i, i + len);
            i += len;
            continue;
        }

        // Single punctuation
        if b.is_ascii_punctuation() {
            push(&mut out, "punctuation", i, i + 1);
            i += 1;
            continue;
        }

        // Non-ascii / other: take one char
        let start = i;
        i += source[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        push(&mut out, "identifier", start, i);
    }

    out
}

fn push(out: &mut Highlighted, kind: &'static str, start: usize, end: usize) {
    if end > start {
        out.tokens.push(HlToken {
            kind,
            span: Span::new(start, end),
        });
    }
}

fn is_ident_start(b: u8, style: IdentStyle) -> bool {
    b.is_ascii_alphabetic()
        || b == b'_'
        || (style == IdentStyle::AsciiDollar && b == b'$')
}

fn is_ident_continue(b: u8, style: IdentStyle) -> bool {
    b.is_ascii_alphanumeric()
        || b == b'_'
        || (style == IdentStyle::AsciiDollar && b == b'$')
}

fn match_operator(bytes: &[u8], i: usize) -> Option<usize> {
    let candidates: &[&[u8]] = &[
        b"<<=", b">>=", b"...", b"===", b"!==", b"??=", b"**=", b"<<", b">>", b"<=", b">=", b"==",
        b"!=", b"&&", b"||", b"**", b"+=", b"-=", b"*=", b"/=", b"%=", b"&=", b"|=", b"^=", b"->",
        b"=>", b"::", b"++", b"--", b"??",
    ];
    for op in candidates {
        if bytes[i..].starts_with(op) {
            return Some(op.len());
        }
    }
    None
}

/// Minimal markup highlighter: tags, attrs, text, comments, doctype.
#[must_use]
pub fn highlight_markup(source: &str, htmlish: bool) -> Highlighted {
    let bytes = source.as_bytes();
    let mut out = Highlighted::default();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            let start = i;
            i += 4;
            while i + 2 < bytes.len() && !bytes[i..].starts_with(b"-->") {
                i += 1;
            }
            if i + 2 < bytes.len() {
                i += 3;
            } else {
                i = bytes.len();
                out.diagnostics.push(Diagnostic::new(
                    Span::new(start, i),
                    "unclosed-comment",
                    "Unclosed HTML/XML comment",
                ));
            }
            push(&mut out, "comment", start, i);
            continue;
        }

        if bytes[i] == b'<' {
            let start = i;
            i += 1;
            // doctype / cdata / pi
            if i < bytes.len() && (bytes[i] == b'!' || bytes[i] == b'?') {
                while i < bytes.len() && bytes[i] != b'>' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                push(&mut out, "tag", start, i);
                continue;
            }
            // closing or open tag
            while i < bytes.len() && bytes[i] != b'>' {
                // string attrs
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let q = bytes[i];
                    let s = i;
                    i += 1;
                    while i < bytes.len() && bytes[i] != q {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                    // flush punctuation before handled by splitting later — keep simple: whole tag as tag
                    let _ = s;
                    continue;
                }
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            push(&mut out, "tag", start, i);
            let _ = htmlish;
            continue;
        }

        // text run
        let start = i;
        while i < bytes.len() && bytes[i] != b'<' {
            i += 1;
        }
        push(&mut out, "text", start, i);
    }
    out
}

/// CSS-ish highlighter.
#[must_use]
pub fn highlight_css(source: &str) -> Highlighted {
    let mut profile = CLikeProfile {
        keywords: &[
            "important",
            "from",
            "to",
            "and",
            "or",
            "not",
            "only",
        ],
        types: &[],
        builtins: &[],
        line_comment: None,
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        strings: StringStyle::CStyle,
        ident_continue: IdentStyle::AsciiDollar,
    };
    // CSS uses // rarely; keep block only. Re-use c_like then fix #hex?
    let _ = &mut profile;
    highlight_c_like(source, &profile)
}

/// YAML highlighter (indent-sensitive-ish: comments, keys before :, strings).
#[must_use]
pub fn highlight_yaml(source: &str) -> Highlighted {
    let profile = CLikeProfile {
        keywords: &["true", "false", "null", "yes", "no", "on", "off"],
        types: &[],
        builtins: &[],
        line_comment: None,
        block_comment: None,
        hash_line_comment: true,
        strings: StringStyle::CStyle,
        ident_continue: IdentStyle::Ascii,
    };
    highlight_c_like(source, &profile)
}

/// TOML highlighter.
#[must_use]
pub fn highlight_toml(source: &str) -> Highlighted {
    let profile = CLikeProfile {
        keywords: &["true", "false"],
        types: &[],
        builtins: &[],
        line_comment: None,
        block_comment: None,
        hash_line_comment: true,
        strings: StringStyle::CStyle,
        ident_continue: IdentStyle::Ascii,
    };
    highlight_c_like(source, &profile)
}

/// Markdown: headings, code fences, emphasis-ish, links loosely.
#[must_use]
pub fn highlight_markdown(source: &str) -> Highlighted {
    let bytes = source.as_bytes();
    let mut out = Highlighted::default();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            push(&mut out, "whitespace", i, i + 1);
            i += 1;
            continue;
        }
        // code fence
        if bytes[i..].starts_with(b"```") || bytes[i..].starts_with(b"~~~") {
            let fence = if bytes[i] == b'`' { b"```" } else { b"~~~" };
            let start = i;
            i += 3;
            while i + 2 < bytes.len() && !bytes[i..].starts_with(fence) {
                i += 1;
            }
            if i + 2 < bytes.len() {
                i += 3;
            } else {
                i = bytes.len();
            }
            push(&mut out, "string", start, i);
            continue;
        }
        // heading
        if (i == 0 || bytes[i - 1] == b'\n') && bytes[i] == b'#' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push(&mut out, "keyword", start, i);
            continue;
        }
        // inline code
        if bytes[i] == b'`' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            push(&mut out, "string", start, i);
            continue;
        }
        // whitespace run
        if bytes[i].is_ascii_whitespace() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() && bytes[i] != b'\n' {
                i += 1;
            }
            push(&mut out, "whitespace", start, i);
            continue;
        }
        // word / punct
        if bytes[i].is_ascii_alphanumeric() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
            {
                i += 1;
            }
            push(&mut out, "text", start, i);
            continue;
        }
        push(&mut out, "punctuation", i, i + 1);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_like_lossless_keywords() {
        let profile = CLikeProfile {
            keywords: &["if", "else", "return"],
            ..CLikeProfile::default()
        };
        let source = "if (x) { return 1; } // hi\n";
        let h = highlight_c_like(source, &profile);
        assert!(h.is_lossless(source));
        assert!(h.tokens.iter().any(|t| t.kind == "keyword"));
        assert!(h.tokens.iter().any(|t| t.kind == "comment"));
    }

    #[test]
    fn markup_comment() {
        let source = "<!-- a --><b>x</b>";
        let h = highlight_markup(source, true);
        assert!(h.is_lossless(source));
        assert!(h.tokens.iter().any(|t| t.kind == "comment"));
        assert!(h.tokens.iter().any(|t| t.kind == "tag"));
    }
}
