//! Forma lexical scanner with UTF-16 offset tracking and comment preservation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub message: String,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for Error {}
pub(crate) type R<T> = Result<T, Error>;
#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub start: usize,
    pub end: usize,
}
pub(super) fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
pub(super) fn string_end(s: &str, start: usize, depth: usize) -> R<usize> {
    if depth > 32 {
        return Err(Error {
            offset: start,
            message: "Превышен предел глубины разметки".into(),
        });
    }
    let quote = s.as_bytes()[start];
    let mut i = start + 1;
    while i < s.len() {
        let b = s.as_bytes()[i];
        if b == b'\\' {
            i += 1;
            if i < s.len() {
                i += s[i..].chars().next().unwrap().len_utf8()
            }
            continue;
        }
        if b == quote {
            return Ok(i + 1);
        }
        if s[i..].starts_with("${") {
            i += 2;
            let mut nesting = 1;
            while i < s.len() && nesting > 0 {
                let c = s.as_bytes()[i];
                if c == b'\'' || c == b'"' {
                    i = string_end(s, i, depth + 1)?;
                    continue;
                }
                if c == b'{' {
                    nesting += 1
                }
                if c == b'}' {
                    nesting -= 1
                }
                i += s[i..].chars().next().unwrap().len_utf8();
            }
            if nesting != 0 {
                return Err(Error {
                    offset: start,
                    message: "Незавершённая интерполяция ${…}".into(),
                });
            }
            continue;
        }
        i += s[i..].chars().next().unwrap().len_utf8();
    }
    Err(Error {
        offset: start,
        message: "Незавершённая строка".into(),
    })
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct BorrowedToken<'a> {
    pub(crate) text: &'a str,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) byte_start: usize,
    pub(crate) byte_end: usize,
}
/// Trivia comment (`//…` or `/*…*/`) skipped by the parser but preserved for
/// the formatter. `start`/`end` are UTF-16 units, like token offsets;
/// `byte_start`/`byte_end` locate the comment in the source bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Comment<'a> {
    pub(crate) text: &'a str,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) byte_start: usize,
    pub(crate) byte_end: usize,
}
pub(crate) fn tokenize_borrowed(
    source: &str,
) -> R<(Vec<BorrowedToken<'_>>, Vec<Comment<'_>>, usize)> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(Error {
            offset: 0,
            message: "Превышен предел размера исходника".into(),
        });
    }
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(source.len() / 5);
    let mut comments = Vec::new();
    let mut i = 0;
    let mut utf16 = 0;
    while i < bytes.len() {
        let ws_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'\x0b') {
            i += 1;
        }
        if i > ws_start {
            utf16 += i - ws_start;
            if i >= bytes.len() {
                break;
            }
        }
        let from = i;
        let b0 = bytes[i];
        let mut skip = false;
        let mut comment = false;
        if b0.is_ascii_alphabetic() || b0 == b'_' {
            i += 1;
            loop {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if i + 1 < bytes.len()
                    && bytes[i] == b'.'
                    && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
                {
                    i += 1;
                } else {
                    break;
                }
            }
        } else if b0.is_ascii_digit() {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < bytes.len()
                && bytes[i] == b'.'
                && bytes[i + 1] != b'.'
                && bytes[i + 1].is_ascii_digit()
            {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                let mut exp = i + 1;
                if exp < bytes.len() && matches!(bytes[exp], b'+' | b'-') {
                    exp += 1;
                }
                if exp < bytes.len() && bytes[exp].is_ascii_digit() {
                    i = exp;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            for unit in [
                "vmin", "vmax", "turn", "rem", "deg", "rad", "ms", "px", "em", "vw", "vh", "ch",
                "fr", "s", "%", "*",
            ] {
                if source[i..].starts_with(unit) {
                    let next = source[i + unit.len()..].chars().next();
                    if !next.is_some_and(|c| ident(c) || c == '.' || c == '(') {
                        i += unit.len();
                        break;
                    }
                }
            }
        } else if b0 == b'\'' || b0 == b'"' {
            i = string_end(source, i, 0)?;
        } else if b0 == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = source[i..].find('\n').map_or(source.len(), |n| i + n);
            skip = true;
            comment = true;
        } else if b0 == b'/' && bytes.get(i + 1) == Some(&b'*') {
            if let Some(end_offset) = source[i + 2..].find("*/") {
                i += end_offset + 4;
                skip = true;
                comment = true;
            } else {
                i += 1;
            }
        } else if b0 == b'#' {
            let end = source[i + 1..]
                .bytes()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            if (3..=8).contains(&end) && !source[i + 1 + end..].chars().next().is_some_and(ident) {
                i += 1 + end;
            } else {
                i += 1;
            }
        } else {
            let op_len = match b0 {
                b'<' if source[i..].starts_with("<->") => 3,
                b'<' if source[i..].starts_with("<=") => 2,
                b'=' if source[i..].starts_with("===") => 3,
                b'=' if source[i..].starts_with("==") || source[i..].starts_with("=>") => 2,
                b'!' if source[i..].starts_with("!==") => 3,
                b'!' if source[i..].starts_with("!=") => 2,
                b'.' if source[i..].starts_with("..=") => 3,
                b'.' if source[i..].starts_with("..") => 2,
                b'|' if source[i..].starts_with("|>") || source[i..].starts_with("||") => 2,
                b'-' if source[i..].starts_with("->") => 2,
                b'>' if source[i..].starts_with(">=") => 2,
                b'&' if source[i..].starts_with("&&") => 2,
                b'?' if source[i..].starts_with("??") || source[i..].starts_with("?.") => 2,
                _ => 0,
            };
            if op_len > 0 {
                i += op_len;
            } else if matches!(
                b0,
                b'{' | b'}'
                    | b'['
                    | b']'
                    | b'('
                    | b')'
                    | b':'
                    | b';'
                    | b','
                    | b'!'
                    | b'@'
                    | b'#'
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'/'
                    | b'%'
                    | b'<'
                    | b'>'
                    | b'='
                    | b'?'
                    | b'.'
                    | b'|'
            ) {
                i += 1;
            } else {
                let c = source[i..].chars().next().unwrap();
                if c.is_whitespace() {
                    i += c.len_utf8();
                    skip = true;
                } else {
                    return Err(Error {
                        offset: utf16,
                        message: format!(
                            "Строка {}: неожиданный символ «{c}»",
                            source[..i].bytes().filter(|b| *b == b'\n').count() + 1
                        ),
                    });
                }
            }
        }
        let slice = &source[from..i];
        let utf16_len = if slice.is_ascii() {
            slice.len()
        } else {
            slice.encode_utf16().count()
        };
        let end = utf16 + utf16_len;
        if comment {
            comments.push(Comment {
                text: slice,
                start: utf16,
                end,
                byte_start: from,
                byte_end: i,
            });
        } else if !skip {
            out.push(BorrowedToken {
                text: slice,
                start: utf16,
                end,
                byte_start: from,
                byte_end: i,
            });
            if out.len() > 1_000_000 {
                return Err(Error {
                    offset: utf16,
                    message: "Превышен предел числа токенов".into(),
                });
            }
        }
        utf16 = end;
    }
    Ok((out, comments, utf16))
}
pub fn tokenize(source: &str) -> R<Vec<Token>> {
    let (borrowed, _, _) = tokenize_borrowed(source)?;
    Ok(borrowed
        .into_iter()
        .map(|t| Token {
            text: t.text.into(),
            start: t.start,
            end: t.end,
        })
        .collect())
}
/// Significant tokens plus trivia comments (`//…` and `/*…*/`), in source
/// order, all with UTF-16 offsets. The parser ignores trivia; tooling such as
/// the formatter uses it to keep comments in the output.
pub fn tokenize_with_trivia(source: &str) -> R<(Vec<Token>, Vec<Token>)> {
    let (borrowed, comments, _) = tokenize_borrowed(source)?;
    Ok((
        borrowed
            .into_iter()
            .map(|t| Token {
                text: t.text.into(),
                start: t.start,
                end: t.end,
            })
            .collect(),
        comments
            .into_iter()
            .map(|c| Token {
                text: c.text.into(),
                start: c.start,
                end: c.end,
            })
            .collect(),
    ))
}
