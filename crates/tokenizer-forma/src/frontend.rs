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
pub fn tokenize(source: &str) -> R<Vec<Token>> {
    if source.len() > 4 * 1024 * 1024 {
        return Err(Error {
            offset: 0,
            message: "Превышен предел размера исходника".into(),
        });
    }
    let mut out = Vec::new();
    let mut i = 0;
    let mut utf16 = 0;
    while i < source.len() {
        let from = i;
        let c = source[i..].chars().next().unwrap();
        let mut skip = false;
        if c.is_whitespace() {
            i += c.len_utf8();
            skip = true
        } else if source[i..].starts_with("//") {
            i = source[i..].find('\n').map_or(source.len(), |n| i + n);
            skip = true
        } else if source[i..].starts_with("/*") && source[i + 2..].contains("*/") {
            i += source[i + 2..].find("*/").unwrap() + 4;
            skip = true
        } else if c == '\'' || c == '"' {
            i = string_end(source, i, 0)?
        } else if let Some(op) = [
            "<->", "===", "!==", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.",
        ]
        .iter()
        .find(|op| source[i..].starts_with(**op))
        {
            i += op.len()
        } else if c == '#' {
            let end = source[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            if (3..=8).contains(&end) && !source[i + 1 + end..].chars().next().is_some_and(ident) {
                i += 1 + end
            } else {
                i += 1
            }
        } else if c.is_ascii_digit() {
            i += 1;
            while i < source.len() && source.as_bytes()[i].is_ascii_digit() {
                i += 1
            }
            if i + 1 < source.len()
                && source.as_bytes()[i] == b'.'
                && source.as_bytes()[i + 1].is_ascii_digit()
            {
                i += 1;
                while i < source.len() && source.as_bytes()[i].is_ascii_digit() {
                    i += 1
                }
            }
            for unit in ["ms", "px", "deg", "fr", "%", "*"] {
                if source[i..].starts_with(unit) {
                    let next = source[i + unit.len()..].chars().next();
                    if !["%", "*"].contains(&unit)
                        || !next.is_some_and(|c| ident(c) || c == '.' || c == '(')
                    {
                        i += unit.len();
                        break;
                    }
                }
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            i += 1;
            loop {
                while i < source.len() && ident(source[i..].chars().next().unwrap()) {
                    i += 1
                }
                if i + 1 < source.len()
                    && source.as_bytes()[i] == b'.'
                    && (source.as_bytes()[i + 1].is_ascii_alphabetic()
                        || source.as_bytes()[i + 1] == b'_')
                {
                    i += 1
                } else {
                    break;
                }
            }
        } else if "{}[]():;,!@#*+-/%<>=?.".contains(c) {
            i += c.len_utf8()
        } else {
            return Err(Error {
                offset: utf16,
                message: format!(
                    "Строка {}: неожиданный символ «{c}»",
                    source[..i].bytes().filter(|b| *b == b'\n').count() + 1
                ),
            });
        }
        let end = utf16 + source[from..i].encode_utf16().count();
        if !skip {
            out.push(Token {
                text: source[from..i].into(),
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
    Ok(out)
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
struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    i: usize,
    depth: usize,
}
impl<'a> Parser<'a> {
    fn new(source: &'a str) -> R<Self> {
        Ok(Self {
            source,
            tokens: tokenize(source)?,
            i: 0,
            depth: 0,
        })
    }
    fn peek(&self) -> &str {
        self.tokens.get(self.i).map_or("", |t| t.text.as_str())
    }
    fn next(&self) -> &str {
        self.tokens.get(self.i + 1).map_or("", |t| t.text.as_str())
    }
    fn pos(&self) -> usize {
        self.tokens
            .get(self.i)
            .map_or_else(|| self.source.encode_utf16().count(), |t| t.start)
    }
    fn end(&self) -> usize {
        self.tokens
            .get(self.i.wrapping_sub(1))
            .map_or(self.pos(), |t| t.end)
    }
    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Error {
            offset: self.pos(),
            message: format!(
                "Строка {}: {}",
                self.source
                    .chars()
                    .scan(0, |pos, c| {
                        let before = *pos;
                        *pos += c.len_utf16();
                        Some((before, c))
                    })
                    .take_while(|(pos, _)| *pos < self.pos())
                    .filter(|(_, c)| *c == '\n')
                    .count()
                    + 1,
                msg.into()
            ),
        })
    }
    fn take(&mut self, expected: &str) -> R<String> {
        let got = self.peek().to_string();
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
                    &got
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
    fn identifier(&mut self, label: &str) -> R<String> {
        let s = self.take("")?;
        if !name(&s) {
            return self.err(format!("{label}: ожидалось имя, получено {s}"));
        }
        Ok(s)
    }
    fn ty(&mut self) -> R<String> {
        let mut s = self.take("")?;
        if !path(&s) {
            return self.err("Ожидалось имя типа");
        }
        if self.peek() == "?" {
            self.i += 1;
            s.push('?')
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
            let op = self.peek().to_string();
            if ["?.", ".", "["].contains(&op.as_str()) {
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
                    if !path(&p) {
                        return self.err("Ожидалось имя свойства");
                    }
                    json!(p)
                };
                if computed {
                    left = json!({"expression":{"kind":"member","object":left,"property":prop,"optional":optional,"computed":true}})
                } else {
                    for (index, part) in prop.as_str().unwrap().split('.').enumerate() {
                        left = json!({"expression":{"kind":"member","object":left,"property":part,"optional":index==0&&optional,"computed":false}})
                    }
                }
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
            let rank = match op.as_str() {
                "??" => 1,
                "||" => 2,
                "&&" => 3,
                "==" | "!=" | "===" | "!==" => 4,
                "<" | "<=" | ">" | ">=" => 5,
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
        match token.as_str() {
            "(" => {
                let v = self.expression(0)?;
                self.take(")")?;
                return Ok(v);
            }
            "[" => {
                let mut a = Vec::new();
                while self.peek() != "]" {
                    a.push(self.expression(0)?);
                    if self.peek() != "]" {
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
                    if has(&v, &k) {
                        return self.err(format!("Повторное свойство {k}"));
                    }
                    self.take(":")?;
                    v[&k] = self.expression(0)?;
                    self.take(";")?;
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
            return parse_string(&token);
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
            if ![
                "min", "max", "clamp", "abs", "round", "floor", "ceil", "len", "String", "Number",
                "Bool",
            ]
            .contains(&token.as_str())
            {
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
    fn pattern(&mut self) -> R<V> {
        if self.peek() == "(" {
            return self.tuple(true);
        }
        if ["<", "<=", ">", ">="].contains(&self.peek()) {
            let op = self.take("")?;
            return Ok(json!({"comparison":op,"value":self.expression(0)?}));
        }
        let p = self.expression(0)?;
        if p.is_object() && !has(&p, "expr") {
            return self.err("Паттерн: литерал, значение enum, сравнение, кортеж или _");
        }
        Ok(p)
    }
    fn match_value(&mut self, group: bool) -> R<V> {
        self.take("match")?;
        let subject = self.tuple(false)?;
        self.take("{")?;
        let mut branches = Vec::new();
        let mut seen = HashSet::new();
        while self.peek() != "}" {
            let p = self.pattern()?;
            if !seen.insert(p.to_string()) {
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
                branches.push(json!({"pattern":p,"props":n["props"],"propertyRanges":n["propertyRanges"],"statementRanges":n["statementRanges"]}));
                self.semi()
            } else {
                let v = self.sequence()?;
                self.take(";")?;
                branches.push(json!({"pattern":p,"value":v}))
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
            if [
                "state", "props", "events", "actions", "base", "design", "viewport",
            ]
            .contains(&item.as_str())
            {
                return self.err(format!("for: зарезервированное имя {item}"));
            }
            n["item"] = json!(item);
            self.take("in")?;
            n["collection"] = self.expression(0)?;
            self.take("key")?;
            n["key"] = self.expression(0)?;
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
    fn declaration(&mut self, n: &mut V) -> R<()> {
        let start = self.pos();
        let required = self.peek() == "required";
        if required {
            self.i += 1
        }
        let kind = self.take("")?;
        if required && kind != "prop" {
            return self.err("required применяется только к prop");
        }
        let name = self.identifier(&kind)?;
        if kind == "enum" {
            if has(&n["enums"], &name) {
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
            n["enums"][&name] = json!(a);
            return Ok(());
        }
        if ["statementRanges", "propDefinitions", "eventDefinitions"]
            .iter()
            .any(|k| has(&n[k], &name))
        {
            return self.err(format!("Повторное свойство или событие {name}"));
        }
        if kind == "prop" {
            self.take(":")?;
            let ty = self.ty()?;
            n["propDefinitions"][&name] = json!({"type":ty,"required":required});
            if self.peek() == "=" {
                if required {
                    return self.err(format!(
                        "required prop {name} не принимает значение по умолчанию"
                    ));
                }
                self.i += 1;
                let from = self.pos();
                n["props"][&name] = self.sequence()?;
                n["propertyRanges"][&name] = json!({"from":from,"to":self.end()})
            } else if !required && !ty.ends_with('?') {
                return self.err(format!(
                    "prop {name}: требуется значение по умолчанию или required"
                ));
            } else if ty.ends_with('?') && !required {
                n["props"][&name] = V::Null
            }
        } else if kind == "event" {
            self.take("(")?;
            let mut args = Vec::new();
            let mut seen = HashSet::new();
            while self.peek() != ")" {
                let arg = self.identifier("event")?;
                if !seen.insert(arg.clone()) {
                    return self.err(format!("Повторный аргумент {arg}"));
                }
                self.take(":")?;
                args.push(json!({"name":arg,"type":self.ty()?}));
                if self.peek() != ")" {
                    self.take(",")?;
                }
            }
            self.take(")")?;
            n["eventDefinitions"][&name] = json!(args)
        } else {
            return self.err(format!("Неизвестное объявление {kind}"));
        }
        let end = self.pos() + 1;
        self.take(";")?;
        n["statementRanges"][&name] = json!({"from":start,"to":end});
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
        let ty = if let Some(t) = implicit {
            t.to_string()
        } else {
            self.take("")?
        };
        if implicit.is_none() && ["If", "For"].contains(&ty.as_str()) {
            return self.err(format!("{ty}: зарезервированный тип"));
        }
        let mut n = empty_node(&ty, start);
        if component {
            for k in ["propDefinitions", "eventDefinitions", "enums"] {
                n[k] = object()
            }
        }
        if self.peek() == ":" {
            self.i += 1;
            n["base"] = json!(self.take("")?)
        }
        self.take("{")?;
        while self.peek() != "}" {
            match self.peek() {
                "required" | "prop" | "event" | "enum" => {
                    if !component {
                        return self
                            .err("Объявления prop, event и enum разрешены только в component");
                    }
                    self.declaration(&mut n)?;
                    continue;
                }
                "match" => {
                    push(&mut n, "matches", self.match_value(true)?);
                    self.semi();
                    continue;
                }
                "forward" => {
                    self.i += 1;
                    self.take("props")?;
                    self.take("{")?;
                    while self.peek() != "}" {
                        let key = self.identifier("forward")?;
                        if n["forward"].as_array().unwrap().contains(&json!(key)) {
                            return self.err(format!("Повторный forward {key}"));
                        }
                        push(&mut n, "forward", json!(key));
                        if self.peek() != "}" {
                            self.take(",")?;
                        }
                    }
                    self.take("}")?;
                    self.take(";")?;
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
                        key = key[1..key.len() - 1].into()
                    }
                    if has(&n["slots"], &key) {
                        return self.err(format!("Повторный override {key}"));
                    }
                    n["slots"][&key] = if self.peek() == "{" {
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
            if has(&n["statementRanges"], &key) {
                return self.err(format!("Повторное свойство или событие {key}"));
            }
            match op.as_str() {
                ":" => {
                    let from = self.pos();
                    n["props"][&key] = self.sequence()?;
                    n["propertyRanges"][&key] = json!({"from":from,"to":self.end()})
                }
                "<->" => {
                    let p = self.take("")?;
                    if !p.contains('.') || !path(&p) {
                        return self.err("Двусторонняя привязка требует путь к полю");
                    }
                    n["bindings"][&key] = json!(p)
                }
                "->" => {
                    let a = self.take("")?;
                    if !path(&a)
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
                    n["events"][&key] = json!(a);
                    if !args.is_empty() {
                        if !has(&n, "eventArgs") {
                            n["eventArgs"] = object()
                        }
                        n["eventArgs"][&key] = json!(args)
                    }
                }
                _ => return self.err(format!("Неизвестный оператор {op}")),
            }
            let end = self.pos() + 1;
            self.take(";")?;
            n["statementRanges"][&key] = json!({"from":start,"to":end});
        }
        n["end"] = json!(self.pos() + 1);
        self.take("}")?;
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
        let mut d = json!({"name":"Preview","nodes":[],"designs":[],"states":[],"entries":[],"designBody":null,"overrides":{},"overrideTypes":{},"defaults":{},"defaultRanges":{},"base":null,"slots":{},"propDefinitions":{},"eventDefinitions":{},"enums":{},"matches":[],"forward":[]});
        while self.peek() == "#" {
            self.i += 1;
            self.take("[")?;
            self.take("design")?;
            if self.peek() == "(" {
                self.i += 1;
                push(&mut d, "designs", self.expression(0)?);
                self.take(")")?;
            }
            self.take("]")?;
        }
        if fragment {
            push(&mut d, "nodes", self.node(None, false)?)
        } else if self.peek() == "component" {
            self.i += 1;
            let c = self.node(None, true)?;
            d["name"] = c["type"].clone();
            d["base"] = c["base"].clone();
            for (k, v) in [
                ("slots", "slots"),
                ("defaultRanges", "statementRanges"),
                ("nodes", "children"),
                ("defaults", "props"),
                ("propDefinitions", "propDefinitions"),
                ("eventDefinitions", "eventDefinitions"),
                ("enums", "enums"),
                ("matches", "matches"),
                ("forward", "forward"),
            ] {
                d[k] = c[v].clone()
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
                let literal = self.tokens.get(self.i).cloned().ok_or_else(|| Error {
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
        check_keys(&d["nodes"], &mut HashSet::new())?;
        Ok(d)
    }
}
fn check_keys(nodes: &V, keys: &mut HashSet<String>) -> R<()> {
    if let Some(nodes) = nodes.as_array() {
        for n in nodes {
            if has(&n["props"], "key") {
                let key = n["props"]["key"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error {
                        offset: 0,
                        message: "key должен быть непустой строкой".into(),
                    })?;
                if !keys.insert(key.into()) {
                    return Err(Error {
                        offset: 0,
                        message: format!("Повторный key {key}"),
                    });
                }
            }
            if n["type"] == "For" {
                check_keys(&n["children"], &mut HashSet::new())?
            } else {
                check_keys(&n["children"], keys)?
            }
            check_keys(&n["elseChildren"], keys)?;
            check_keys(&n["emptyChildren"], keys)?;
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
    let mut out = String::new();
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
    ["px", "ms", "deg", "fr", "%"].iter().any(|unit| {
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
