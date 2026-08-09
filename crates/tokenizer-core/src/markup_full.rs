//! Full markup / data-document engines: XML/HTML-ish tree, CSS rules, YAML/TOML docs, Markdown blocks, SQL stmts.

use crate::{
    Diagnostic, HostDiagnostic, HostSpan, HostToken, HostTokenization, Span, verify_lossless_spans,
};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkupTok {
    Whitespace,
    Comment,
    Tag,
    Text,
    String,
    Ident,
    Number,
    Punct,
    Keyword,
    Error,
}

impl MarkupTok {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::Comment => "comment",
            Self::Tag => "tag",
            Self::Text => "text",
            Self::String => "string",
            Self::Ident => "identifier",
            Self::Number => "number",
            Self::Punct => "punctuation",
            Self::Keyword => "keyword",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MToken {
    pub kind: MarkupTok,
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MLexed {
    pub tokens: Vec<MToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl MLexed {
    pub fn is_lossless(&self, source: &str) -> bool {
        verify_lossless_spans(source, self.tokens.iter().map(|t| t.span)).is_ok()
    }
}

// ─── XML / HTML AST ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkupDoc<'s> {
    pub span: Span,
    pub roots: Vec<MarkupNode<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkupNode<'s> {
    Element {
        span: Span,
        name: &'s str,
        name_span: Span,
        attrs: Vec<MarkupAttr<'s>>,
        children: Vec<MarkupNode<'s>>,
        self_closing: bool,
    },
    Text {
        span: Span,
        text: &'s str,
    },
    Comment {
        span: Span,
    },
    Error {
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkupAttr<'s> {
    pub span: Span,
    pub name: &'s str,
    pub value: Option<&'s str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkupParse<'s> {
    pub source: &'s str,
    pub lexed: MLexed,
    pub doc: MarkupDoc<'s>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lex_markup(source: &str) -> MLexed {
    let h = crate::langkit::highlight_markup(source, true);
    MLexed {
        tokens: h
            .tokens
            .into_iter()
            .map(|t| MToken {
                kind: match t.kind {
                    "comment" => MarkupTok::Comment,
                    "tag" => MarkupTok::Tag,
                    "text" => MarkupTok::Text,
                    "whitespace" => MarkupTok::Whitespace,
                    _ => MarkupTok::Text,
                },
                span: t.span,
            })
            .collect(),
        diagnostics: h.diagnostics,
    }
}

pub fn parse_markup<'s>(source: &'s str) -> MarkupParse<'s> {
    let lexed = lex_markup(source);
    let mut diags = lexed.diagnostics.clone();
    let mut roots = Vec::new();
    let mut i = 0usize;
    let bytes = source.as_bytes();
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
                diags.push(Diagnostic::new(
                    Span::new(start, i),
                    "unclosed-comment",
                    "Unclosed comment",
                ));
            }
            roots.push(MarkupNode::Comment {
                span: Span::new(start, i),
            });
            continue;
        }
        if bytes[i] == b'<' {
            match parse_element(source, &mut i, &mut diags) {
                Some(n) => roots.push(n),
                None => {
                    let start = i;
                    i += 1;
                    roots.push(MarkupNode::Error {
                        span: Span::new(start, i),
                    });
                }
            }
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'<' {
            i += 1;
        }
        let text = &source[start..i];
        if !text.is_empty() {
            roots.push(MarkupNode::Text {
                span: Span::new(start, i),
                text,
            });
        }
    }
    MarkupParse {
        source,
        lexed,
        doc: MarkupDoc {
            span: Span::new(0, source.len()),
            roots,
        },
        diagnostics: diags,
    }
}

fn parse_element<'s>(
    source: &'s str,
    i: &mut usize,
    diags: &mut Vec<Diagnostic>,
) -> Option<MarkupNode<'s>> {
    let bytes = source.as_bytes();
    if *i >= bytes.len() || bytes[*i] != b'<' {
        return None;
    }
    let start = *i;
    *i += 1;
    if *i < bytes.len() && (bytes[*i] == b'!' || bytes[*i] == b'?') {
        while *i < bytes.len() && bytes[*i] != b'>' {
            *i += 1;
        }
        if *i < bytes.len() {
            *i += 1;
        }
        return Some(MarkupNode::Element {
            span: Span::new(start, *i),
            name: "!",
            name_span: Span::new(start, *i),
            attrs: vec![],
            children: vec![],
            self_closing: true,
        });
    }
    let closing = *i < bytes.len() && bytes[*i] == b'/';
    if closing {
        *i += 1;
    }
    let name_start = *i;
    while *i < bytes.len()
        && (bytes[*i].is_ascii_alphanumeric() || matches!(bytes[*i], b'-' | b':' | b'_'))
    {
        *i += 1;
    }
    let name = &source[name_start..*i];
    let name_span = Span::new(name_start, *i);
    let mut attrs = Vec::new();
    loop {
        while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i >= bytes.len() {
            break;
        }
        if bytes[*i] == b'>' {
            *i += 1;
            break;
        }
        if bytes[*i] == b'/' && *i + 1 < bytes.len() && bytes[*i + 1] == b'>' {
            *i += 2;
            return Some(MarkupNode::Element {
                span: Span::new(start, *i),
                name,
                name_span,
                attrs,
                children: vec![],
                self_closing: true,
            });
        }
        let an_start = *i;
        while *i < bytes.len()
            && (bytes[*i].is_ascii_alphanumeric() || matches!(bytes[*i], b'-' | b':' | b'_'))
        {
            *i += 1;
        }
        if *i == an_start {
            *i += 1;
            continue;
        }
        let an = &source[an_start..*i];
        while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
            *i += 1;
        }
        let mut val = None;
        if *i < bytes.len() && bytes[*i] == b'=' {
            *i += 1;
            while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
                *i += 1;
            }
            if *i < bytes.len() && (bytes[*i] == b'"' || bytes[*i] == b'\'') {
                let q = bytes[*i];
                *i += 1;
                let vs = *i;
                while *i < bytes.len() && bytes[*i] != q {
                    *i += 1;
                }
                val = Some(&source[vs..*i]);
                if *i < bytes.len() {
                    *i += 1;
                }
            }
        }
        attrs.push(MarkupAttr {
            span: Span::new(an_start, *i),
            name: an,
            value: val,
        });
    }
    if closing {
        return Some(MarkupNode::Element {
            span: Span::new(start, *i),
            name,
            name_span,
            attrs,
            children: vec![],
            self_closing: true,
        });
    }
    let mut children = Vec::new();
    loop {
        if *i >= bytes.len() {
            diags.push(Diagnostic::new(
                Span::new(start, *i),
                "unclosed-element",
                "Unclosed element",
            ));
            break;
        }
        if bytes[*i..].starts_with(b"</") {
            let close_start = *i;
            *i += 2;
            let cn_start = *i;
            while *i < bytes.len()
                && (bytes[*i].is_ascii_alphanumeric() || matches!(bytes[*i], b'-' | b':' | b'_'))
            {
                *i += 1;
            }
            let cname = &source[cn_start..*i];
            while *i < bytes.len() && bytes[*i] != b'>' {
                *i += 1;
            }
            if *i < bytes.len() {
                *i += 1;
            }
            if cname != name && !cname.is_empty() {
                diags.push(Diagnostic::new(
                    Span::new(close_start, *i),
                    "mismatched-tag",
                    "Mismatched closing tag",
                ));
            }
            break;
        }
        if bytes[*i..].starts_with(b"<!--") {
            let cs = *i;
            *i += 4;
            while *i + 2 < bytes.len() && !bytes[*i..].starts_with(b"-->") {
                *i += 1;
            }
            if *i + 2 < bytes.len() {
                *i += 3;
            }
            children.push(MarkupNode::Comment {
                span: Span::new(cs, *i),
            });
            continue;
        }
        if bytes[*i] == b'<' {
            if let Some(ch) = parse_element(source, i, diags) {
                children.push(ch);
            } else {
                *i += 1;
            }
            continue;
        }
        let ts = *i;
        while *i < bytes.len() && bytes[*i] != b'<' {
            *i += 1;
        }
        children.push(MarkupNode::Text {
            span: Span::new(ts, *i),
            text: &source[ts..*i],
        });
    }
    Some(MarkupNode::Element {
        span: Span::new(start, *i),
        name,
        name_span,
        attrs,
        children,
        self_closing: false,
    })
}

pub fn markup_to_host(parse: &MarkupParse<'_>) -> HostTokenization {
    let valid = parse.diagnostics.is_empty();
    HostTokenization {
        tokens: parse
            .lexed
            .tokens
            .iter()
            .map(|t| HostToken {
                kind: Cow::Borrowed(t.kind.as_str()),
                span: HostSpan::from(t.span),
                error: t.kind == MarkupTok::Error,
            })
            .collect(),
        diagnostics: parse
            .diagnostics
            .iter()
            .cloned()
            .map(HostDiagnostic::from_diagnostic)
            .collect(),
        valid,
    }
}

// ─── Document full (YAML/TOML-ish key tree via fullkit) ──────────────────────

/// Re-export fullkit parse for data langs that are expression-heavy.
pub use crate::fullkit::{
    FullProfile, Lexed, Parse, analyze_full_host, lex_full, lex_to_host, parse_full, semantic_full,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_html_element() {
        let p = parse_markup("<div class=\"a\">hi</div>");
        assert!(p.lexed.is_lossless("<div class=\"a\">hi</div>"));
        assert!(!p.doc.roots.is_empty());
    }
}
