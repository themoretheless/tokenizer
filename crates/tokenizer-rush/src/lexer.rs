use themoretheless_tokenizer_core::{Diagnostic, InputLimits, LexToken, Lexed, Span, SyntaxKind};

pub(crate) const KEYWORDS: &[&str] = &[
    "fn", "let", "const", "mut", "return", "yield", "if", "else", "while", "for", "foreach", "in",
    "match", "break", "continue", "and", "or", "not", "true", "false", "null",
    // Reserved until their grammar and semantics are specified.
    "async", "await", "import",
];
const TYPES: &[&str] = &[
    "int", "f64", "str", "bool", "list", "map", "float", "string",
];

pub(crate) fn run(source: &str, limits: InputLimits) -> (Lexed, bool) {
    let mut out = Lexed::default();
    if limits.exceeds_input_bytes(source.len()) {
        let span = Span::new(0, source.len());
        out.tokens.push(LexToken {
            kind: SyntaxKind::Error,
            span,
        });
        if limits.max_diagnostics > 0 {
            out.diagnostics.push(Diagnostic::new(
                span,
                "input-limit",
                "Input byte limit exceeded",
            ));
        }
        return (out, false);
    }

    let mut valid = true;
    let mut i = 0;
    while i < source.len() {
        let start = i;
        let rest = &source[i..];
        let c = rest.chars().next().unwrap();
        let mut error = None;
        let kind;
        if out.tokens.len() >= limits.max_tokens {
            error = Some(("token-limit", "Token limit exceeded"));
            i = source.len();
            kind = SyntaxKind::Error;
        } else if c.is_ascii_whitespace() {
            i += c.len_utf8();
            while i < source.len() && source.as_bytes()[i].is_ascii_whitespace() {
                i += 1;
            }
            kind = SyntaxKind::Whitespace;
        } else if rest.starts_with("//") {
            i += rest.find(['\r', '\n']).unwrap_or(rest.len());
            kind = SyntaxKind::LineComment;
        } else if let Some(comment) = rest.strip_prefix("/*") {
            if let Some(end) = comment.find("*/") {
                i += end + 4;
            } else {
                i = source.len();
                error = Some(("unclosed-comment", "Expected */ to close the comment"));
            }
            kind = SyntaxKind::BlockComment;
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut closed = false;
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if matches!(next, '\n' | '\r') {
                    break;
                }
                i += next.len_utf8();
                if next == c {
                    closed = true;
                    break;
                }
                if next == '\\' && i < source.len() {
                    let escaped = source[i..].chars().next().unwrap();
                    if matches!(escaped, '\n' | '\r') {
                        break;
                    }
                    i += escaped.len_utf8();
                }
            }
            if !closed {
                error = Some((
                    "unclosed-string",
                    "Expected a closing quote before the end of the line",
                ));
            }
            kind = SyntaxKind::StringLit;
        } else if c.is_ascii_digit() {
            i += 1;
            while i < source.len() && source.as_bytes()[i].is_ascii_digit() {
                i += 1;
            }
            if source.as_bytes().get(i) == Some(&b'.')
                && source.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit)
            {
                i += 1;
                while i < source.len() && source.as_bytes()[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if matches!(source.as_bytes().get(i), Some(b'e' | b'E')) {
                i += 1;
                if matches!(source.as_bytes().get(i), Some(b'+' | b'-')) {
                    i += 1;
                }
                let digits = i;
                while i < source.len() && source.as_bytes()[i].is_ascii_digit() {
                    i += 1;
                }
                if digits == i {
                    error = Some(("invalid-number", "Expected exponent digits"));
                }
            }
            if source[i..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
            {
                while i < source.len() {
                    let ch = source[i..].chars().next().unwrap();
                    if !ch.is_alphanumeric() && ch != '_' {
                        break;
                    }
                    i += ch.len_utf8();
                }
                error = Some((
                    "invalid-number",
                    "A number cannot contain an identifier suffix",
                ));
            }
            kind = SyntaxKind::NumberLit;
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            i += c.len_utf8();
            while i < source.len() {
                let ch = source[i..].chars().next().unwrap();
                if !ch.is_alphanumeric() && ch != '_' {
                    break;
                }
                i += ch.len_utf8();
            }
            let text = &source[start..i];
            if text == "$" {
                error = Some(("invalid-name", "Expected a name after $"));
            }
            kind = if KEYWORDS.contains(&text) {
                SyntaxKind::Keyword
            } else if TYPES.contains(&text) {
                SyntaxKind::TypeIdent
            } else {
                SyntaxKind::Identifier
            };
        } else {
            let operator = [
                "->", "=>", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "%=", "**", "&&", "||",
            ]
            .into_iter()
            .find(|op| rest.starts_with(op));
            if let Some(op) = operator {
                i += op.len();
                kind = SyntaxKind::Operator;
            } else {
                i += c.len_utf8();
                kind = if "()[]{}.,:;".contains(c) {
                    SyntaxKind::Punctuation
                } else if "+-*/%=<>!|".contains(c) {
                    SyntaxKind::Operator
                } else {
                    error = Some(("invalid-character", "Unexpected character in Rush source"));
                    SyntaxKind::Error
                };
            }
        }
        if let Some((code, message)) = error {
            valid = false;
            if out.diagnostics.len() < limits.max_diagnostics {
                out.diagnostics
                    .push(Diagnostic::new(Span::new(start, i), code, message));
            }
        }
        out.tokens.push(LexToken {
            kind,
            span: Span::new(start, i),
        });
    }
    (out, valid)
}
