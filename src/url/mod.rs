//! URL / path highlighting tokens and lightweight validation.
//!
//! Intended for editor and inspector colouring: incomplete URLs still tokenise
//! so half-typed input keeps meaningful spans. Tokens always cover the full
//! source (lossless concat). Optional [`validate`] reports structure issues
//! without refusing to highlight.
//!
//! Class names in [`UrlKind::class_name`] match the Proxima inspector CSS
//! (`u-scheme`, `u-host`, `u-key`, …).

use crate::{Diagnostic, Span};

/// Semantic highlight class for one URL fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UrlKind {
    Scheme,
    Sep,
    User,
    Host,
    Port,
    Path,
    Key,
    Val,
    Frag,
    Var,
}

impl UrlKind {
    /// CSS / inspector class stem (`u-scheme`, …).
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Scheme => "u-scheme",
            Self::Sep => "u-sep",
            Self::User => "u-user",
            Self::Host => "u-host",
            Self::Port => "u-port",
            Self::Path => "u-path",
            Self::Key => "u-key",
            Self::Val => "u-val",
            Self::Frag => "u-frag",
            Self::Var => "u-var",
        }
    }
}

/// One highlighted run of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UrlToken {
    pub kind: UrlKind,
    pub span: Span,
}

impl UrlToken {
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        self.span.slice(source)
    }
}

/// Full tokenisation of a URL or path string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UrlTokenization {
    pub tokens: Vec<UrlToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl UrlTokenization {
    /// Concatenation of token texts equals `source` when spans are valid.
    #[must_use]
    pub fn is_lossless(&self, source: &str) -> bool {
        let mut rebuilt = String::new();
        for token in &self.tokens {
            let Some(piece) = token.text(source) else {
                return false;
            };
            rebuilt.push_str(piece);
        }
        rebuilt == source
    }
}

/// Tokenize a URL, absolute path, or partial editor string.
///
/// Empty input yields an empty token list. Spans are UTF-8 byte offsets.
/// `{{var}}` placeholders win over every other class.
#[must_use]
pub fn tokenize(source: &str) -> UrlTokenization {
    let mut out = UrlTokenization::default();
    if source.is_empty() {
        return out;
    }

    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;

    // scheme: when present at the start.
    let mut scheme_at: Option<usize> = None;
    if n > 0 && is_scheme_start(bytes[0]) {
        let mut s = 1;
        while s < n && is_scheme_char(bytes[s]) {
            s += 1;
        }
        if s < n && bytes[s] == b':' {
            scheme_at = Some(s);
        }
    }

    if let Some(colon) = scheme_at {
        push_range(&mut out, source, UrlKind::Scheme, 0, colon + 1);
        i = colon + 1;
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            push_range(&mut out, source, UrlKind::Sep, i, i + 2);
            i += 2;
        }
    }

    // Authority: [userinfo@]host[:port] until / ? # or end.
    // When there is no scheme, a leading path-only string starts with `/` and
    // authority is empty (authEnd == i).
    if scheme_at.is_some() || i == 0 {
        let mut auth_end = i;
        while auth_end < n {
            match bytes[auth_end] {
                b'/' | b'?' | b'#' => break,
                _ => auth_end += 1,
            }
        }
        if auth_end > i {
            let at = last_index_of(bytes, b'@', i, auth_end);
            let mut host_start = i;
            if let Some(at) = at {
                push_range(&mut out, source, UrlKind::User, i, at);
                push_range(&mut out, source, UrlKind::Sep, at, at + 1);
                host_start = at + 1;
            }
            let colon = last_index_of(bytes, b':', host_start, auth_end);
            if let Some(colon) = colon {
                let bracket = last_index_of(bytes, b']', host_start, auth_end).unwrap_or(0);
                let mut digits = colon + 1 < auth_end;
                let mut d = colon + 1;
                while digits && d < auth_end {
                    if !bytes[d].is_ascii_digit() {
                        digits = false;
                        break;
                    }
                    d += 1;
                }
                // Empty ":port" (trailing colon) still counts as a port slot.
                if bracket < colon && (colon + 1 == auth_end || digits) {
                    push_range(&mut out, source, UrlKind::Host, host_start, colon);
                    push_range(&mut out, source, UrlKind::Sep, colon, colon + 1);
                    push_range(&mut out, source, UrlKind::Port, colon + 1, auth_end);
                } else {
                    push_range(&mut out, source, UrlKind::Host, host_start, auth_end);
                }
            } else {
                push_range(&mut out, source, UrlKind::Host, host_start, auth_end);
            }
            i = auth_end;
        }
    }

    // Path until ? or #.
    if i < n && bytes[i] != b'?' && bytes[i] != b'#' {
        let mut path_end = i;
        while path_end < n {
            match bytes[path_end] {
                b'?' | b'#' => break,
                _ => path_end += 1,
            }
        }
        let mut p = i;
        while p < path_end {
            if bytes[p] == b'/' {
                push_range(&mut out, source, UrlKind::Sep, p, p + 1);
                p += 1;
                continue;
            }
            let mut seg = p;
            while seg < path_end && bytes[seg] != b'/' {
                seg += 1;
            }
            push_range(&mut out, source, UrlKind::Path, p, seg);
            p = seg;
        }
        i = path_end;
    }

    // Query: ?key=value&key=value
    if i < n && bytes[i] == b'?' {
        push_range(&mut out, source, UrlKind::Sep, i, i + 1);
        i += 1;
        while i < n && bytes[i] != b'#' {
            if bytes[i] == b'&' {
                push_range(&mut out, source, UrlKind::Sep, i, i + 1);
                i += 1;
                continue;
            }
            let mut key_end = i;
            while key_end < n {
                match bytes[key_end] {
                    b'=' | b'&' | b'#' => break,
                    _ => key_end += 1,
                }
            }
            push_range(&mut out, source, UrlKind::Key, i, key_end);
            i = key_end;
            if i < n && bytes[i] == b'=' {
                push_range(&mut out, source, UrlKind::Sep, i, i + 1);
                i += 1;
                let mut val_end = i;
                while val_end < n {
                    match bytes[val_end] {
                        b'&' | b'#' => break,
                        _ => val_end += 1,
                    }
                }
                push_range(&mut out, source, UrlKind::Val, i, val_end);
                i = val_end;
            }
        }
    }

    // Fragment.
    if i < n && bytes[i] == b'#' {
        push_range(&mut out, source, UrlKind::Sep, i, i + 1);
        i += 1;
        push_range(&mut out, source, UrlKind::Frag, i, n);
        i = n;
    }

    // Malformed remainder as path colour.
    if i < n {
        push_range(&mut out, source, UrlKind::Path, i, n);
    }

    out
}

/// Structural validation for a URL-ish string. Does not require a complete
/// URL: empty and partial editor input get mild diagnostics only.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if source.is_empty() {
        return diagnostics;
    }

    if source.chars().any(|c| c.is_whitespace()) {
        diagnostics.push(Diagnostic {
            span: Span::new(0, source.len()),
            code: "url-whitespace",
            message: "URL must not contain whitespace",
        });
    }

    let tokens = tokenize(source);
    let has_scheme = tokens.tokens.iter().any(|t| t.kind == UrlKind::Scheme);
    let has_host = tokens
        .tokens
        .iter()
        .any(|t| t.kind == UrlKind::Host && t.span.len() > 0);
    let has_path = tokens.tokens.iter().any(|t| t.kind == UrlKind::Path);
    let path_only = source.starts_with('/');

    if has_scheme && !has_host && !path_only {
        // scheme without authority (e.g. "https:") mid-edit is not fatal, but
        // flag empty host when // is present.
        if source.contains("://") {
            diagnostics.push(Diagnostic {
                span: Span::new(0, source.len()),
                code: "url-empty-host",
                message: "Scheme is present but host is empty",
            });
        }
    }

    for token in &tokens.tokens {
        if token.kind != UrlKind::Port {
            continue;
        }
        let Some(text) = token.text(source) else {
            continue;
        };
        if text.is_empty() {
            diagnostics.push(Diagnostic {
                span: token.span,
                code: "url-empty-port",
                message: "Port is empty",
            });
            continue;
        }
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            diagnostics.push(Diagnostic {
                span: token.span,
                code: "url-invalid-port",
                message: "Port must be decimal digits",
            });
            continue;
        }
        if let Ok(port) = text.parse::<u32>() {
            if port > 65535 {
                diagnostics.push(Diagnostic {
                    span: token.span,
                    code: "url-port-range",
                    message: "Port must be between 0 and 65535",
                });
            }
        }
    }

    // Absolute URL-ish strings should have a host or be path-only.
    if has_scheme && !has_host && !has_path && !path_only && !source.contains("://") {
        // "mailto:user" style — allow.
    }

    // Unclosed {{var
    if let Some(open) = source.find("{{") {
        if !source[open..].contains("}}") {
            diagnostics.push(Diagnostic {
                span: Span::new(open, source.len()),
                code: "url-unclosed-var",
                message: "Unclosed {{var}} placeholder",
            });
        }
    }

    diagnostics
}

/// Tokenize and attach validation diagnostics (highlight still lossless).
#[must_use]
pub fn tokenize_and_validate(source: &str) -> UrlTokenization {
    let mut result = tokenize(source);
    result.diagnostics.extend(validate(source));
    result
}

fn is_scheme_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_scheme_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-')
}

fn last_index_of(bytes: &[u8], needle: u8, start: usize, end: usize) -> Option<usize> {
    if start >= end || end > bytes.len() {
        return None;
    }
    bytes[start..end]
        .iter()
        .rposition(|&b| b == needle)
        .map(|rel| start + rel)
}

/// Push `kind` for `[from, to)`, splitting out `{{var}}` runs as [`UrlKind::Var`].
fn push_range(out: &mut UrlTokenization, source: &str, kind: UrlKind, from: usize, to: usize) {
    if to <= from || to > source.len() || from > source.len() {
        return;
    }
    if !source.is_char_boundary(from) || !source.is_char_boundary(to) {
        // Fall back to a single token so we never panic on bad indices.
        out.tokens.push(UrlToken {
            kind,
            span: Span::new(from.min(source.len()), to.min(source.len())),
        });
        return;
    }

    let mut j = from;
    let mut start = from;
    while j < to {
        if j + 1 < to && source.as_bytes()[j] == b'{' && source.as_bytes()[j + 1] == b'{' {
            let rest = &source[j..to];
            if let Some(rel) = rest.find("}}") {
                let close = j + rel;
                if j > start {
                    out.tokens.push(UrlToken {
                        kind,
                        span: Span::new(start, j),
                    });
                }
                out.tokens.push(UrlToken {
                    kind: UrlKind::Var,
                    span: Span::new(j, close + 2),
                });
                start = close + 2;
                j = start;
                continue;
            }
            // Unclosed {{... : mark rest as var.
            if j > start {
                out.tokens.push(UrlToken {
                    kind,
                    span: Span::new(start, j),
                });
            }
            out.tokens.push(UrlToken {
                kind: UrlKind::Var,
                span: Span::new(j, to),
            });
            return;
        }
        j += 1;
        // Stay on char boundaries if we ever land mid-sequence (ASCII-only scan
        // above is fine for `{`; general advance for safety).
        while j < to && !source.is_char_boundary(j) {
            j += 1;
        }
    }
    if to > start {
        out.tokens.push(UrlToken {
            kind,
            span: Span::new(start, to),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<(UrlKind, &str)> {
        tokenize(source)
            .tokens
            .into_iter()
            .map(|t| (t.kind, t.text(source).unwrap()))
            .collect()
    }

    #[test]
    fn full_https_url_with_query() {
        let source = "https://api.example.com/v1/weather?q=Moscow&units=metric";
        let got = kinds(source);
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Scheme && *t == "https:"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Host && *t == "api.example.com"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Path && *t == "v1"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Key && *t == "q"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Val && *t == "Moscow"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Key && *t == "units"));
        assert!(tokenize(source).is_lossless(source));
    }

    #[test]
    fn path_only_with_query() {
        let source = "/data/2.5/weather?q=Moscow";
        let got = kinds(source);
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Path && *t == "data"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Key && *t == "q"));
        assert!(tokenize(source).is_lossless(source));
    }

    #[test]
    fn userinfo_and_port() {
        let source = "https://user:pass@host.example:8443/x";
        let got = kinds(source);
        assert!(got.iter().any(|(k, t)| *k == UrlKind::User && t.starts_with("user")));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Host && *t == "host.example"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Port && *t == "8443"));
        assert!(tokenize(source).is_lossless(source));
    }

    #[test]
    fn env_var_placeholder_wins() {
        let source = "https://{{host}}/v1?key={{token}}";
        let got = kinds(source);
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Var && *t == "{{host}}"));
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Var && *t == "{{token}}"));
        assert!(tokenize(source).is_lossless(source));
    }

    #[test]
    fn fragment() {
        let source = "https://ex.test/a#section";
        let got = kinds(source);
        assert!(got.iter().any(|(k, t)| *k == UrlKind::Frag && *t == "section"));
        assert!(tokenize(source).is_lossless(source));
    }

    #[test]
    fn incomplete_scheme_still_tokenises() {
        let source = "https:";
        assert!(tokenize(source).is_lossless(source));
        assert!(kinds(source).iter().any(|(k, _)| *k == UrlKind::Scheme));
    }

    #[test]
    fn validate_flags_whitespace_and_port() {
        let d = validate("https://ex.test:99999/x y");
        assert!(d.iter().any(|x| x.code == "url-whitespace"));
        assert!(d.iter().any(|x| x.code == "url-port-range"));
    }

    #[test]
    fn validate_unclosed_var() {
        let d = validate("https://ex.test/{{open");
        assert!(d.iter().any(|x| x.code == "url-unclosed-var"));
    }

    #[test]
    fn class_names_match_inspector_css() {
        assert_eq!(UrlKind::Scheme.class_name(), "u-scheme");
        assert_eq!(UrlKind::Key.class_name(), "u-key");
        assert_eq!(UrlKind::Val.class_name(), "u-val");
    }
}
