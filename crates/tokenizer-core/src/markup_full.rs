//! Full markup / data-document engines: XML/HTML-ish tree, CSS rules, YAML/TOML docs, Markdown blocks, SQL stmts.

use crate::{
    Diagnostic, HostDiagnostic, HostSpan, HostToken, HostTokenization, Span,
    langkit::HTML_RAW_TEXT, verify_lossless_spans,
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

pub fn lex_markup(source: &str, flavor: MarkupFlavor) -> MLexed {
    let h = crate::langkit::highlight_markup(source, flavor.is_html());
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

/// Which markup dialect a document follows.
///
/// XML is strict: every element needs a matching close, and names are case
/// sensitive. HTML5 omits end tags (`<li>a<li>b`), has void elements that never
/// take children (`<br>`), and stops parsing inside `<script>`. Handing HTML the
/// XML rules makes it reject valid documents, so the dialect is part of the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkupFlavor {
    Xml,
    Html5,
}

/// Elements that never have children, so `>` ends them.
const HTML_VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements the spec lets an author leave unclosed when the next sibling or the
/// parent's close tag follows.
const HTML_OPTIONAL_END: [&str; 14] = [
    "p", "li", "dd", "dt", "rt", "rp", "option", "optgroup", "thead", "tbody", "tfoot", "tr", "td",
    "th",
];

impl MarkupFlavor {
    const fn is_html(self) -> bool {
        matches!(self, Self::Html5)
    }

    /// Tag and attribute names are case sensitive in XML, ASCII-case insensitive
    /// in HTML.
    fn tag_eq(self, a: &str, b: &str) -> bool {
        if self.is_html() {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    }

    fn in_set(self, set: &[&str], name: &str) -> bool {
        self.is_html() && set.iter().any(|item| self.tag_eq(item, name))
    }
}

/// Strict XML parse: every element needs a matching close tag.
#[must_use]
pub fn parse_markup<'s>(source: &'s str) -> MarkupParse<'s> {
    parse_markup_as(source, MarkupFlavor::Xml)
}

/// Markup parse under a chosen dialect.
#[must_use]
pub fn parse_markup_as<'s>(source: &'s str, flavor: MarkupFlavor) -> MarkupParse<'s> {
    let lexed = lex_markup(source, flavor);
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
            match parse_element(source, &mut i, &mut diags, flavor) {
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

fn skip_tag_body(source: &str, i: &mut usize) {
    let bytes = source.as_bytes();
    while *i < bytes.len() && bytes[*i] != b'>' {
        *i += 1;
    }
    if *i < bytes.len() {
        *i += 1;
    }
}

fn parse_element<'s>(
    source: &'s str,
    i: &mut usize,
    diags: &mut Vec<Diagnostic>,
    flavor: MarkupFlavor,
) -> Option<MarkupNode<'s>> {
    let bytes = source.as_bytes();
    if *i >= bytes.len() || bytes[*i] != b'<' {
        return None;
    }
    let start = *i;
    *i += 1;
    if *i < bytes.len() && (bytes[*i] == b'!' || bytes[*i] == b'?') {
        skip_tag_body(source, i);
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
        // Only the document root reaches this branch: a child loop consumes a
        // close tag before recursing. So a `</x>` with nothing open is either a
        // stray end tag or the ancestor that let an optional end tag slip by
        // never appeared — a parse error in both dialects.
        diags.push(Diagnostic::new(
            Span::new(start, *i),
            "stray-close-tag",
            "Closing tag with no element open",
        ));
        return Some(MarkupNode::Element {
            span: Span::new(start, *i),
            name,
            name_span,
            attrs,
            children: vec![],
            self_closing: true,
        });
    }
    // A void element ends at its `>`, and a raw-text element ends at its close
    // tag whatever the body holds — that is what keeps `if (a < b)` inside a
    // `<script>` from being read as a `< b>` tag.
    if flavor.in_set(&HTML_VOID, name) {
        return Some(MarkupNode::Element {
            span: Span::new(start, *i),
            name,
            name_span,
            attrs,
            children: vec![],
            self_closing: true,
        });
    }
    if flavor.in_set(&HTML_RAW_TEXT, name) {
        let body_start = *i;
        // Shared with the lexer, so tokens and tree end the body at the same byte.
        let children = match crate::langkit::find_close_tag(source, body_start, name) {
            Some(close) => {
                let mut children = Vec::new();
                if close > body_start {
                    children.push(MarkupNode::Text {
                        span: Span::new(body_start, close),
                        text: &source[body_start..close],
                    });
                }
                *i = close;
                skip_tag_body(source, i);
                children
            }
            None => {
                *i = bytes.len();
                diags.push(Diagnostic::new(
                    Span::new(start, *i),
                    "unclosed-element",
                    "Unclosed element",
                ));
                vec![MarkupNode::Text {
                    span: Span::new(body_start, *i),
                    text: &source[body_start..*i],
                }]
            }
        };
        return Some(MarkupNode::Element {
            span: Span::new(start, *i),
            name,
            name_span,
            attrs,
            children,
            self_closing: false,
        });
    }
    let omits_end = flavor.in_set(&HTML_OPTIONAL_END, name);
    let mut children = Vec::new();
    loop {
        if *i >= bytes.len() {
            if !omits_end {
                diags.push(Diagnostic::new(
                    Span::new(start, *i),
                    "unclosed-element",
                    "Unclosed element",
                ));
            }
            break;
        }
        if bytes[*i..].starts_with(b"</") {
            let close_start = *i;
            let cn_start = *i + 2;
            let mut ne = cn_start;
            while ne < bytes.len()
                && (bytes[ne].is_ascii_alphanumeric() || matches!(bytes[ne], b'-' | b':' | b'_'))
            {
                ne += 1;
            }
            let cname = &source[cn_start..ne];
            *i = ne;
            skip_tag_body(source, i);
            if flavor.tag_eq(cname, name) {
                break;
            }
            // An omitted end tag: the close belongs to an ancestor, so stop here
            // without consuming it and let the parent match it.
            if omits_end {
                *i = close_start;
                break;
            }
            // No ancestor match is reachable from here without a stack, so this
            // is the spec's other branch: report the end tag and ignore it,
            // leaving the element open. Breaking instead would let the parent
            // report the author's real close tag as a second mistake.
            if !cname.is_empty() {
                diags.push(Diagnostic::new(
                    Span::new(close_start, *i),
                    "mismatched-tag",
                    "Mismatched closing tag",
                ));
            }
            continue;
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
            // `<li>a<li>b` and `<p>one<p>two` are two siblings, not nesting: an
            // optional end tag is implied by the matching start tag.
            if omits_end && next_tag_is(source, *i, name, flavor) {
                break;
            }
            if let Some(ch) = parse_element(source, i, diags, flavor) {
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

/// Whether `<` at `from` opens a start tag named `name`.
fn next_tag_is(source: &str, from: usize, name: &str, flavor: MarkupFlavor) -> bool {
    let bytes = source.as_bytes();
    if from + 1 >= bytes.len() || bytes[from] != b'<' || bytes[from + 1] == b'/' {
        return false;
    }
    let ns = from + 1;
    let mut ne = ns;
    while ne < bytes.len()
        && (bytes[ne].is_ascii_alphanumeric() || matches!(bytes[ne], b'-' | b':' | b'_'))
    {
        ne += 1;
    }
    flavor.tag_eq(&source[ns..ne], name)
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

    fn codes(source: &str, flavor: MarkupFlavor) -> Vec<&'static str> {
        parse_markup_as(source, flavor)
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect()
    }

    /// The whole point of the dialect: these are documents people write, and
    /// reading them with XML rules produced 12 false positives out of 20 shapes.
    #[test]
    fn html5_rules_accept_what_the_spec_allows() {
        let html = MarkupFlavor::Html5;
        for source in [
            "<html><head><meta charset=\"utf-8\"></head><body>x</body></html>",
            "<div>a<br>b<img src=\"x\" alt=\"y\"><hr></div>",
            "<form><input type=\"text\" name=\"q\"><input type=submit value=Go></form>",
            "<head><link rel=\"stylesheet\" href=\"a.css\"></head>",
            "<picture><source srcset=\"a.webp\" type=\"image/webp\"><img src=\"a.png\" alt=\"\"></picture>",
            "<script>if (a < b) { console.log(1); }</script>",
            "<style>a > b { color: red }</style>",
            "<div><p>one<p>two</div>",
            "<ul><li>a<li>b</ul>",
            "<table><tr><td>1<td>2</td></tr></table>",
            "<title>T</title><p>hi",
            "<select><option>a<option>b</select>",
            "<input disabled required>",
            "<DIV>x</div>",
        ] {
            assert_eq!(
                codes(source, html),
                Vec::<&str>::new(),
                "valid html flagged: {source}"
            );
        }
    }

    /// The tree and the token stream must agree, or the playground highlights a
    /// `< b>` inside a script body even though the parser knows it is text.
    #[test]
    fn the_lexer_reads_a_raw_text_body_as_one_text_run() {
        let source = "<script>if (1 < 2) f();</script>";
        let kinds: Vec<&str> = lex_markup(source, MarkupFlavor::Html5)
            .tokens
            .iter()
            .map(|t| t.kind.as_str())
            .collect();
        assert_eq!(kinds, ["tag", "text", "tag"]);
        assert!(lex_markup(source, MarkupFlavor::Html5).is_lossless(source));
        // XML has no raw-text rule, so the same bytes split at the comparison
        // operator and the rest of the body is swallowed as a tag.
        let xml_kinds: Vec<&str> = lex_markup(source, MarkupFlavor::Xml)
            .tokens
            .iter()
            .map(|t| t.kind.as_str())
            .collect();
        assert_eq!(xml_kinds, ["tag", "text", "tag"]);
    }

    #[test]
    fn the_two_flavors_disagree_where_a_script_body_ends() {
        let html = parse_markup_as("<script>if (1 < 2) f();</script>", MarkupFlavor::Html5);
        assert!(html.diagnostics.is_empty());
        let xml = parse_markup_as("<script>if (1 < 2) f();</script>", MarkupFlavor::Xml);
        assert!(!xml.diagnostics.is_empty(), "the same bytes are not XML");
    }

    #[test]
    fn a_raw_text_body_is_text_until_its_own_close_tag() {
        let parse = parse_markup_as("<script>if (a < b) f();</script>", MarkupFlavor::Html5);
        assert!(parse.diagnostics.is_empty());
        let MarkupNode::Element { name, children, .. } = &parse.doc.roots[0] else {
            panic!("expected an element, got {:?}", parse.doc.roots);
        };
        assert_eq!(*name, "script");
        // One text child holding the whole body, `< b>` included.
        assert_eq!(children.len(), 1);
        let MarkupNode::Text { text, .. } = &children[0] else {
            panic!("expected text, got {:?}", children);
        };
        assert_eq!(*text, "if (a < b) f();");
    }

    /// A void element takes no children, so the following tag belongs to the
    /// parent — the shape that made the XML rules cascade into three errors.
    #[test]
    fn a_void_element_closes_at_its_angle_bracket() {
        let parse = parse_markup_as("<div>a<br>b</div>", MarkupFlavor::Html5);
        assert!(parse.diagnostics.is_empty());
        let MarkupNode::Element { children, .. } = &parse.doc.roots[0] else {
            panic!("expected an element");
        };
        assert_eq!(children.len(), 3, "text, void element, text");
        let MarkupNode::Element {
            name,
            children,
            self_closing,
            ..
        } = &children[1]
        else {
            panic!("expected an element child, got {:?}", children[1]);
        };
        assert_eq!(*name, "br");
        assert!(children.is_empty() && *self_closing);
    }

    #[test]
    fn html5_still_rejects_what_is_actually_broken() {
        let html = MarkupFlavor::Html5;
        assert_eq!(codes("<div>a", html), ["unclosed-element"]);
        assert_eq!(codes("<div", html), ["unclosed-element"]);
        assert_eq!(codes("<script>var a = 1;", html), ["unclosed-element"]);
        assert_eq!(codes("</p>", html), ["stray-close-tag"]);
        // One report for one mistake: the unmatched end tag is ignored rather
        // than silently closing the element, so the author's real `</div>` is
        // not then blamed a second time.
        assert_eq!(codes("<div>a</span></div>", html), ["mismatched-tag"]);
        assert_eq!(
            codes("<b><i>x</b></i>", html),
            ["mismatched-tag", "unclosed-element"]
        );
    }

    #[test]
    fn xml_keeps_nothing_html_allows() {
        let xml = MarkupFlavor::Xml;
        assert_eq!(codes("<open>", xml), ["unclosed-element"]);
        assert_eq!(
            codes("<a></b>", xml),
            ["mismatched-tag", "unclosed-element"]
        );
        assert_eq!(
            codes("<A></a>", xml),
            ["mismatched-tag", "unclosed-element"]
        );
        // Two elements, neither closed: `br` swallows `</div>`, then both hit
        // end of input.
        assert_eq!(
            codes("<div><br></div>", xml),
            ["mismatched-tag", "unclosed-element", "unclosed-element"]
        );
    }
}
