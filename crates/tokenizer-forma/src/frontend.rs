//! Complete Forma component/design frontend. Transport AST preserves Studio's
//! existing JSON schema; offsets use UTF-16 units, as required by CodeMirror.
use serde_json::{Value as V, json};
use std::collections::HashSet;
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
type R<T> = Result<T, Error>;
#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub start: usize,
    pub end: usize,
}
fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
fn string_end(s: &str, start: usize, depth: usize) -> R<usize> {
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
struct BorrowedToken<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}
fn tokenize_borrowed(source: &str) -> R<(Vec<BorrowedToken<'_>>, usize)> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(Error {
            offset: 0,
            message: "Превышен предел размера исходника".into(),
        });
    }
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(source.len() / 5);
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
            for unit in [
                "turn", "rem", "deg", "rad", "ms", "px", "em", "vw", "vh", "ch", "fr", "s", "%",
                "*",
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
        } else if b0 == b'/' && bytes.get(i + 1) == Some(&b'*') {
            if let Some(end_offset) = source[i + 2..].find("*/") {
                i += end_offset + 4;
                skip = true;
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
        if !skip {
            out.push(BorrowedToken {
                text: slice,
                start: utf16,
                end,
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
    Ok((out, utf16))
}
pub fn tokenize(source: &str) -> R<Vec<Token>> {
    let (borrowed, _) = tokenize_borrowed(source)?;
    Ok(borrowed
        .into_iter()
        .map(|t| Token {
            text: t.text.into(),
            start: t.start,
            end: t.end,
        })
        .collect())
}
fn empty_node(kind: &str, start: usize) -> V {
    json!({"type":kind,"start":start,"props":{},"propertyRanges":{},"statementRanges":{},"bindings":{},"events":{},"children":[],"slots":{},"matches":[],"forward":[]})
}
fn object() -> V {
    json!({})
}
fn array() -> V {
    json!([])
}
fn push(v: &mut V, key: &str, item: V) {
    v[key].as_array_mut().unwrap().push(item)
}
fn has(v: &V, key: &str) -> bool {
    v.as_object().is_some_and(|o| o.contains_key(key))
}
fn nonempty(v: &V) -> bool {
    v.as_array().is_some_and(|a| !a.is_empty()) || v.as_object().is_some_and(|o| !o.is_empty())
}
fn name(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(ident)
}
fn path(s: &str) -> bool {
    s.split('.').all(name)
}
const PURE_FUNCTIONS: &[&str] = &[
    "min",
    "max",
    "clamp",
    "abs",
    "round",
    "floor",
    "ceil",
    "pow",
    "sqrt",
    "sign",
    "lerp",
    "remap",
    "avg",
    "len",
    "String",
    "Number",
    "Bool",
    "trim",
    "upper",
    "lower",
    "capitalize",
    "padStart",
    "padEnd",
    "truncate",
    "repeat",
    "contains",
    "includes",
    "indexOf",
    "startsWith",
    "endsWith",
    "replace",
    "slice",
    "split",
    "join",
    "reverse",
    "take",
    "skip",
    "sum",
    "range",
    "formatNumber",
    "sortBy",
    "pluck",
    "count",
    "any",
    "all",
    "alpha",
    "mix",
    "lighten",
    "darken",
    "contrast",
    "invert",
    "saturate",
    "desaturate",
    "unique",
    "find",
    "compact",
    "flatten",
    "chunk",
    "zip",
    "first",
    "last",
    "groupBy",
    "keys",
    "values",
    "entries",
    "fromEntries",
    "merge",
    "pick",
    "omit",
    "isEmpty",
    "coalesce",
    "plural",
    "isValid",
];
struct Parser<'a> {
    source: &'a str,
    tokens: Vec<BorrowedToken<'a>>,
    eof_utf16: usize,
    i: usize,
    depth: usize,
}
impl<'a> Parser<'a> {
    fn new(source: &'a str) -> R<Self> {
        let (tokens, eof_utf16) = tokenize_borrowed(source)?;
        Ok(Self {
            source,
            tokens,
            eof_utf16,
            i: 0,
            depth: 0,
        })
    }
    #[inline(always)]
    fn peek(&self) -> &'a str {
        self.tokens.get(self.i).map_or("", |t| t.text)
    }
    #[inline(always)]
    fn next(&self) -> &'a str {
        self.tokens.get(self.i + 1).map_or("", |t| t.text)
    }
    #[inline(always)]
    fn pos(&self) -> usize {
        self.tokens.get(self.i).map_or(self.eof_utf16, |t| t.start)
    }
    #[inline(always)]
    fn end(&self) -> usize {
        self.tokens
            .get(self.i.wrapping_sub(1))
            .map_or(self.pos(), |t| t.end)
    }
    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        let target_pos = self.pos();
        Err(Error {
            offset: target_pos,
            message: format!(
                "Строка {}: {}",
                self.source
                    .chars()
                    .scan(0, |pos, c| {
                        let before = *pos;
                        *pos += c.len_utf16();
                        Some((before, c))
                    })
                    .take_while(|(pos, _)| *pos < target_pos)
                    .filter(|(_, c)| *c == '\n')
                    .count()
                    + 1,
                msg.into()
            ),
        })
    }
    #[inline]
    fn take(&mut self, expected: &str) -> R<&'a str> {
        let got = self.peek();
        if got.is_empty() || (!expected.is_empty() && got != expected) {
            return self.err(format!(
                "Ожидалось {}, получено {}",
                if expected.is_empty() {
                    "значение"
                } else {
                    expected
                },
                if got.is_empty() {
                    "конец файла"
                } else {
                    got
                }
            ));
        }
        self.i += 1;
        Ok(got)
    }
    fn semi(&mut self) {
        if self.peek() == ";" {
            self.i += 1
        }
    }
    fn identifier(&mut self, label: &str) -> R<&'a str> {
        let s = self.take("")?;
        if !name(s) {
            return self.err(format!("{label}: ожидалось имя, получено {s}"));
        }
        Ok(s)
    }
    fn ty_single(&mut self) -> R<String> {
        let mut s = self.take("")?.to_owned();
        if !path(&s) {
            return self.err("Ожидалось имя типа");
        }
        if self.peek() == "<" {
            self.i += 1;
            let mut inner = self.ty()?;
            while self.peek() == "," {
                self.i += 1;
                let next_ty = self.ty()?;
                inner.push_str(", ");
                inner.push_str(&next_ty);
            }
            self.take(">")?;
            s = format!("{s}<{inner}>");
        }
        if self.peek() == "?" {
            self.i += 1;
            s.push('?')
        }
        Ok(s)
    }
    fn ty(&mut self) -> R<String> {
        let mut s = self.ty_single()?;
        while self.peek() == "|" {
            self.i += 1;
            let rhs = self.ty_single()?;
            s.push_str(" | ");
            s.push_str(&rhs);
        }
        Ok(s)
    }
    fn expression(&mut self, min: u8) -> R<V> {
        self.depth += 1;
        if self.depth > 32 {
            return self.err("Превышен предел глубины разметки");
        }
        let result = self.expression_inner(min);
        self.depth -= 1;
        result
    }
    fn expression_inner(&mut self, min: u8) -> R<V> {
        let mut left = self.primary()?;
        loop {
            let op = self.peek();
            if ["?.", ".", "["].contains(&op) {
                self.i += 1;
                let optional = op == "?.";
                let computed = op == "[" || (optional && self.peek() == "[");
                let prop = if computed {
                    if op != "[" {
                        self.take("[")?;
                    }
                    let p = self.expression(0)?;
                    self.take("]")?;
                    p
                } else {
                    let p = self.take("")?;
                    if !path(p) {
                        return self.err("Ожидалось имя свойства");
                    }
                    json!(p)
                };
                if computed {
                    left = json!({"expression":{"kind":"member","object":left,"property":prop,"optional":optional,"computed":true}})
                } else {
                    let prop_str = prop.as_str().unwrap();
                    if !optional && !prop_str.contains('.') && self.peek() == "(" {
                        if !PURE_FUNCTIONS.contains(&prop_str) {
                            return self.err(format!("Неизвестная чистая функция {prop_str}"));
                        }
                        self.take("(")?;
                        let mut args = vec![left];
                        while self.peek() != ")" {
                            args.push(self.expression(0)?);
                            if self.peek() != ")" {
                                self.take(",")?;
                            }
                        }
                        self.take(")")?;
                        left = json!({"expression":{"kind":"call","name":prop_str,"args":args}});
                    } else {
                        for (index, part) in prop_str.split('.').enumerate() {
                            left = json!({"expression":{"kind":"member","object":left,"property":part,"optional":index==0&&optional,"computed":false}})
                        }
                    }
                }
                continue;
            }
            if op == "|>" && min <= 1 {
                self.i += 1;
                let fn_name = self.take("")?;
                if !PURE_FUNCTIONS.contains(&fn_name) {
                    return self.err(format!("Неизвестная чистая функция {fn_name}"));
                }
                let mut args = vec![left];
                if self.peek() == "(" {
                    self.take("(")?;
                    while self.peek() != ")" {
                        args.push(self.expression(0)?);
                        if self.peek() != ")" {
                            self.take(",")?;
                        }
                    }
                    self.take(")")?;
                }
                left = json!({"expression":{"kind":"call","name":fn_name,"args":args}});
                continue;
            }
            if op == "?" && min == 0 {
                self.i += 1;
                let then = self.expression(0)?;
                self.take(":")?;
                let otherwise = self.expression(0)?;
                left = json!({"expression":{"kind":"conditional","condition":left,"then":then,"else":otherwise}});
                continue;
            }
            let rank = match op {
                "??" => 1,
                "||" => 2,
                "&&" => 3,
                "==" | "!=" | "===" | "!==" => 4,
                "<" | "<=" | ">" | ">=" | ".." | "..=" => 5,
                "+" | "-" => 6,
                "*" | "/" | "%" => 7,
                _ => break,
            };
            if rank < min {
                break;
            }
            self.i += 1;
            left = json!({"expression":{"kind":"binary","operator":op,"left":left,"right":self.expression(rank+1)?}});
        }
        Ok(left)
    }
    fn primary(&mut self) -> R<V> {
        if self.peek() == "match" {
            return self.match_value(false);
        }
        if self.peek().starts_with(|c: char| c.is_ascii_uppercase()) && self.next() == "{" {
            let n = self.node(None, false)?;
            if n["type"] == "Brush"
                && ["children", "bindings", "events"]
                    .iter()
                    .any(|k| nonempty(&n[k]))
            {
                return self.err("Brush содержит только свойства");
            }
            return Ok(n);
        }
        let token = self.take("")?;
        match token {
            "(" => {
                let v = self.expression(0)?;
                self.take(")")?;
                return Ok(v);
            }
            "[" => {
                let mut a = Vec::new();
                while self.peek() != "]" {
                    a.push(self.expression(0)?);
                    if self.peek() == "," {
                        self.i += 1;
                    } else if self.peek() != "]" {
                        self.take(",")?;
                    }
                }
                self.take("]")?;
                return Ok(json!(a));
            }
            "{" => {
                let mut v = object();
                while self.peek() != "}" {
                    let k = self.take("")?;
                    if has(&v, k) {
                        return self.err(format!("Повторное свойство {k}"));
                    }
                    self.take(":")?;
                    v[k] = self.expression(0)?;
                    if self.peek() == "," || self.peek() == ";" {
                        self.i += 1;
                    } else if self.peek() != "}" {
                        self.take(";")?;
                    }
                }
                self.take("}")?;
                return Ok(v);
            }
            "!" | "-" | "+" => {
                if token == "-" && ["", ";", ",", "]", ")", "}"].contains(&self.peek()) {
                    return Ok(json!({"expr":"-"}));
                }
                let a = self.expression(8)?;
                if token == "!"
                    && a["expr"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty() && s.chars().all(|c| ident(c) || c == '.'))
                {
                    return Ok(json!({"expr":format!("!{}",a["expr"].as_str().unwrap())}));
                }
                if let Some(n) = a.as_f64()
                    && token != "!"
                {
                    return Ok(json!(if token == "-" { -n } else { n }));
                }
                if token == "-" && a["expr"].as_str().is_some_and(unit_literal) {
                    return Ok(json!({"expr":format!("-{}",a["expr"].as_str().unwrap())}));
                }
                return Ok(json!({"expression":{"kind":"unary","operator":token,"argument":a}}));
            }
            "true" => return Ok(json!(true)),
            "false" => return Ok(json!(false)),
            "null" => return Ok(V::Null),
            _ => {}
        }
        if token.starts_with(['\'', '"']) {
            return parse_string(token);
        }
        if let Ok(n) = token.parse::<f64>() {
            return Ok(json!(n));
        }
        if self.peek() == "(" {
            self.take("(")?;
            let mut args = Vec::new();
            while self.peek() != ")" {
                args.push(self.expression(0)?);
                if self.peek() != ")" {
                    self.take(",")?;
                }
            }
            self.take(")")?;
            if token == "c" || token.starts_with("design.") {
                return Ok(json!(format!(
                    "{}({})",
                    token,
                    args.iter()
                        .map(serialize_value)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            if !PURE_FUNCTIONS.contains(&token) {
                return self.err(format!("Неизвестная чистая функция {token}"));
            }
            return Ok(json!({"expression":{"kind":"call","name":token,"args":args}}));
        }
        if !token.starts_with(|c: char| ident(c) || c == '#' || c == '*') {
            return self.err(format!("Ожидалось значение, получено {token}"));
        }
        Ok(json!({"expr":token}))
    }
    fn sequence(&mut self) -> R<V> {
        let mut a = vec![self.expression(0)?];
        while self.peek() != ";" {
            a.push(self.expression(0)?)
        }
        Ok(if a.len() == 1 { a.remove(0) } else { json!(a) })
    }
    fn tuple(&mut self, pattern: bool) -> R<V> {
        if self.peek() != "(" {
            return if pattern {
                self.pattern()
            } else {
                self.expression(0)
            };
        }
        self.take("(")?;
        let mut a = vec![if pattern {
            self.pattern()?
        } else {
            self.expression(0)?
        }];
        while self.peek() == "," {
            self.i += 1;
            a.push(if pattern {
                self.pattern()?
            } else {
                self.expression(0)?
            })
        }
        self.take(")")?;
        Ok(if a.len() == 1 { a.remove(0) } else { json!(a) })
    }
    fn single_pattern(&mut self) -> R<V> {
        if self.peek() == "(" {
            return self.tuple(true);
        }
        if ["<", "<=", ">", ">="].contains(&self.peek()) {
            let op = self.take("")?;
            return Ok(json!({"comparison":op,"value":self.expression(0)?}));
        }
        let p = self.expression(0)?;
        if p.is_object()
            && !has(&p, "expr")
            && !(p["expression"]["kind"] == "binary"
                && ["..", "..="].contains(&p["expression"]["operator"].as_str().unwrap_or("")))
        {
            return self.err("Паттерн: литерал, значение enum, диапазон, сравнение, кортеж или _");
        }
        Ok(p)
    }
    fn pattern(&mut self) -> R<V> {
        let first = self.single_pattern()?;
        if self.peek() != "|" {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.peek() == "|" {
            self.i += 1;
            alts.push(self.single_pattern()?);
        }
        Ok(json!({"or": alts}))
    }
    fn is_markup_match(&self) -> bool {
        let mut j = self.i;
        if self.tokens.get(j).map(|t| t.text) != Some("match") {
            return false;
        }
        j += 1;
        while j < self.tokens.len() && self.tokens[j].text != "{" {
            j += 1;
        }
        j += 1;
        let mut paren = 0usize;
        while j < self.tokens.len() {
            let t = self.tokens[j].text;
            if t == "(" {
                paren += 1;
            } else if t == ")" {
                paren = paren.saturating_sub(1);
            } else if paren == 0 && t == "=>" {
                let b1 = self.tokens.get(j + 1).map(|x| x.text).unwrap_or("");
                let b2 = self.tokens.get(j + 2).map(|x| x.text).unwrap_or("");
                if b1.starts_with(|c: char| c.is_ascii_uppercase()) && b2 == "{" {
                    return true;
                }
                if b1 == "{"
                    && (b2.starts_with(|c: char| c.is_ascii_uppercase())
                        || ["if", "for", "let", "match"].contains(&b2))
                {
                    return true;
                }
                return false;
            }
            j += 1;
        }
        false
    }
    fn match_node(&mut self) -> R<V> {
        let start = self.pos();
        self.take("match")?;
        let subject = self.tuple(false)?;
        self.take("{")?;
        let mut branches = Vec::new();
        let mut seen = HashSet::new();
        while self.peek() != "}" {
            let p = self.pattern()?;
            let guard = if self.peek() == "if" {
                self.i += 1;
                Some(self.expression(0)?)
            } else {
                None
            };
            let dedup_key = if let Some(ref g) = guard {
                format!("{} if {}", p, g)
            } else {
                p.to_string()
            };
            if !seen.insert(dedup_key) {
                return self.err("Повторный паттерн match");
            }
            self.take("=>")?;
            let branch_children = if self.peek() == "{" {
                let (c, _) = self.children()?;
                c
            } else {
                json!([self.child()?])
            };
            self.semi();
            let mut b = json!({"pattern": p, "children": branch_children});
            if let Some(g) = guard {
                b["guard"] = g;
            }
            branches.push(b);
        }
        let end = self.pos() + 1;
        self.take("}")?;
        self.semi();
        if branches.is_empty() {
            return self.err("match требует хотя бы одну ветку");
        }
        let mut n = empty_node("Match", start);
        n["match"] = subject;
        n["branches"] = json!(branches);
        n["end"] = json!(end);
        Ok(n)
    }
    fn match_value(&mut self, group: bool) -> R<V> {
        self.take("match")?;
        let subject = self.tuple(false)?;
        self.take("{")?;
        let mut branches = Vec::new();
        let mut seen = HashSet::new();
        while self.peek() != "}" {
            let p = self.pattern()?;
            let guard = if self.peek() == "if" {
                self.i += 1;
                Some(self.expression(0)?)
            } else {
                None
            };
            let dedup_key = if let Some(ref g) = guard {
                format!("{} if {}", p, g)
            } else {
                p.to_string()
            };
            if !seen.insert(dedup_key) {
                return self.err("Повторный паттерн match");
            }
            self.take("=>")?;
            if group {
                let n = self.node(Some("Patch"), false)?;
                if [
                    "children", "matches", "forward", "bindings", "events", "slots",
                ]
                .iter()
                .any(|k| nonempty(&n[k]))
                {
                    return self.err("Групповой match содержит только свойства");
                }
                let mut b = json!({"pattern":p,"props":n["props"],"propertyRanges":n["propertyRanges"],"statementRanges":n["statementRanges"]});
                if let Some(g) = guard {
                    b["guard"] = g;
                }
                branches.push(b);
                self.semi()
            } else {
                let v = self.sequence()?;
                self.take(";")?;
                let mut b = json!({"pattern":p,"value":v});
                if let Some(g) = guard {
                    b["guard"] = g;
                }
                branches.push(b)
            }
        }
        self.take("}")?;
        if branches.is_empty() {
            return self.err("match требует хотя бы одну ветку");
        }
        Ok(json!({"match":subject,"branches":branches}))
    }
    fn children(&mut self) -> R<(V, usize)> {
        self.take("{")?;
        let mut a = Vec::new();
        while self.peek() != "}" {
            a.push(self.child()?)
        }
        let end = self.pos() + 1;
        self.take("}")?;
        Ok((json!(a), end))
    }
    fn child(&mut self) -> R<V> {
        let kind = self.peek();
        if kind == "let" {
            let start = self.pos();
            self.take("let")?;
            let var = self.identifier("let")?;
            let reserved_names = [
                "state", "props", "events", "actions", "base", "design", "viewport", "i18n",
            ];
            if reserved_names.contains(&var) {
                return self.err(format!("let: зарезервированное имя {var}"));
            }
            self.take("=")?;
            let val = self.sequence()?;
            let end = self.pos() + 1;
            self.take(";")?;
            let mut n = empty_node("Let", start);
            n["name"] = json!(var);
            n["value"] = val;
            n["end"] = json!(end);
            return Ok(n);
        }
        if kind == "match" {
            return self.match_node();
        }
        if kind != "if" && kind != "for" {
            return self.node(None, false);
        }
        let kind = self.take("")?;
        let start = self.tokens[self.i - 1].start;
        let mut n = empty_node(if kind == "if" { "If" } else { "For" }, start);
        if kind == "if" {
            n["condition"] = self.expression(0)?;
            let (c, end) = self.children()?;
            n["children"] = c;
            n["end"] = json!(end);
            n["elseChildren"] = array();
            if self.peek() == "else" {
                self.i += 1;
                if self.peek() == "if" {
                    let other = self.child()?;
                    n["end"] = other["end"].clone();
                    n["elseChildren"] = json!([other])
                } else {
                    let (c, end) = self.children()?;
                    n["elseChildren"] = c;
                    n["end"] = json!(end)
                }
            }
        } else {
            let item = self.identifier("for")?;
            let reserved_names = [
                "state", "props", "events", "actions", "base", "design", "viewport", "i18n",
            ];
            if reserved_names.contains(&item) {
                return self.err(format!("for: зарезервированное имя {item}"));
            }
            n["item"] = json!(item);
            if self.peek() == "," {
                self.i += 1;
                let idx = self.identifier("for")?;
                if reserved_names.contains(&idx) || idx == item {
                    return self.err(format!("for: недопустимое имя индекса {idx}"));
                }
                n["index"] = json!(idx);
            }
            self.take("in")?;
            n["collection"] = self.expression(0)?;
            if self.peek() == "where" {
                self.i += 1;
                n["where"] = self.expression(0)?;
            }
            if self.peek() == "key" {
                self.take("key")?;
                n["key"] = self.expression(0)?;
            } else if (n["collection"]["expression"]["kind"] == "binary"
                && ["..", "..="].contains(
                    &n["collection"]["expression"]["operator"]
                        .as_str()
                        .unwrap_or(""),
                ))
                || (n["collection"]["expression"]["kind"] == "call"
                    && n["collection"]["expression"]["name"] == "range")
            {
                n["key"] = json!({"expr":item});
            } else {
                self.take("key")?;
            }
            let (c, end) = self.children()?;
            n["children"] = c;
            n["end"] = json!(end);
            n["emptyChildren"] = array();
            if self.peek() == "empty" {
                self.i += 1;
                let (c, end) = self.children()?;
                n["emptyChildren"] = c;
                n["end"] = json!(end)
            }
        }
        Ok(n)
    }
    fn parse_constraints(&mut self) -> R<V> {
        let mut constraints = object();
        while self.peek() == "#" {
            self.i += 1;
            self.take("[")?;
            let attr = self.take("")?;
            match attr {
                "range" => {
                    self.take("(")?;
                    let min = self.expression(0)?;
                    self.take(",")?;
                    let max = self.expression(0)?;
                    self.take(")")?;
                    constraints["range"] = json!([min, max]);
                }
                "pattern" => {
                    self.take("(")?;
                    let pat = self.expression(0)?;
                    self.take(")")?;
                    constraints["pattern"] = pat;
                }
                _ => return self.err(format!("Неизвестное ограничение #[{attr}]")),
            }
            self.take("]")?;
        }
        Ok(constraints)
    }
    fn declaration(&mut self, n: &mut V) -> R<()> {
        let start = self.pos();
        let constraints = self.parse_constraints()?;
        let required = self.peek() == "required";
        if required {
            self.i += 1
        }
        let kind = self.take("")?;
        if required && kind != "prop" {
            return self.err("required применяется только к prop");
        }
        if nonempty(&constraints) && kind != "prop" {
            return self
                .err("Ограничения #[range] и #[pattern] применяются к prop или полям struct");
        }
        if kind == "use" {
            let file = self.expression(0)?;
            let Some(file_str) = file.as_str().filter(|s| !s.is_empty()) else {
                return self.err("use ожидает непустой путь в кавычках");
            };
            self.take(";")?;
            push(n, "uses", json!(file_str));
            return Ok(());
        }
        if kind == "shortcut" {
            let combo = self.expression(0)?;
            let Some(combo_str) = combo.as_str().filter(|s| !s.is_empty()) else {
                return self.err("shortcut ожидает непустую строку комбинации клавиш");
            };
            self.take("->")?;
            let action = self.take("")?;
            if !path(action)
                || !["actions.", "events.", "state."]
                    .iter()
                    .any(|p| action.starts_with(p))
            {
                return self.err("Обработчик требует путь actions.…, events.… или state.…");
            }
            self.take("(")?;
            let mut args = Vec::new();
            while self.peek() != ")" {
                args.push(self.expression(0)?);
                if self.peek() != ")" {
                    self.take(",")?;
                }
            }
            self.take(")")?;
            let end = self.pos() + 1;
            self.take(";")?;
            push(
                n,
                "shortcuts",
                json!({"key":combo_str,"action":action,"args":args,"from":start,"to":end}),
            );
            return Ok(());
        }
        if kind == "tokens" || kind == "i18n" {
            let dict_key = if kind == "i18n" { "i18n" } else { "tokens" };
            let group = if self.peek() != "{" {
                Some(self.identifier(kind)?.to_string())
            } else {
                None
            };
            self.take("{")?;
            while self.peek() != "}" {
                let k = self.take("")?;
                if !path(k) {
                    return self.err(format!("{kind}: ожидалось имя ключа, получено {k}"));
                }
                let full_key = match &group {
                    Some(g) => format!("{g}.{k}"),
                    None => k.to_string(),
                };
                if has(&n[dict_key], &full_key) {
                    return self.err(format!("Повторный ключ {full_key}"));
                }
                self.take(":")?;
                let val = self.sequence()?;
                if let Some(g) = &group {
                    if !has(&n[dict_key], k) {
                        n[dict_key][k] = val.clone();
                    }
                    if !n[dict_key][g].is_object() {
                        n[dict_key][g] = object();
                    }
                    n[dict_key][g][k] = val.clone();
                }
                n[dict_key][&full_key] = val;
                self.take(";")?;
            }
            self.take("}")?;
            self.semi();
            return Ok(());
        }
        if kind == "style" {
            let style_name = self.identifier("style")?;
            if has(&n["styles"], style_name) {
                return self.err(format!("Повторный стиль {style_name}"));
            }
            self.take("{")?;
            let mut props = object();
            while self.peek() != "}" {
                let k = self.take("")?;
                if !path(k) {
                    return self.err(format!("style: ожидалось имя свойства, получено {k}"));
                }
                self.take(":")?;
                props[k] = self.sequence()?;
                self.take(";")?;
            }
            self.take("}")?;
            self.semi();
            n["styles"][style_name] = props;
            return Ok(());
        }
        let name = self.identifier(kind)?;
        if kind == "enum" {
            if has(&n["enums"], name) {
                return self.err(format!("Повторный enum {name}"));
            }
            self.take("{")?;
            let mut a = Vec::new();
            while self.peek() != "}" {
                let v = self.identifier("enum")?;
                if a.contains(&v) {
                    return self.err(format!("Повторный вариант {name}.{v}"));
                }
                a.push(v);
                if self.peek() != "}" {
                    self.take(",")?;
                }
            }
            self.take("}")?;
            self.semi();
            if a.is_empty() {
                return self.err(format!("enum {name} требует варианты"));
            }
            n["enums"][name] = json!(a);
            return Ok(());
        }
        if kind == "struct" {
            if has(&n["structs"], name) || has(&n["enums"], name) {
                return self.err(format!("Повторный тип {name}"));
            }
            self.take("{")?;
            let mut fields = object();
            let mut field_constraints = object();
            while self.peek() != "}" {
                let fc = self.parse_constraints()?;
                let field_name = self.identifier("struct")?;
                if has(&fields, field_name) {
                    return self.err(format!("Повторное поле {name}.{field_name}"));
                }
                self.take(":")?;
                let field_ty = self.ty()?;
                fields[field_name] = json!(field_ty);
                if nonempty(&fc) {
                    field_constraints[field_name] = fc;
                }
                if self.peek() == "," || self.peek() == ";" {
                    self.i += 1;
                } else if self.peek() != "}" {
                    self.take(",")?;
                }
            }
            self.take("}")?;
            self.semi();
            if !nonempty(&fields) {
                return self.err(format!("struct {name} требует поля"));
            }
            n["structs"][name] = fields;
            if nonempty(&field_constraints) {
                n["structConstraints"][name] = field_constraints;
            }
            return Ok(());
        }
        if kind == "slot" {
            let mut presenter = empty_node("ContentPresenter", start);
            presenter["props"]["key"] = json!(name);
            if self.peek() == "(" {
                self.take("(")?;
                let mut slot_args = Vec::new();
                while self.peek() != ")" {
                    let param = self.identifier("slot")?;
                    self.take(":")?;
                    let val = self.expression(0)?;
                    slot_args.push(json!({"name": param, "value": val}));
                    if self.peek() != ")" {
                        self.take(",")?;
                    }
                }
                self.take(")")?;
                presenter["slotArgs"] = json!(slot_args);
            }
            if self.peek() == "{" {
                let (c, end) = self.children()?;
                presenter["children"] = c;
                presenter["end"] = json!(end);
                self.semi();
            } else {
                let end = self.pos() + 1;
                self.take(";")?;
                presenter["end"] = json!(end);
            }
            if n.get("slotDefinitions").is_some() {
                n["slotDefinitions"][name] = presenter.clone();
            }
            push(n, "children", presenter);
            return Ok(());
        }
        if [
            "statementRanges",
            "propDefinitions",
            "stateDefinitions",
            "eventDefinitions",
            "lets",
        ]
        .iter()
        .any(|k| has(&n[k], name))
        {
            return self.err(format!("Повторное свойство или событие {name}"));
        }
        if kind == "state" {
            self.take(":")?;
            let ty = self.ty()?;
            self.take("=")?;
            let def_val = self.sequence()?;
            let end = self.pos() + 1;
            self.take(";")?;
            n["stateDefinitions"][name] = json!({"type": ty, "default": def_val});
            n["statementRanges"][name] = json!({"from": start, "to": end});
            return Ok(());
        }
        if kind == "let" {
            self.take("=")?;
            let from = self.pos();
            let mut val = self.sequence()?;
            let to = self.end();
            let end = self.pos() + 1;
            self.take(";")?;
            n["lets"][name] = val.clone();
            // Also expose as `props.<name>` for expressions that refer to `props.name` or bare `name`:
            rewrite_bare_props_in_place(
                &mut val,
                &n["lets"],
                &n["propDefinitions"],
                &n["stateDefinitions"],
            );
            n["props"][name] = val;
            n["propertyRanges"][name] = json!({"from":from,"to":to});
            n["statementRanges"][name] = json!({"from":start,"to":end});
            return Ok(());
        }
        if kind == "prop" {
            self.take(":")?;
            let ty = self.ty()?;
            let mut def = json!({"type":ty,"required":required});
            if nonempty(&constraints) {
                def["constraints"] = constraints;
            }
            n["propDefinitions"][name] = def;
            if self.peek() == "=" {
                if required {
                    return self.err(format!(
                        "required prop {name} не принимает значение по умолчанию"
                    ));
                }
                self.i += 1;
                let from = self.pos();
                n["props"][name] = self.sequence()?;
                n["propertyRanges"][name] = json!({"from":from,"to":self.end()})
            } else if !required && !ty.ends_with('?') {
                return self.err(format!(
                    "prop {name}: требуется значение по умолчанию или required"
                ));
            } else if ty.ends_with('?') && !required {
                n["props"][name] = V::Null
            }
        } else if kind == "event" {
            self.take("(")?;
            let mut args = Vec::new();
            let mut seen = HashSet::new();
            while self.peek() != ")" {
                let arg = self.identifier("event")?;
                if !seen.insert(arg) {
                    return self.err(format!("Повторный аргумент {arg}"));
                }
                self.take(":")?;
                args.push(json!({"name":arg,"type":self.ty()?}));
                if self.peek() != ")" {
                    self.take(",")?;
                }
            }
            self.take(")")?;
            n["eventDefinitions"][name] = json!(args)
        } else {
            return self.err(format!("Неизвестное объявление {kind}"));
        }
        let end = self.pos() + 1;
        self.take(";")?;
        n["statementRanges"][name] = json!({"from":start,"to":end});
        Ok(())
    }
    fn node(&mut self, implicit: Option<&str>, component: bool) -> R<V> {
        self.depth += 1;
        if self.depth > 32 {
            return self.err("Превышен предел глубины разметки");
        }
        let result = self.node_inner(implicit, component);
        self.depth -= 1;
        result
    }
    fn node_inner(&mut self, implicit: Option<&str>, component: bool) -> R<V> {
        let start = self.pos();
        let ty = match implicit {
            Some(t) => t,
            None => self.take("")?,
        };
        if implicit.is_none() && ["If", "For", "Match"].contains(&ty) {
            return self.err(format!("{ty}: зарезервированный тип"));
        }
        let mut n = empty_node(ty, start);
        if component {
            for k in [
                "propDefinitions",
                "stateDefinitions",
                "eventDefinitions",
                "enums",
                "structs",
                "structConstraints",
                "lets",
                "tokens",
                "i18n",
                "styles",
                "slotDefinitions",
            ] {
                n[k] = object()
            }
            n["shortcuts"] = array();
            n["uses"] = array();
        }
        if self.peek() == ":" {
            self.i += 1;
            n["base"] = json!(self.take("")?)
        }
        self.take("{")?;
        while self.peek() != "}" {
            match self.peek() {
                "#" | "required" | "prop" | "state" | "event" | "enum" | "struct" | "tokens"
                | "i18n" | "style" | "shortcut" | "use"
                    if !matches!(self.next(), ":" | "<->" | "->") =>
                {
                    if !component {
                        return self.err(
                            "Объявления prop, state, event, enum, struct, tokens, i18n, style, use и shortcut разрешены только в component",
                        );
                    }
                    self.declaration(&mut n)?;
                    continue;
                }
                "apply" if !matches!(self.next(), ":" | "<->" | "->") => {
                    self.i += 1;
                    let mut applies = n
                        .get("apply")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    loop {
                        let s_name = self.identifier("apply")?;
                        applies.push(json!(s_name));
                        if self.peek() == "," {
                            self.i += 1;
                        } else {
                            break;
                        }
                    }
                    self.take(";")?;
                    n["apply"] = json!(applies);
                    continue;
                }
                "let" => {
                    if component {
                        self.declaration(&mut n)?;
                    } else {
                        push(&mut n, "children", self.child()?);
                    }
                    continue;
                }
                "slot" if name(self.next()) && self.next() != ":" => {
                    self.declaration(&mut n)?;
                    continue;
                }
                "@" => {
                    self.i += 1;
                    let slot_name = self.identifier("slot")?;
                    let mut slot_params = Vec::new();
                    if self.peek() == "(" {
                        self.take("(")?;
                        while self.peek() != ")" {
                            slot_params.push(self.identifier("slot")?);
                            if self.peek() != ")" {
                                self.take(",")?;
                            }
                        }
                        self.take(")")?;
                    }
                    let (c, _) = self.children()?;
                    let children = c.as_array().unwrap();
                    let mut val = if children.len() == 1 {
                        children[0].clone()
                    } else {
                        let mut frame = empty_node("Frame", start);
                        frame["children"] = c;
                        frame
                    };
                    if !slot_params.is_empty() {
                        val["slotParams"] = json!(slot_params);
                    }
                    if component {
                        n["slots"][slot_name] = val;
                    } else {
                        n["props"][slot_name] = val;
                    }
                    self.semi();
                    continue;
                }
                "match" => {
                    if !component && self.is_markup_match() {
                        push(&mut n, "children", self.match_node()?);
                    } else {
                        push(&mut n, "matches", self.match_value(true)?);
                        self.semi();
                    }
                    continue;
                }
                "forward" => {
                    self.i += 1;
                    self.take("props")?;
                    self.take("{")?;
                    while self.peek() != "}" {
                        let key = self.identifier("forward")?;
                        if n["forward"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|x| x.as_str() == Some(key))
                        {
                            return self.err(format!("Повторный forward {key}"));
                        }
                        push(&mut n, "forward", json!(key));
                        if self.peek() != "}" {
                            self.take(",")?;
                        }
                    }
                    self.take("}")?;
                    self.semi();
                    continue;
                }
                "if" | "for" => {
                    push(&mut n, "children", self.child()?);
                    continue;
                }
                "override" => {
                    self.i += 1;
                    let mut key = self.take("")?;
                    if key.starts_with(['\'', '"']) {
                        key = &key[1..key.len() - 1];
                    }
                    if has(&n["slots"], key) {
                        return self.err(format!("Повторный override {key}"));
                    }
                    n["slots"][key] = if self.peek() == "{" {
                        let v = json!({"propertyOverride":true,"patch":self.node(Some("Patch"),false)?});
                        self.semi();
                        v
                    } else if self.peek() == "from" {
                        self.i += 1;
                        let file = self.expression(0)?;
                        if !file.is_string() {
                            return self.err("override from требует путь в кавычках");
                        }
                        let patch = if self.peek() == "{" {
                            self.node(Some("Patch"), false)?
                        } else {
                            V::Null
                        };
                        if patch.is_null() || self.peek() == ";" {
                            self.take(";")?;
                        }
                        json!({"fileOverride":true,"file":file,"patch":patch})
                    } else {
                        self.take(":")?;
                        let v = self.expression(0)?;
                        self.take(";")?;
                        v
                    };
                    continue;
                }
                _ => {}
            }
            if self.next() == "{" {
                push(&mut n, "children", self.node(None, false)?);
                continue;
            }
            let start = self.pos();
            let key = self.take("")?;
            let op = self.take("")?;
            if has(&n["statementRanges"], key) {
                return self.err(format!("Повторное свойство или событие {key}"));
            }
            match op {
                ":" => {
                    let from = self.pos();
                    n["props"][key] = self.sequence()?;
                    n["propertyRanges"][key] = json!({"from":from,"to":self.end()})
                }
                "<->" => {
                    let p = self.take("")?;
                    if !p.contains('.') || !path(p) {
                        return self.err("Двусторонняя привязка требует путь к полю");
                    }
                    n["bindings"][key] = json!(p)
                }
                "->" => {
                    let a = self.take("")?;
                    if !path(a)
                        || !["actions.", "events.", "state."]
                            .iter()
                            .any(|p| a.starts_with(p))
                    {
                        return self.err("Обработчик требует путь actions.…, events.… или state.…");
                    }
                    self.take("(")?;
                    let mut args = Vec::new();
                    while self.peek() != ")" {
                        args.push(self.expression(0)?);
                        if self.peek() != ")" {
                            self.take(",")?;
                        }
                    }
                    self.take(")")?;
                    n["events"][key] = json!(a);
                    if !args.is_empty() {
                        if !has(&n, "eventArgs") {
                            n["eventArgs"] = object()
                        }
                        n["eventArgs"][key] = json!(args)
                    }
                }
                _ => return self.err(format!("Неизвестный оператор {op}")),
            }
            let end = self.pos() + 1;
            self.take(";")?;
            n["statementRanges"][key] = json!({"from":start,"to":end});
        }
        n["end"] = json!(self.pos() + 1);
        self.take("}")?;
        if component
            && (nonempty(&n["lets"])
                || nonempty(&n["propDefinitions"])
                || nonempty(&n["stateDefinitions"]))
        {
            let lets = std::mem::take(&mut n["lets"]);
            let prop_defs = std::mem::take(&mut n["propDefinitions"]);
            let state_defs = std::mem::take(&mut n["stateDefinitions"]);
            for key in ["children", "matches", "slots", "styles", "i18n", "props"] {
                rewrite_bare_props_in_place(&mut n[key], &lets, &prop_defs, &state_defs);
            }
            n["lets"] = lets;
            n["propDefinitions"] = prop_defs;
            n["stateDefinitions"] = state_defs;
        }
        Ok(n)
    }
    fn design_entry(&mut self, overrides: &mut V, types: &mut V, nodes: &mut V) -> R<()> {
        if !self.peek().starts_with(|c: char| c.is_ascii_uppercase()) || !name(self.peek()) {
            return self.err("В design ожидается тип элемента с явным key");
        }
        let item = self.node(None, false)?;
        let key = item["props"]["key"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error {
                offset: self.pos(),
                message: "Дизайн-key должен быть непустой строкой".into(),
            })?;
        if has(overrides, key) {
            return self.err(format!("Повторный дизайн-key {key}"));
        }
        if ["children", "matches", "forward", "events", "bindings"]
            .iter()
            .any(|k| nonempty(&item[k]))
        {
            return self.err("Дизайн переопределяет только свойства");
        }
        let mut props = item["props"].clone();
        props.as_object_mut().unwrap().remove("key");
        types[key] = item["type"].clone();
        overrides[key] = props;
        nodes.as_array_mut().unwrap().push(item);
        Ok(())
    }
    fn document(&mut self, fragment: bool) -> R<V> {
        let mut d = json!({"name":"Preview","nodes":[],"designs":[],"uses":[],"states":[],"entries":[],"designBody":null,"overrides":{},"overrideTypes":{},"defaults":{},"defaultRanges":{},"base":null,"slots":{},"propDefinitions":{},"stateDefinitions":{},"eventDefinitions":{},"enums":{},"structs":{},"structConstraints":{},"lets":{},"tokens":{},"i18n":{},"styles":{},"slotDefinitions":{},"shortcuts":[],"matches":[],"forward":[]});
        while (self.peek() == "#" && ["use", "design"].contains(&self.next()))
            || (self.peek() == "#"
                && self.tokens.get(self.i + 1).map(|t| t.text) == Some("[")
                && ["use", "design"]
                    .contains(&self.tokens.get(self.i + 2).map(|t| t.text).unwrap_or("")))
            || ["tokens", "i18n", "style", "enum", "struct", "use"].contains(&self.peek())
        {
            if self.peek() == "#" {
                self.i += 1;
                self.take("[")?;
                let attr = self.take("")?;
                if attr != "design" && attr != "use" {
                    return self.err(format!("Неизвестный атрибут #[{attr}]"));
                }
                if self.peek() == "(" {
                    self.i += 1;
                    let arg = self.expression(0)?;
                    self.take(")")?;
                    push(&mut d, if attr == "use" { "uses" } else { "designs" }, arg);
                }
                self.take("]")?;
            } else {
                self.declaration(&mut d)?;
            }
        }
        if fragment {
            push(&mut d, "nodes", self.node(None, false)?)
        } else if self.peek().is_empty()
            && (nonempty(&d["tokens"])
                || nonempty(&d["i18n"])
                || nonempty(&d["styles"])
                || nonempty(&d["enums"])
                || nonempty(&d["structs"])
                || nonempty(&d["uses"]))
        {
            // Shared module file (tokens/i18n/styles/enums/structs only)
        } else if self.peek() == "component" {
            self.i += 1;
            let c = self.node(None, true)?;
            d["name"] = c["type"].clone();
            d["base"] = c["base"].clone();
            for u in c["uses"].as_array().cloned().unwrap_or_default() {
                push(&mut d, "uses", u);
            }
            for (k, v) in [
                ("slots", "slots"),
                ("defaultRanges", "statementRanges"),
                ("nodes", "children"),
                ("defaults", "props"),
                ("propDefinitions", "propDefinitions"),
                ("stateDefinitions", "stateDefinitions"),
                ("eventDefinitions", "eventDefinitions"),
                ("lets", "lets"),
                ("slotDefinitions", "slotDefinitions"),
                ("shortcuts", "shortcuts"),
                ("matches", "matches"),
                ("forward", "forward"),
            ] {
                d[k] = c[v].clone()
            }
            for k in [
                "enums",
                "structs",
                "structConstraints",
                "tokens",
                "i18n",
                "styles",
            ] {
                if let Some(o) = c[k].as_object() {
                    for (key, val) in o {
                        d[k][key] = val.clone();
                    }
                }
            }
            if nonempty(&c["bindings"]) || nonempty(&c["events"]) {
                return self.err(
                    "На уровне component разрешены только значения свойств и объявления event",
                );
            }
        } else if self.peek() == "design" {
            self.i += 1;
            d["name"] = json!(self.take("")?);
            let open = self.pos();
            self.take("{")?;
            let (mut overrides, mut types, mut entries) = (object(), object(), array());
            while self.peek() != "}" {
                if self.peek() != "state" {
                    self.design_entry(&mut overrides, &mut types, &mut entries)?;
                    continue;
                }
                let start = self.pos();
                self.i += 1;
                let literal = self.tokens.get(self.i).copied().ok_or_else(|| Error {
                    offset: self.pos(),
                    message: "Имя состояния ожидает строку в кавычках".into(),
                })?;
                let label = self.expression(0)?;
                if !literal.text.starts_with(['\'', '"']) {
                    return self.err("Имя состояния ожидает строку в кавычках");
                }
                if !label.as_str().is_some_and(|s| !s.is_empty()) {
                    return self.err("Имя состояния должно быть непустым");
                }
                if d["states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["name"] == label)
                {
                    return self.err(format!("Повторное состояние {}", label.as_str().unwrap()));
                }
                self.take("{")?;
                let (mut o, mut t, mut n) = (object(), object(), array());
                while self.peek() != "}" {
                    self.design_entry(&mut o, &mut t, &mut n)?;
                }
                let end = self.pos() + 1;
                self.take("}")?;
                push(
                    &mut d,
                    "states",
                    json!({"name":label,"start":start,"end":end,"nameStart":literal.start,"nameEnd":literal.end,"overrides":o,"overrideTypes":t,"nodes":n}),
                );
            }
            d["overrides"] = overrides;
            d["overrideTypes"] = types;
            d["entries"] = entries;
            d["designBody"] = json!({"from":open,"to":self.pos()});
            self.take("}")?;
        } else {
            return self.err("Ожидается component или design. Старый preview/state больше не поддерживается; используйте design с переопределениями по key.");
        }
        if !self.peek().is_empty() {
            return self.err("Лишний текст после компонента");
        }
        check_keys(&d["nodes"], &mut HashSet::new(), false)?;
        Ok(d)
    }
}
fn rewrite_bare_props_in_place(v: &mut V, lets: &V, props: &V, states: &V) {
    match v {
        V::Object(o) => {
            if let Some(V::String(s)) = o.get_mut("expr") {
                let neg = s.starts_with('!');
                let raw = if neg { &s[1..] } else { s.as_str() };
                let head = raw.split('.').next().unwrap_or("");
                if has(states, head) {
                    let prefix = if neg { "!state." } else { "state." };
                    *s = format!("{prefix}{raw}");
                } else if has(lets, head) || has(props, head) {
                    let prefix = if neg { "!props." } else { "props." };
                    *s = format!("{prefix}{raw}");
                }
                return;
            }
            for val in o.values_mut() {
                rewrite_bare_props_in_place(val, lets, props, states);
            }
        }
        V::Array(a) => {
            for x in a {
                rewrite_bare_props_in_place(x, lets, props, states);
            }
        }
        _ => {}
    }
}
fn check_keys(nodes: &V, keys: &mut HashSet<String>, in_for: bool) -> R<()> {
    if let Some(nodes) = nodes.as_array() {
        for n in nodes {
            if has(&n["props"], "key") {
                if let Some(key) = n["props"]["key"].as_str() {
                    if key.is_empty() {
                        return Err(Error {
                            offset: 0,
                            message: "key должен быть непустой строкой".into(),
                        });
                    }
                    if !keys.insert(key.into()) {
                        return Err(Error {
                            offset: 0,
                            message: format!("Повторный key {key}"),
                        });
                    }
                } else if !in_for
                    || (!n["props"]["key"].get("expr").is_some()
                        && !n["props"]["key"].get("interpolation").is_some()
                        && !n["props"]["key"].get("expression").is_some())
                {
                    return Err(Error {
                        offset: 0,
                        message: "key должен быть непустой строкой".into(),
                    });
                }
            }
            if n["type"] == "For" {
                check_keys(&n["children"], &mut HashSet::new(), true)?
            } else if n["type"] == "Match" {
                if let Some(branches) = n["branches"].as_array() {
                    for b in branches {
                        check_keys(&b["children"], keys, in_for)?;
                    }
                }
            } else {
                check_keys(&n["children"], keys, in_for)?
            }
            check_keys(&n["elseChildren"], keys, in_for)?;
            check_keys(&n["emptyChildren"], keys, in_for)?;
        }
    }
    Ok(())
}
pub fn parse(source: &str) -> R<V> {
    parse_with_options(source, false)
}
pub fn parse_with_options(source: &str, fragment: bool) -> R<V> {
    Parser::new(source)?.document(fragment)
}
pub fn parse_expression(source: &str) -> R<V> {
    let mut p = Parser::new(source)?;
    let v = p.expression(0)?;
    if !p.peek().is_empty() {
        return p.err(format!("Лишний текст в выражении: {}", p.peek()));
    }
    Ok(v)
}
fn decode(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                match n {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '\'' | '"' | '\\' | '$' => out.push(n),
                    _ => {
                        out.push('\\');
                        out.push(n)
                    }
                }
            } else {
                out.push(c)
            }
        } else {
            out.push(c)
        }
    }
    out
}
fn parse_string(s: &str) -> R<V> {
    let body = &s[1..s.len() - 1];
    if !body.contains(['\\', '$']) {
        return Ok(json!(body));
    }
    let (mut i, mut start) = (0, 0);
    let mut parts = Vec::new();
    while i < body.len() {
        if body.as_bytes()[i] == b'\\' {
            i += 1;
            if i < body.len() {
                i += body[i..].chars().next().unwrap().len_utf8()
            }
            continue;
        }
        if body[i..].starts_with("${") {
            if i > start {
                parts.push(json!(decode(&body[start..i])))
            }
            let from = i + 2;
            let mut end = from;
            let mut depth = 1;
            while end < body.len() && depth > 0 {
                let b = body.as_bytes()[end];
                if b == b'\'' || b == b'"' {
                    end = string_end(body, end, 0)?;
                    continue;
                }
                if b == b'{' {
                    depth += 1
                }
                if b == b'}' {
                    depth -= 1
                }
                if depth > 0 {
                    end += body[end..].chars().next().unwrap().len_utf8()
                }
            }
            if depth > 0 {
                return Err(Error {
                    offset: 0,
                    message: "Незавершённая интерполяция ${…}".into(),
                });
            }
            parts.push(parse_expression(&body[from..end])?);
            i = end + 1;
            start = i;
        } else {
            i += body[i..].chars().next().unwrap().len_utf8()
        }
    }
    if parts.is_empty() {
        return Ok(json!(decode(body)));
    }
    if start < body.len() {
        parts.push(json!(decode(&body[start..])))
    }
    Ok(json!({"interpolation":parts}))
}
fn unit_literal(s: &str) -> bool {
    [
        "turn", "rem", "deg", "rad", "px", "ms", "em", "vw", "vh", "ch", "fr", "s", "%",
    ]
    .iter()
    .any(|unit| {
        s.strip_suffix(unit)
            .is_some_and(|v| v.parse::<f64>().is_ok())
    })
}
fn quote(s: &str) -> String {
    format!(
        "'{}'",
        s.replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
            .replace("${", "\\${")
    )
}
pub fn serialize_value(v: &V) -> String {
    if let Some(s) = v["expr"].as_str() {
        return s.into();
    }
    if let Some(a) = v["interpolation"].as_array() {
        return format!(
            "'{}'",
            a.iter()
                .map(|v| if let Some(s) = v.as_str() {
                    let q = quote(s);
                    q[1..q.len() - 1].into()
                } else {
                    format!("${{{}}}", serialize_value(v))
                })
                .collect::<String>()
        );
    }
    let e = &v["expression"];
    if let Some(k) = e["kind"].as_str() {
        let sv = |k: &str| serialize_value(&e[k]);
        return match k {
            "binary" => format!(
                "({} {} {})",
                sv("left"),
                e["operator"].as_str().unwrap(),
                sv("right")
            ),
            "unary" => format!("{}({})", e["operator"].as_str().unwrap(), sv("argument")),
            "conditional" => format!("({} ? {} : {})", sv("condition"), sv("then"), sv("else")),
            "call" => format!(
                "{}({})",
                e["name"].as_str().unwrap(),
                e["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(serialize_value)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "member" => {
                let optional = e["optional"] == true;
                let tail = if e["computed"] == false && e["property"].is_string() {
                    format!(
                        "{}{}",
                        if optional { "?." } else { "." },
                        e["property"].as_str().unwrap()
                    )
                } else {
                    format!("{}[{}]", if optional { "?." } else { "" }, sv("property"))
                };
                format!("{}{tail}", sv("object"))
            }
            _ => String::new(),
        };
    }
    if let Some(subject) = v.get("match") {
        let tuple = |v: &V| {
            if let Some(a) = v.as_array() {
                format!(
                    "({})",
                    a.iter().map(serialize_value).collect::<Vec<_>>().join(", ")
                )
            } else {
                serialize_value(v)
            }
        };
        return format!(
            "match {} {{ {} }}",
            tuple(subject),
            v["branches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| format!(
                    "{} => {};",
                    tuple(&b["pattern"]),
                    serialize_value(&b["value"])
                ))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    if let Some(a) = v.as_array() {
        return format!(
            "[{}]",
            a.iter().map(serialize_value).collect::<Vec<_>>().join(", ")
        );
    }
    if let Some(s) = v.as_str() {
        return quote(s);
    }
    if let Some(o) = v.as_object() {
        return format!(
            "{{ {} }}",
            o.iter()
                .map(|(k, v)| format!("{k}: {};", serialize_value(v)))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    if let Some(n) = v.as_f64() {
        return n.to_string();
    }
    v.to_string()
}

pub fn format_source(source: &str) -> R<String> {
    let _ = parse(source)?;
    let (tokens, _) = tokenize_borrowed(source)?;
    let mut out = String::with_capacity(source.len() + 64);
    let mut indent = 0usize;
    let mut at_line_start = true;
    for (idx, tok) in tokens.iter().enumerate() {
        let t = tok.text;
        let next = tokens.get(idx + 1).map(|x| x.text).unwrap_or("");
        if t == "}" {
            indent = indent.saturating_sub(1);
            if !at_line_start {
                out.push('\n');
            }
            for _ in 0..indent {
                out.push_str("  ");
            }
            out.push('}');
            if next != ";" && next != "else" && next != "empty" {
                out.push('\n');
                at_line_start = true;
            } else {
                if next == "else" || next == "empty" {
                    out.push(' ');
                }
                at_line_start = false;
            }
            continue;
        }
        if at_line_start {
            for _ in 0..indent {
                out.push_str("  ");
            }
            at_line_start = false;
        }
        if t == "{" {
            if !out.ends_with(' ') && !out.ends_with("  ") {
                out.push(' ');
            }
            out.push('{');
            out.push('\n');
            indent += 1;
            at_line_start = true;
            continue;
        }
        if t == ";" {
            out.push(';');
            out.push('\n');
            at_line_start = true;
            continue;
        }
        out.push_str(t);
        let no_space_after =
            t == "(" || t == "[" || t == "#[" || t == "@" || t == "!" || t == "." || t == "?.";
        let no_space_before_next = next == ")"
            || next == "]"
            || next == ";"
            || next == ","
            || next == ":"
            || next == "."
            || next == "?."
            || next == "(";
        let force_space_before_paren = matches!(
            t,
            "if" | "for" | "match" | "return" | "where" | "=" | "=>" | "->" | "<->"
        );
        if !no_space_after
            && (!no_space_before_next || (next == "(" && force_space_before_paren))
            && !next.is_empty()
            && next != "{"
        {
            out.push(' ');
        }
    }
    Ok(out.trim_end().to_string() + "\n")
}

pub fn lint_document(source: &str) -> R<V> {
    let doc = parse(source)?;
    let mut warnings = Vec::new();
    let mut used_props: HashSet<&str> = HashSet::new();
    let mut used_states: HashSet<&str> = HashSet::new();
    let mut used_styles: HashSet<&str> = HashSet::new();

    fn collect_all<'a>(
        v: &'a V,
        used_props: &mut HashSet<&'a str>,
        used_states: &mut HashSet<&'a str>,
        used_styles: &mut HashSet<&'a str>,
        collect_style_decls: bool,
    ) {
        if let Some(s) = v["expr"].as_str() {
            let s = s.strip_prefix('!').unwrap_or(s);
            if let Some(rest) = s.strip_prefix("props.") {
                if let Some(head) = rest.split('.').next() {
                    used_props.insert(head);
                }
            } else if let Some(rest) = s.strip_prefix("state.") {
                if let Some(head) = rest.split('.').next() {
                    used_states.insert(head);
                }
            } else if !s.contains('.') {
                used_props.insert(s);
            }
        }
        if let Some(forwards) = v["forward"].as_array() {
            for f in forwards {
                if let Some(name) = f.as_str() {
                    used_props.insert(name);
                }
            }
        }
        if let Some(applies) = v["apply"].as_array() {
            for a in applies {
                if let Some(sn) = a.as_str() {
                    used_styles.insert(sn);
                }
            }
        }
        if let Some(bindings) = v["bindings"].as_object() {
            for bval in bindings.values() {
                if let Some(s) = bval.as_str() {
                    if let Some(rest) = s.strip_prefix("state.") {
                        if let Some(head) = rest.split('.').next() {
                            used_states.insert(head);
                        }
                    } else if let Some(rest) = s.strip_prefix("props.")
                        && let Some(head) = rest.split('.').next()
                    {
                        used_props.insert(head);
                    }
                }
            }
        }
        if let Some(events) = v["events"].as_object() {
            for eval in events.values() {
                if let Some(s) = eval.as_str()
                    && let Some(rest) = s.strip_prefix("state.")
                    && let Some(head) = rest.split('.').next()
                {
                    used_states.insert(head);
                }
            }
        }
        if let Some(action) = v["action"].as_str()
            && let Some(rest) = action.strip_prefix("state.")
            && let Some(head) = rest.split('.').next()
        {
            used_states.insert(head);
        }
        if let Some(a) = v.as_array() {
            for item in a {
                collect_all(
                    item,
                    used_props,
                    used_states,
                    used_styles,
                    collect_style_decls,
                );
            }
        } else if let Some(o) = v.as_object() {
            for (k, val) in o {
                if matches!(
                    k.as_str(),
                    "propDefinitions" | "stateDefinitions" | "defaultRanges" | "statementRanges"
                ) || (!collect_style_decls && k == "styles")
                {
                    continue;
                }
                collect_all(
                    val,
                    used_props,
                    used_states,
                    used_styles,
                    collect_style_decls,
                );
            }
        }
    }

    fn check_matches(v: &V, warnings: &mut Vec<V>) {
        if let Some(branches) = v["branches"].as_array() {
            let mut wildcard_seen = false;
            for b in branches {
                if wildcard_seen {
                    warnings.push(json!({
                        "code": "unreachable-branch",
                        "message": "Недостижимая ветка match после шаблона _"
                    }));
                }
                if b["pattern"]["expr"] == "_" && b.get("guard").is_none() {
                    wildcard_seen = true;
                }
            }
        }
        if let Some(a) = v.as_array() {
            for item in a {
                check_matches(item, warnings);
            }
        } else if let Some(o) = v.as_object() {
            for val in o.values() {
                check_matches(val, warnings);
            }
        }
    }

    for section in ["nodes", "matches", "shortcuts", "styles", "i18n"] {
        collect_all(
            &doc[section],
            &mut used_props,
            &mut used_states,
            &mut used_styles,
            section == "styles",
        );
    }
    if let Some(defaults) = doc["defaults"].as_object() {
        for (k, v) in defaults {
            let is_let = doc["lets"].get(k).is_some();
            if is_let {
                collect_all(
                    v,
                    &mut used_props,
                    &mut used_states,
                    &mut used_styles,
                    false,
                );
            } else {
                let mut ignore_props = HashSet::new();
                collect_all(
                    v,
                    &mut ignore_props,
                    &mut used_states,
                    &mut used_styles,
                    false,
                );
            }
        }
    }

    if let Some(lets) = doc["lets"].as_object() {
        for name in lets.keys() {
            if !used_props.contains(name.as_str()) {
                let range = &doc["defaultRanges"][name];
                warnings.push(json!({
                    "code": "unused-let",
                    "name": name,
                    "from": range["from"],
                    "to": range["to"],
                    "message": format!("Неиспользуемое вычисляемое свойство let {name}")
                }));
            }
        }
    }
    if let Some(prop_defs) = doc["propDefinitions"].as_object() {
        for name in prop_defs.keys() {
            if !used_props.contains(name.as_str()) {
                let range = &doc["defaultRanges"][name];
                warnings.push(json!({
                    "code": "unused-prop",
                    "name": name,
                    "from": range["from"],
                    "to": range["to"],
                    "message": format!("Неиспользуемое свойство prop {name}")
                }));
            }
        }
    }
    if let Some(state_defs) = doc["stateDefinitions"].as_object() {
        for name in state_defs.keys() {
            if !used_states.contains(name.as_str()) {
                let range = &doc["defaultRanges"][name];
                warnings.push(json!({
                    "code": "unused-state",
                    "name": name,
                    "from": range["from"],
                    "to": range["to"],
                    "message": format!("Неиспользуемое состояние state {name}")
                }));
            }
        }
    }
    if let Some(styles) = doc["styles"].as_object() {
        for name in styles.keys() {
            if !used_styles.contains(name.as_str()) {
                warnings.push(json!({
                    "code": "unused-style",
                    "name": name,
                    "message": format!("Неиспользуемый стиль style {name}")
                }));
            }
        }
    }

    if let Some(shortcuts) = doc["shortcuts"].as_array() {
        let mut seen_shortcuts = HashSet::with_capacity(shortcuts.len());
        for s in shortcuts {
            if let Some(key) = s["key"].as_str() {
                let norm = key.to_lowercase();
                if !seen_shortcuts.insert(norm) {
                    warnings.push(json!({
                        "code": "duplicate-shortcut",
                        "key": key,
                        "from": s["from"],
                        "to": s["to"],
                        "message": format!("Повторная комбинация клавиш shortcut '{key}'")
                    }));
                }
            }
        }
    }

    check_matches(&doc["nodes"], &mut warnings);
    check_matches(&doc["matches"], &mut warnings);
    check_matches(&doc["defaults"], &mut warnings);

    Ok(json!(warnings))
}
