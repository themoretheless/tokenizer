//! Bounded pure expression evaluation. References are resolved by the host;
//! there is no host-language eval or access to arbitrary object properties.
use crate::diagnostic::Diagnostic;
use serde_json::{Value as V, json};
type R<T> = Result<T, Diagnostic>;
pub fn truth(v: &V) -> R<bool> {
    v.as_bool()
        .ok_or_else(|| Diagnostic::new("expected-bool", "Логическое выражение ожидает Bool"))
}
fn number(v: &V) -> R<f64> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| Diagnostic::new("expected-number", "Арифметика ожидает конечное число"))
}
fn numeric(n: f64) -> R<V> {
    if n.is_finite() {
        Ok(json!(n))
    } else {
        Err(Diagnostic::new(
            "expected-number",
            "Арифметика ожидает конечное число",
        ))
    }
}
pub fn text(v: &V) -> R<String> {
    if let Some(s) = v["expr"].as_str() {
        return Ok(s.into());
    }
    match v {
        V::Null => Ok(String::new()),
        V::String(s) => Ok(s.clone()),
        V::Bool(b) => Ok(b.to_string()),
        V::Number(n) => Ok(if n.as_f64() == Some(0.) {
            "0".into()
        } else {
            n.as_f64().unwrap().to_string()
        }),
        _ => Err(Diagnostic::new(
            "expected-scalar",
            "Строковое преобразование ожидает скалярное значение",
        )),
    }
}
pub fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Number(a), V::Number(b)) => a.as_f64() == b.as_f64(),
        (V::Array(a), V::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        (V::Object(a), V::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|b| equal(v, b)))
        }
        _ => a == b,
    }
}
fn compare(a: &V, b: &V) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (V::String(a), V::String(b)) => Some(a.cmp(b)),
        (V::Number(a), V::Number(b)) => a.as_f64()?.partial_cmp(&b.as_f64()?),
        _ => None,
    }
}
fn parse_hex_color(v: &V) -> R<(u8, u8, u8, u8)> {
    let owned;
    let raw = if let Some(s) = v.as_str().or_else(|| v["expr"].as_str()) {
        s
    } else {
        owned = text(v)?;
        &owned
    };
    let s = raw
        .trim()
        .strip_prefix('#')
        .ok_or_else(|| {
            Diagnostic::new(
                "invalid-color",
                "Функция цвета ожидает hex-цвет (#rgb, #rrggbb или #rrggbbaa)",
            )
        })?;
    let hex = |slice: &str| {
        u8::from_str_radix(slice, 16)
            .map_err(|_| Diagnostic::new("invalid-color", "Некорректный hex-цвет"))
    };
    match s.len() {
        3 => {
            let r = hex(&s[0..1])?;
            let g = hex(&s[1..2])?;
            let b = hex(&s[2..3])?;
            Ok((r * 17, g * 17, b * 17, 255))
        }
        6 => Ok((hex(&s[0..2])?, hex(&s[2..4])?, hex(&s[4..6])?, 255)),
        8 => Ok((
            hex(&s[0..2])?,
            hex(&s[2..4])?,
            hex(&s[4..6])?,
            hex(&s[6..8])?,
        )),
        _ => Err(Diagnostic::new(
            "invalid-color",
            "Функция цвета ожидает #rgb, #rrggbb или #rrggbbaa",
        )),
    }
}
fn format_hex_color(r: u8, g: u8, b: u8, a: u8, as_expr: bool) -> V {
    let s = if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    };
    if as_expr {
        json!({"expr": s})
    } else {
        json!(s)
    }
}
pub fn matches_constraint_pattern(s: &str, pat: &str) -> R<bool> {
    Ok(match pat {
        "nonempty" => !s.trim().is_empty(),
        "digits" => !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
        "alpha" => !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
        "alphanumeric" => !s.is_empty() && s.chars().all(|c| c.is_alphanumeric()),
        "email" => match s.split_once('@') {
            Some((user, domain)) => {
                !user.is_empty()
                    && !user.contains('@')
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                    && domain.split('.').all(|p| !p.is_empty())
            }
            None => false,
        },
        "url" => {
            let rest = s
                .strip_prefix("https://")
                .or_else(|| s.strip_prefix("http://"));
            rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/') && !r.contains(' '))
        }
        "hex" => s.strip_prefix('#').is_some_and(|h| {
            [3, 6, 8].contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit())
        }),
        _ => return regex_like_match(s, pat),
    })
}

#[derive(Clone)]
enum Atom {
    Any,
    Literal(char),
    Digit,
    NonDigit,
    Word,
    NonWord,
    Space,
    NonSpace,
    Class(Vec<(char, char)>, bool),
}

impl Atom {
    fn matches(&self, c: char) -> bool {
        match self {
            Atom::Any => c != '\n',
            Atom::Literal(expected) => c == *expected,
            Atom::Digit => c.is_ascii_digit(),
            Atom::NonDigit => !c.is_ascii_digit(),
            Atom::Word => c.is_alphanumeric() || c == '_',
            Atom::NonWord => !(c.is_alphanumeric() || c == '_'),
            Atom::Space => c.is_whitespace(),
            Atom::NonSpace => !c.is_whitespace(),
            Atom::Class(ranges, neg) => {
                let hit = ranges.iter().any(|(lo, hi)| c >= *lo && c <= *hi);
                if *neg { !hit } else { hit }
            }
        }
    }
}

#[derive(Clone)]
struct TokenPat {
    atom: Atom,
    min: usize,
    max: Option<usize>,
}

fn parse_regex_tokens(pat: &str) -> (bool, bool, Vec<TokenPat>) {
    let (anchored_start, pat) = match pat.strip_prefix('^') {
        Some(rest) => (true, rest),
        None => (false, pat),
    };
    let mut chars: Vec<char> = pat.chars().collect();
    let anchored_end =
        if chars.last() == Some(&'$') && (chars.len() < 2 || chars[chars.len() - 2] != '\\') {
            chars.pop();
            true
        } else {
            false
        };
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let atom = match chars[i] {
            '.' => {
                i += 1;
                Atom::Any
            }
            '\\' if i + 1 < chars.len() => {
                let esc = chars[i + 1];
                i += 2;
                match esc {
                    'd' => Atom::Digit,
                    'D' => Atom::NonDigit,
                    'w' => Atom::Word,
                    'W' => Atom::NonWord,
                    's' => Atom::Space,
                    'S' => Atom::NonSpace,
                    'n' => Atom::Literal('\n'),
                    'r' => Atom::Literal('\r'),
                    't' => Atom::Literal('\t'),
                    other => Atom::Literal(other),
                }
            }
            '[' => {
                i += 1;
                let neg = if i < chars.len() && chars[i] == '^' {
                    i += 1;
                    true
                } else {
                    false
                };
                let mut ranges = Vec::new();
                while i < chars.len() && chars[i] != ']' {
                    let c1 = if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 2;
                        chars[i - 1]
                    } else {
                        let c = chars[i];
                        i += 1;
                        c
                    };
                    if i + 1 < chars.len() && chars[i] == '-' && chars[i + 1] != ']' {
                        i += 1;
                        let c2 = if chars[i] == '\\' && i + 1 < chars.len() {
                            i += 2;
                            chars[i - 1]
                        } else {
                            let c = chars[i];
                            i += 1;
                            c
                        };
                        ranges.push((c1, c2));
                    } else {
                        ranges.push((c1, c1));
                    }
                }
                if i < chars.len() && chars[i] == ']' {
                    i += 1;
                }
                Atom::Class(ranges, neg)
            }
            c => {
                i += 1;
                Atom::Literal(c)
            }
        };
        let (min, max) = if i < chars.len() {
            match chars[i] {
                '*' => {
                    i += 1;
                    (0, None)
                }
                '+' => {
                    i += 1;
                    (1, None)
                }
                '?' => {
                    i += 1;
                    (0, Some(1))
                }
                '{' => {
                    let save = i;
                    i += 1;
                    let mut n1 = String::new();
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        n1.push(chars[i]);
                        i += 1;
                    }
                    if let Ok(lo) = n1.parse::<usize>() {
                        if i < chars.len() && chars[i] == '}' {
                            i += 1;
                            (lo, Some(lo))
                        } else if i < chars.len() && chars[i] == ',' {
                            i += 1;
                            let mut n2 = String::new();
                            while i < chars.len() && chars[i].is_ascii_digit() {
                                n2.push(chars[i]);
                                i += 1;
                            }
                            if i < chars.len() && chars[i] == '}' {
                                i += 1;
                                let hi = if n2.is_empty() {
                                    None
                                } else {
                                    n2.parse::<usize>().ok()
                                };
                                (lo, hi)
                            } else {
                                i = save;
                                (1, Some(1))
                            }
                        } else {
                            i = save;
                            (1, Some(1))
                        }
                    } else {
                        i = save;
                        (1, Some(1))
                    }
                }
                _ => (1, Some(1)),
            }
        } else {
            (1, Some(1))
        };
        tokens.push(TokenPat { atom, min, max });
    }
    (anchored_start, anchored_end, tokens)
}

fn match_tokens_at(
    tokens: &[TokenPat],
    ti: usize,
    chars: &[char],
    ci: usize,
    anchored_end: bool,
    steps: &mut usize,
) -> R<bool> {
    *steps += 1;
    if *steps > 8192 {
        return Err(Diagnostic::new(
            "pattern-too-complex",
            "Шаблон слишком сложен: превышен предел в 8192 шага сопоставления",
        ));
    }
    if ti == tokens.len() {
        return Ok(!anchored_end || ci == chars.len());
    }
    let tok = &tokens[ti];
    let mut count = 0usize;
    let mut end_ci = ci;
    while end_ci < chars.len()
        && tok.max.is_none_or(|m| count < m)
        && tok.atom.matches(chars[end_ci])
    {
        end_ci += 1;
        count += 1;
    }
    if count < tok.min {
        return Ok(false);
    }
    for k in (tok.min..=count).rev() {
        if match_tokens_at(tokens, ti + 1, chars, ci + k, anchored_end, steps)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn regex_like_match(s: &str, pat: &str) -> R<bool> {
    if pat.contains('|') && !pat.contains('[') {
        for sub in pat.split('|') {
            if regex_like_match(s, sub)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let (anchored_start, anchored_end, tokens) = parse_regex_tokens(pat);
    let chars: Vec<char> = s.chars().collect();
    let mut steps = 0usize;
    if anchored_start {
        match_tokens_at(&tokens, 0, &chars, 0, anchored_end, &mut steps)
    } else {
        for start in 0..=chars.len() {
            if match_tokens_at(&tokens, 0, &chars, start, anchored_end, &mut steps)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub fn matches_pattern<F>(p: &V, v: &V, evaluate: &mut F) -> R<bool>
where
    F: FnMut(&V) -> R<V>,
{
    if p["expr"] == "_" {
        return Ok(true);
    }
    if let Some(alts) = p["or"].as_array() {
        for alt in alts {
            if matches_pattern(alt, v, evaluate)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if let Some(a) = p.as_array() {
        let Some(b) = v.as_array() else {
            return Ok(false);
        };
        if a.len() != b.len() {
            return Ok(false);
        }
        for (p, v) in a.iter().zip(b) {
            if !matches_pattern(p, v, evaluate)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if let Some(op) = p["comparison"].as_str() {
        let value = evaluate(&p["value"])?;
        return Ok(compare(v, &value).is_some_and(|o| match op {
            "<" => o.is_lt(),
            "<=" => !o.is_gt(),
            ">" => o.is_gt(),
            _ => !o.is_lt(),
        }));
    }
    if p["expression"]["kind"] == "binary"
        && let Some(op) = p["expression"]["operator"].as_str()
        && (op == ".." || op == "..=")
    {
        let lo = evaluate(&p["expression"]["left"])?;
        let hi = evaluate(&p["expression"]["right"])?;
        let Some(vn) = v.as_f64() else {
            return Ok(false);
        };
        let lo = number(&lo)?;
        let hi = number(&hi)?;
        return Ok(if op == "..=" {
            vn >= lo && vn <= hi
        } else {
            vn >= lo && vn < hi
        });
    }
    Ok(equal(&evaluate(p)?, v))
}
pub fn evaluate<F>(value: &V, resolver: &mut F) -> R<V>
where
    F: FnMut(&str, bool) -> R<V>,
{
    run(value, false, 0, resolver)
}
fn run<F>(v: &V, optional: bool, depth: usize, resolver: &mut F) -> R<V>
where
    F: FnMut(&str, bool) -> R<V>,
{
    if depth > 128 {
        return Err(Diagnostic::new(
            "nesting-limit",
            "Превышен предел вложенности выражения",
        ));
    }
    let next = depth + 1;
    if let Some(a) = v.as_array() {
        return a
            .iter()
            .map(|v| run(v, false, next, resolver))
            .collect::<R<Vec<_>>>()
            .map(V::Array);
    }
    if !v.is_object() {
        return Ok(v.clone());
    }
    if let Some(path) = v["expr"].as_str() {
        let neg = path.starts_with('!');
        let value = resolver(if neg { &path[1..] } else { path }, optional)?;
        return if neg {
            Ok(json!(!truth(&value)?))
        } else {
            Ok(value)
        };
    }
    if let Some(parts) = v["interpolation"].as_array() {
        let mut out = String::new();
        for p in parts {
            out.push_str(&if let Some(s) = p.as_str() {
                s.into()
            } else {
                text(&run(p, false, next, resolver)?)?
            })
        }
        return Ok(json!(out));
    }
    if let Some(subject) = v.get("match") {
        let actual = run(subject, false, next, resolver)?;
        for branch in v["branches"]
            .as_array()
            .ok_or_else(|| Diagnostic::new("invalid-match", "Некорректные ветки match"))?
        {
            if matches_pattern(&branch["pattern"], &actual, &mut |p| {
                run(p, false, next, resolver)
            })? {
                if let Some(g) = branch.get("guard")
                    && !truth(&run(g, false, next, resolver)?)?
                {
                    continue;
                }
                return run(&branch["value"], false, next, resolver);
            }
        }
        return Err(Diagnostic::new(
            "no-match-branch",
            "match: нет подходящей ветки; добавьте _ => …",
        ));
    }
    let Some(e) = v.get("expression") else {
        let mut o = serde_json::Map::new();
        for (k, v) in v.as_object().unwrap() {
            o.insert(k.clone(), run(v, false, next, resolver)?);
        }
        return Ok(V::Object(o));
    };
    let k = e["kind"]
        .as_str()
        .ok_or_else(|| Diagnostic::new("invalid-expression", "Некорректное выражение"))?;
    match k {
        "conditional" => {
            let condition = run(&e["condition"], false, next, resolver)?;
            run(
                &e[if truth(&condition)? { "then" } else { "else" }],
                false,
                next,
                resolver,
            )
        }
        "unary" => {
            let a = run(&e["argument"], false, next, resolver)?;
            match e["operator"].as_str().unwrap_or("") {
                "!" => Ok(json!(!truth(&a)?)),
                "-" => numeric(-number(&a)?),
                _ => numeric(number(&a)?),
            }
        }
        "member" => {
            let opt = e["optional"] == true;
            let object = run(&e["object"], optional || opt, next, resolver)?;
            if object.is_null() {
                return if optional || opt {
                    Ok(V::Null)
                } else {
                    Err(Diagnostic::new(
                        "null-property-access",
                        "Нельзя прочитать свойство пустого значения; используйте ?.",
                    ))
                };
            }
            let computed;
            let key: &str = if let Some(s) = e["property"].as_str() {
                s
            } else {
                let p = run(&e["property"], false, next, resolver)?;
                computed = text(&p)?;
                &computed
            };
            if matches!(key, "__proto__" | "prototype" | "constructor") {
                return Err(Diagnostic::new(
                    "forbidden-property",
                    format!("Недоступное свойство {key}"),
                ));
            }
            if key == "length" {
                if let Some(s) = object.as_str() {
                    return Ok(json!(s.encode_utf16().count()));
                }
                if let Some(a) = object.as_array() {
                    return Ok(json!(a.len()));
                }
            }
            let value = if let Some(a) = object.as_array() {
                key.parse::<usize>().ok().and_then(|i| a.get(i))
            } else {
                object.as_object().and_then(|o| o.get(key))
            };
            match value {
                Some(v) => Ok(v.clone()),
                None if optional || opt => Ok(V::Null),
                _ => Err(Diagnostic::new(
                    "unknown-property",
                    format!("Неизвестное свойство {key}"),
                )),
            }
        }
        "call" => {
            let name = e["name"]
                .as_str()
                .ok_or_else(|| Diagnostic::new("invalid-call", "Некорректный вызов"))?;
            let raw_args = e["args"]
                .as_array()
                .ok_or_else(|| Diagnostic::new("invalid-call", "Некорректные аргументы"))?;
            if name == "coalesce" {
                if raw_args.is_empty() {
                    return Err(Diagnostic::new(
                        "invalid-argument-count",
                        "coalesce: неверное число аргументов",
                    ));
                }
                for raw in raw_args {
                    let val = run(raw, true, next, resolver)?;
                    if !val.is_null() {
                        return Ok(val);
                    }
                }
                return Ok(V::Null);
            }
            let mut args = Vec::with_capacity(raw_args.len());
            for v in raw_args {
                args.push(run(v, false, next, resolver)?);
            }
            let count = args.len();
            let valid = match name {
                "min" | "max" => count >= 1,
                "merge" => count >= 2,
                "remap" => count == 5,
                "clamp" | "replace" | "find" | "lerp" => count == 3,
                "plural" => count == 3 || count == 4,
                "contains" | "includes" | "indexOf" | "startsWith" | "endsWith" | "split"
                | "take" | "skip" | "range" | "pluck" | "alpha" | "lighten" | "darken"
                | "saturate" | "desaturate" | "contrast" | "groupBy" | "pow" | "repeat"
                | "chunk" | "zip" | "pick" | "omit" => count == 2,
                "slice" | "mix" | "isValid" | "padStart" | "padEnd" | "truncate" => {
                    count == 2 || count == 3
                }
                "join" | "formatNumber" | "sortBy" | "unique" => count == 1 || count == 2,
                _ => count == 1,
            };
            if !valid {
                return Err(Diagnostic::new(
                    "invalid-argument-count",
                    format!("{name}: неверное число аргументов"),
                ));
            }
            match name {
                "String" => Ok(json!(text(&args[0])?)),
                "Bool" => Ok(json!(truth(&args[0])?)),
                "Number" => {
                    let n = if args[0].is_number() {
                        number(&args[0])?
                    } else if let Some(s) = args[0].as_str() {
                        let s = s.trim();
                        if s.is_empty() || s.starts_with(['i', 'I', 'n', 'N']) {
                            return Err(Diagnostic::new(
                                "invalid-argument",
                                "Number ожидает число или непустую числовую строку",
                            ));
                        }
                        s.parse::<f64>().map_err(|_| {
                            Diagnostic::new(
                                "invalid-argument",
                                "Number ожидает число или непустую числовую строку",
                            )
                        })?
                    } else {
                        return Err(Diagnostic::new(
                            "invalid-argument",
                            "Number ожидает число или непустую числовую строку",
                        ));
                    };
                    numeric(n)
                }
                "len" => {
                    if let Some(s) = args[0].as_str() {
                        Ok(json!(s.encode_utf16().count()))
                    } else if let Some(a) = args[0].as_array() {
                        Ok(json!(a.len()))
                    } else if let Some(o) = args[0].as_object()
                        && !o.contains_key("expr")
                    {
                        Ok(json!(o.len()))
                    } else {
                        Err(Diagnostic::new(
                            "invalid-argument",
                            "len ожидает строку, массив или объект",
                        ))
                    }
                }
                "isEmpty" => {
                    let empty = match &args[0] {
                        V::Null => true,
                        V::String(s) => s.trim().is_empty(),
                        V::Array(a) => a.is_empty(),
                        V::Object(o) => !o.contains_key("expr") && o.is_empty(),
                        _ => false,
                    };
                    Ok(json!(empty))
                }
                "count" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "count ожидает массив")
                    })?;
                    Ok(json!(a.len()))
                }
                "any" => {
                    let a = args[0]
                        .as_array()
                        .ok_or_else(|| Diagnostic::new("invalid-argument", "any ожидает массив"))?;
                    let mut res = false;
                    for item in a {
                        if truth(item)? {
                            res = true;
                            break;
                        }
                    }
                    Ok(json!(res))
                }
                "all" => {
                    let a = args[0]
                        .as_array()
                        .ok_or_else(|| Diagnostic::new("invalid-argument", "all ожидает массив"))?;
                    let mut res = true;
                    for item in a {
                        if !truth(item)? {
                            res = false;
                            break;
                        }
                    }
                    Ok(json!(res))
                }
                "pluck" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "pluck ожидает массив")
                    })?;
                    let key = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "pluck ожидает имя поля-строку")
                    })?;
                    let mut out = Vec::with_capacity(a.len());
                    for item in a {
                        out.push(item.get(key).cloned().unwrap_or(V::Null));
                    }
                    Ok(V::Array(out))
                }
                "sortBy" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "sortBy ожидает массив")
                    })?;
                    let key = if count == 2 {
                        Some(args[1].as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "sortBy ожидает имя поля-строку")
                        })?)
                    } else {
                        None
                    };
                    let mut v = a.clone();
                    v.sort_by(|x, y| {
                        let kx = key.map_or(x, |k| x.get(k).unwrap_or(&V::Null));
                        let ky = key.map_or(y, |k| y.get(k).unwrap_or(&V::Null));
                        compare(kx, ky).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    Ok(V::Array(v))
                }
                "alpha" => {
                    let (r, g, b, _) = parse_hex_color(&args[0])?;
                    let a = (number(&args[1])?.clamp(0.0, 1.0) * 255.0).round() as u8;
                    Ok(format_hex_color(r, g, b, a, args[0].get("expr").is_some()))
                }
                "lighten" | "darken" => {
                    let (r, g, b, a) = parse_hex_color(&args[0])?;
                    let amt = number(&args[1])?.clamp(0.0, 1.0);
                    let shift = |c: u8| -> u8 {
                        if name == "lighten" {
                            (c as f64 + (255.0 - c as f64) * amt).round() as u8
                        } else {
                            (c as f64 * (1.0 - amt)).round() as u8
                        }
                    };
                    Ok(format_hex_color(
                        shift(r),
                        shift(g),
                        shift(b),
                        a,
                        args[0].get("expr").is_some(),
                    ))
                }
                "saturate" | "desaturate" => {
                    let (r, g, b, a) = parse_hex_color(&args[0])?;
                    let amt = number(&args[1])?.clamp(0.0, 1.0);
                    let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                    let max = rf.max(gf).max(bf);
                    let min = rf.min(gf).min(bf);
                    let l = (max + min) * 0.5;
                    if (max - min).abs() < f64::EPSILON {
                        Ok(format_hex_color(r, g, b, a, args[0].get("expr").is_some()))
                    } else {
                        let d = max - min;
                        let s = if l > 0.5 {
                            d / (2.0 - max - min)
                        } else {
                            d / (max + min)
                        };
                        let new_s = if name == "saturate" {
                            (s + (1.0 - s) * amt).clamp(0.0, 1.0)
                        } else {
                            (s * (1.0 - amt)).clamp(0.0, 1.0)
                        };
                        let factor = if s > 0.0 { new_s / s } else { 0.0 };
                        let adj =
                            |c: f64| ((l + (c - l) * factor).clamp(0.0, 1.0) * 255.0).round() as u8;
                        Ok(format_hex_color(
                            adj(rf),
                            adj(gf),
                            adj(bf),
                            a,
                            args[0].get("expr").is_some(),
                        ))
                    }
                }
                "invert" => {
                    let (r, g, b, a) = parse_hex_color(&args[0])?;
                    Ok(format_hex_color(
                        255 - r,
                        255 - g,
                        255 - b,
                        a,
                        args[0].get("expr").is_some(),
                    ))
                }
                "contrast" => {
                    let (r1, g1, b1, _) = parse_hex_color(&args[0])?;
                    let (r2, g2, b2, _) = parse_hex_color(&args[1])?;
                    let lum = |r: u8, g: u8, b: u8| -> f64 {
                        let ch = |c: u8| -> f64 {
                            let s = c as f64 / 255.0;
                            if s <= 0.03928 {
                                s / 12.92
                            } else {
                                ((s + 0.055) / 1.055).powf(2.4)
                            }
                        };
                        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
                    };
                    let l1 = lum(r1, g1, b1);
                    let l2 = lum(r2, g2, b2);
                    let ratio = (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05);
                    numeric((ratio * 100.0).round() / 100.0)
                }
                "mix" => {
                    let (r1, g1, b1, a1) = parse_hex_color(&args[0])?;
                    let (r2, g2, b2, a2) = parse_hex_color(&args[1])?;
                    let w = if count == 3 {
                        number(&args[2])?.clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    let lerp = |c1: u8, c2: u8| -> u8 {
                        (c1 as f64 * (1.0 - w) + c2 as f64 * w).round() as u8
                    };
                    Ok(format_hex_color(
                        lerp(r1, r2),
                        lerp(g1, g2),
                        lerp(b1, b2),
                        lerp(a1, a2),
                        args[0].get("expr").is_some() || args[1].get("expr").is_some(),
                    ))
                }
                "min" | "max" => {
                    let numbers = args.iter().map(number).collect::<R<Vec<_>>>()?;
                    numeric(
                        numbers
                            .into_iter()
                            .reduce(if name == "min" { f64::min } else { f64::max })
                            .unwrap(),
                    )
                }
                "clamp" => {
                    let a = number(&args[0])?;
                    let lo = number(&args[1])?;
                    let hi = number(&args[2])?;
                    if lo > hi {
                        return Err(Diagnostic::new(
                            "invalid-range",
                            "clamp: минимум превышает максимум",
                        ));
                    }
                    numeric(a.max(lo).min(hi))
                }
                "pow" => numeric(number(&args[0])?.powf(number(&args[1])?)),
                "sqrt" => {
                    let n = number(&args[0])?;
                    if n < 0.0 {
                        return Err(Diagnostic::new(
                            "invalid-argument",
                            "sqrt ожидает неотрицательное число",
                        ));
                    }
                    numeric(n.sqrt())
                }
                "sign" => {
                    let n = number(&args[0])?;
                    numeric(if n == 0.0 { 0.0 } else { n.signum() })
                }
                "lerp" => {
                    let a = number(&args[0])?;
                    let b = number(&args[1])?;
                    let t = number(&args[2])?;
                    numeric(a + (b - a) * t)
                }
                "remap" => {
                    let v = number(&args[0])?;
                    let i0 = number(&args[1])?;
                    let i1 = number(&args[2])?;
                    let o0 = number(&args[3])?;
                    let o1 = number(&args[4])?;
                    if (i1 - i0).abs() < f64::EPSILON {
                        return Err(Diagnostic::new(
                            "invalid-range",
                            "remap: пустой входной диапазон",
                        ));
                    }
                    numeric(o0 + ((v - i0) / (i1 - i0)) * (o1 - o0))
                }
                "avg" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "avg ожидает массив чисел")
                    })?;
                    if a.is_empty() {
                        return Ok(json!(0.0));
                    }
                    let mut total = 0.0;
                    for item in a {
                        total += number(item)?;
                    }
                    numeric(total / a.len() as f64)
                }
                "abs" | "round" | "floor" | "ceil" => {
                    let n = number(&args[0])?;
                    numeric(match name {
                        "abs" => n.abs(),
                        "round" => (n + 0.5).floor(),
                        "floor" => n.floor(),
                        _ => n.ceil(),
                    })
                }
                "trim" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "trim ожидает строку")
                    })?;
                    Ok(json!(s.trim()))
                }
                "upper" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "upper ожидает строку")
                    })?;
                    Ok(json!(s.to_uppercase()))
                }
                "lower" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "lower ожидает строку")
                    })?;
                    Ok(json!(s.to_lowercase()))
                }
                "capitalize" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "capitalize ожидает строку")
                    })?;
                    let mut chars = s.chars();
                    let out = match chars.next() {
                        Some(first) => {
                            first.to_uppercase().collect::<String>()
                                + &chars.as_str().to_lowercase()
                        }
                        None => String::new(),
                    };
                    Ok(json!(out))
                }
                "padStart" | "padEnd" => {
                    let s = args[0]
                        .as_str()
                        .ok_or_else(|| Diagnostic::new("invalid-argument", "pad ожидает строку"))?;
                    let target = (number(&args[1])?.max(0.) as usize).min(4096);
                    let pad = if count == 3 {
                        args[2].as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "pad ожидает строку заполнения")
                        })?
                    } else {
                        " "
                    };
                    let cur = s.chars().count();
                    if cur >= target || pad.is_empty() {
                        return Ok(json!(s));
                    }
                    let needed = target - cur;
                    let pad_chars: Vec<char> = pad.chars().collect();
                    let fill: String = (0..needed)
                        .map(|i| pad_chars[i % pad_chars.len()])
                        .collect();
                    Ok(json!(if name == "padStart" {
                        format!("{fill}{s}")
                    } else {
                        format!("{s}{fill}")
                    }))
                }
                "truncate" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "truncate ожидает строку")
                    })?;
                    let max_len = number(&args[1])?.max(0.) as usize;
                    let suffix = if count == 3 {
                        args[2].as_str().ok_or_else(|| {
                            Diagnostic::new(
                                "invalid-argument",
                                "truncate ожидает строковый суффикс",
                            )
                        })?
                    } else {
                        "…"
                    };
                    let chars: Vec<char> = s.chars().collect();
                    if chars.len() <= max_len {
                        Ok(json!(s))
                    } else {
                        let sfx_len = suffix.chars().count();
                        let keep = max_len.saturating_sub(sfx_len);
                        let prefix: String = chars[..keep].iter().collect();
                        Ok(json!(format!("{prefix}{suffix}")))
                    }
                }
                "repeat" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "repeat ожидает строку")
                    })?;
                    let n = number(&args[1])?.max(0.) as usize;
                    if s.len().saturating_mul(n) > 65536 {
                        return Err(Diagnostic::new(
                            "size-limit",
                            "repeat: превышен предел длины строки",
                        ));
                    }
                    Ok(json!(s.repeat(n)))
                }
                "contains" | "includes" => {
                    if let Some(s) = args[0].as_str() {
                        let sub = args[1].as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "contains ожидает подстроку")
                        })?;
                        Ok(json!(s.contains(sub)))
                    } else if let Some(a) = args[0].as_array() {
                        Ok(json!(a.iter().any(|x| equal(x, &args[1]))))
                    } else {
                        Err(Diagnostic::new(
                            "invalid-argument",
                            "contains ожидает строку или массив",
                        ))
                    }
                }
                "indexOf" => {
                    if let Some(s) = args[0].as_str() {
                        let sub = args[1].as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "indexOf ожидает подстроку")
                        })?;
                        let idx = s
                            .find(sub)
                            .map(|byte_idx| s[..byte_idx].chars().count() as i64)
                            .unwrap_or(-1);
                        Ok(json!(idx))
                    } else if let Some(a) = args[0].as_array() {
                        let idx = a
                            .iter()
                            .position(|x| equal(x, &args[1]))
                            .map(|i| i as i64)
                            .unwrap_or(-1);
                        Ok(json!(idx))
                    } else {
                        Err(Diagnostic::new(
                            "invalid-argument",
                            "indexOf ожидает строку или массив",
                        ))
                    }
                }
                "startsWith" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "startsWith ожидает строку")
                    })?;
                    let p = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "startsWith ожидает префикс-строку")
                    })?;
                    Ok(json!(s.starts_with(p)))
                }
                "endsWith" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "endsWith ожидает строку")
                    })?;
                    let sfx = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "endsWith ожидает суффикс-строку")
                    })?;
                    Ok(json!(s.ends_with(sfx)))
                }
                "replace" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "replace ожидает строку")
                    })?;
                    let from = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "replace ожидает искомую подстроку")
                    })?;
                    let to = args[2].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "replace ожидает строку замены")
                    })?;
                    Ok(json!(s.replace(from, to)))
                }
                "slice" => {
                    let start = number(&args[1])? as isize;
                    let end = if count == 3 {
                        Some(number(&args[2])? as isize)
                    } else {
                        None
                    };
                    let norm = |idx: isize, len: usize| -> usize {
                        if idx < 0 {
                            len.saturating_sub(idx.unsigned_abs())
                        } else {
                            (idx as usize).min(len)
                        }
                    };
                    if let Some(s) = args[0].as_str() {
                        let chars: Vec<char> = s.chars().collect();
                        let len = chars.len();
                        let st = norm(start, len);
                        let en = norm(end.unwrap_or(len as isize), len).max(st);
                        Ok(json!(chars[st..en].iter().collect::<String>()))
                    } else if let Some(a) = args[0].as_array() {
                        let len = a.len();
                        let st = norm(start, len);
                        let en = norm(end.unwrap_or(len as isize), len).max(st);
                        Ok(json!(a[st..en].to_vec()))
                    } else {
                        Err(Diagnostic::new(
                            "invalid-argument",
                            "slice ожидает строку или массив",
                        ))
                    }
                }
                "split" => {
                    let s = args[0].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "split ожидает строку")
                    })?;
                    let sep = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "split ожидает разделитель-строку")
                    })?;
                    Ok(json!(s.split(sep).collect::<Vec<_>>()))
                }
                "join" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "join ожидает массив")
                    })?;
                    let sep = if count == 2 {
                        args[1].as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "join ожидает разделитель-строку")
                        })?
                    } else {
                        ", "
                    };
                    let mut parts = Vec::with_capacity(a.len());
                    for item in a {
                        parts.push(text(item)?);
                    }
                    Ok(json!(parts.join(sep)))
                }
                "reverse" => {
                    if let Some(a) = args[0].as_array() {
                        let mut v = a.clone();
                        v.reverse();
                        Ok(V::Array(v))
                    } else if let Some(s) = args[0].as_str() {
                        Ok(json!(s.chars().rev().collect::<String>()))
                    } else {
                        Err(Diagnostic::new(
                            "invalid-argument",
                            "reverse ожидает массив или строку",
                        ))
                    }
                }
                "take" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "take ожидает массив")
                    })?;
                    let n = number(&args[1])?.max(0.) as usize;
                    Ok(V::Array(a.iter().take(n).cloned().collect()))
                }
                "skip" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "skip ожидает массив")
                    })?;
                    let n = number(&args[1])?.max(0.) as usize;
                    Ok(V::Array(a.iter().skip(n).cloned().collect()))
                }
                "sum" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "sum ожидает массив чисел")
                    })?;
                    let mut total = 0.0;
                    for item in a {
                        total += number(item)?;
                    }
                    numeric(total)
                }
                "range" => {
                    let st = number(&args[0])? as i64;
                    let en = number(&args[1])? as i64;
                    if en < st {
                        return Ok(json!([]));
                    }
                    if en - st > 4096 {
                        return Err(Diagnostic::new(
                            "size-limit",
                            "Превышен предел размера диапазона (макс. 4096)",
                        ));
                    }
                    Ok(V::Array((st..en).map(|i| json!(i)).collect()))
                }
                "formatNumber" => {
                    let n = number(&args[0])?;
                    if count == 2 {
                        let dec = (number(&args[1])?.max(0.) as usize).min(12);
                        Ok(json!(format!("{n:.dec$}")))
                    } else {
                        text(&json!(n)).map(|s| json!(s))
                    }
                }
                "unique" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "unique ожидает массив")
                    })?;
                    let key = if count == 2 {
                        Some(args[1].as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "unique ожидает имя поля-строку")
                        })?)
                    } else {
                        None
                    };
                    let mut out = Vec::new();
                    for item in a {
                        let k = key.map_or(item, |f| item.get(f).unwrap_or(&V::Null));
                        let exists = out.iter().any(|existing: &V| {
                            let ek = key.map_or(existing, |f| existing.get(f).unwrap_or(&V::Null));
                            equal(ek, k)
                        });
                        if !exists {
                            out.push(item.clone());
                        }
                    }
                    Ok(V::Array(out))
                }
                "find" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "find ожидает массив")
                    })?;
                    let key = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "find ожидает имя поля-строку")
                    })?;
                    let target = &args[2];
                    let found = a
                        .iter()
                        .find(|item| item.get(key).is_some_and(|v| equal(v, target)))
                        .cloned()
                        .unwrap_or(V::Null);
                    Ok(found)
                }
                "compact" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "compact ожидает массив")
                    })?;
                    let out: Vec<V> = a
                        .iter()
                        .filter(|v| !v.is_null() && *v != &V::Bool(false) && v.as_str() != Some(""))
                        .cloned()
                        .collect();
                    Ok(V::Array(out))
                }
                "flatten" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "flatten ожидает массив")
                    })?;
                    let mut out = Vec::with_capacity(a.len());
                    for item in a {
                        if let Some(sub) = item.as_array() {
                            out.extend(sub.iter().cloned());
                        } else {
                            out.push(item.clone());
                        }
                    }
                    Ok(V::Array(out))
                }
                "chunk" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "chunk ожидает массив")
                    })?;
                    let sz = number(&args[1])? as usize;
                    if sz == 0 {
                        return Err(Diagnostic::new(
                            "invalid-argument",
                            "chunk: размер должен быть положительным",
                        ));
                    }
                    Ok(V::Array(
                        a.chunks(sz).map(|c| V::Array(c.to_vec())).collect(),
                    ))
                }
                "zip" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "zip ожидает два массива")
                    })?;
                    let b = args[1].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "zip ожидает два массива")
                    })?;
                    Ok(V::Array(
                        a.iter().zip(b.iter()).map(|(x, y)| json!([x, y])).collect(),
                    ))
                }
                "first" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "first ожидает массив")
                    })?;
                    Ok(a.first().cloned().unwrap_or(V::Null))
                }
                "last" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "last ожидает массив")
                    })?;
                    Ok(a.last().cloned().unwrap_or(V::Null))
                }
                "groupBy" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "groupBy ожидает массив")
                    })?;
                    let key = args[1].as_str().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "groupBy ожидает имя поля-строку")
                    })?;
                    let mut groups: Vec<(V, Vec<V>)> = Vec::new();
                    for item in a {
                        let k = item.get(key).cloned().unwrap_or(V::Null);
                        if let Some((_, items)) = groups.iter_mut().find(|(gk, _)| equal(gk, &k)) {
                            items.push(item.clone());
                        } else {
                            groups.push((k, vec![item.clone()]));
                        }
                    }
                    Ok(V::Array(
                        groups
                            .into_iter()
                            .map(|(k, items)| json!({"key": k, "items": items}))
                            .collect(),
                    ))
                }
                "keys" => {
                    let o = args[0].as_object().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "keys ожидает объект")
                    })?;
                    Ok(V::Array(o.keys().map(|k| json!(k)).collect()))
                }
                "values" => {
                    let o = args[0].as_object().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "values ожидает объект")
                    })?;
                    Ok(V::Array(o.values().cloned().collect()))
                }
                "entries" => {
                    let o = args[0].as_object().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "entries ожидает объект")
                    })?;
                    Ok(V::Array(o.iter().map(|(k, v)| json!([k, v])).collect()))
                }
                "fromEntries" => {
                    let a = args[0].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "fromEntries ожидает массив пар")
                    })?;
                    let mut out = serde_json::Map::with_capacity(a.len());
                    for item in a {
                        if let Some(pair) = item.as_array()
                            && pair.len() == 2
                        {
                            out.insert(text(&pair[0])?, pair[1].clone());
                        } else if let Some(obj) = item.as_object()
                            && let (Some(k), Some(v)) = (obj.get("key"), obj.get("value"))
                        {
                            out.insert(text(k)?, v.clone());
                        } else {
                            return Err(Diagnostic::new(
                                "invalid-argument",
                                "fromEntries ожидает пары [key, value] или {key, value}",
                            ));
                        }
                    }
                    Ok(V::Object(out))
                }
                "merge" => {
                    let mut out = serde_json::Map::new();
                    for arg in &args {
                        let o = arg.as_object().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "merge ожидает объекты")
                        })?;
                        for (k, v) in o {
                            out.insert(k.clone(), v.clone());
                        }
                    }
                    Ok(V::Object(out))
                }
                "pick" | "omit" => {
                    let o = args[0]
                        .as_object()
                        .ok_or_else(|| Diagnostic::new("invalid-argument", "ожидается объект"))?;
                    let ks = args[1].as_array().ok_or_else(|| {
                        Diagnostic::new("invalid-argument", "ожидается массив ключей")
                    })?;
                    let mut key_set = std::collections::HashSet::with_capacity(ks.len());
                    for k in ks {
                        key_set.insert(k.as_str().ok_or_else(|| {
                            Diagnostic::new("invalid-argument", "ключ должен быть строкой")
                        })?);
                    }
                    let mut out = serde_json::Map::new();
                    for (k, v) in o {
                        let hit = key_set.contains(k.as_str());
                        if (name == "pick" && hit) || (name == "omit" && !hit) {
                            out.insert(k.clone(), v.clone());
                        }
                    }
                    Ok(V::Object(out))
                }
                "plural" => {
                    let n = number(&args[0])?;
                    if count == 3 {
                        let idx = if n.abs() == 1.0 { 1 } else { 2 };
                        Ok(json!(text(&args[idx])?))
                    } else {
                        let abs_int = (n.abs().trunc() as i64).abs();
                        let mod10 = abs_int % 10;
                        let mod100 = abs_int % 100;
                        let idx = if n.fract() != 0.0 {
                            2
                        } else if mod10 == 1 && mod100 != 11 {
                            1
                        } else if (2..=4).contains(&mod10) && !(12..=14).contains(&mod100) {
                            2
                        } else {
                            3
                        };
                        Ok(json!(text(&args[idx])?))
                    }
                }
                "isValid" => {
                    if count == 3 {
                        let min = number(&args[1])?;
                        let max = number(&args[2])?;
                        let ok = if let Some(n) = args[0].as_f64() {
                            n >= min && n <= max
                        } else if let Some(s) = args[0].as_str() {
                            let len = s.chars().count() as f64;
                            len >= min && len <= max
                        } else if let Some(a) = args[0].as_array() {
                            let len = a.len() as f64;
                            len >= min && len <= max
                        } else {
                            false
                        };
                        Ok(json!(ok))
                    } else {
                        let pat = args[1].as_str().ok_or_else(|| {
                            Diagnostic::new(
                                "invalid-argument",
                                "isValid ожидает строковый шаблон или правило",
                            )
                        })?;
                        let s = text(&args[0])?;
                        Ok(json!(matches_constraint_pattern(&s, pat)?))
                    }
                }
                _ => Err(Diagnostic::new(
                    "unknown-function",
                    format!("Неизвестная чистая функция {name}"),
                )),
            }
        }
        "binary" => {
            let op = e["operator"]
                .as_str()
                .ok_or_else(|| Diagnostic::new("invalid-operator", "Некорректный оператор"))?;
            let a = run(&e["left"], op == "??" || optional, next, resolver)?;
            match op {
                "??" => {
                    return if a.is_null() {
                        run(&e["right"], false, next, resolver)
                    } else {
                        Ok(a)
                    };
                }
                "&&" => {
                    return if truth(&a)? {
                        let b = run(&e["right"], false, next, resolver)?;
                        Ok(json!(truth(&b)?))
                    } else {
                        Ok(json!(false))
                    };
                }
                "||" => {
                    return if truth(&a)? {
                        Ok(json!(true))
                    } else {
                        let b = run(&e["right"], false, next, resolver)?;
                        Ok(json!(truth(&b)?))
                    };
                }
                _ => {}
            }
            let b = run(&e["right"], false, next, resolver)?;
            match op {
                "==" | "===" => Ok(json!(equal(&a, &b))),
                "!=" | "!==" => Ok(json!(!equal(&a, &b))),
                "<" | "<=" | ">" | ">=" => {
                    let o = compare(&a, &b).ok_or_else(|| {
                        Diagnostic::new(
                            "type-mismatch",
                            "Сравнение ожидает два числа или две строки",
                        )
                    })?;
                    Ok(json!(match op {
                        "<" => o.is_lt(),
                        "<=" => !o.is_gt(),
                        ">" => o.is_gt(),
                        _ => !o.is_lt(),
                    }))
                }
                ".." | "..=" => {
                    let st = number(&a)? as i64;
                    let en = number(&b)? as i64;
                    let inclusive = op == "..=";
                    let count = if inclusive { en - st + 1 } else { en - st };
                    if count <= 0 {
                        return Ok(json!([]));
                    }
                    if count > 4096 {
                        return Err(Diagnostic::new(
                            "size-limit",
                            "Превышен предел размера диапазона (макс. 4096)",
                        ));
                    }
                    let items: Vec<V> = if inclusive {
                        (st..=en).map(|i| json!(i)).collect()
                    } else {
                        (st..en).map(|i| json!(i)).collect()
                    };
                    Ok(V::Array(items))
                }
                "+" if a.is_string() && b.is_string() => Ok(json!(format!(
                    "{}{}",
                    a.as_str().unwrap(),
                    b.as_str().unwrap()
                ))),
                _ => {
                    let a = number(&a)?;
                    let b = number(&b)?;
                    numeric(match op {
                        "+" => a + b,
                        "-" => a - b,
                        "*" => a * b,
                        "/" => a / b,
                        "%" => a % b,
                        _ => {
                            return Err(Diagnostic::new(
                                "unknown-operator",
                                format!("Неизвестный оператор {op}"),
                            ));
                        }
                    })
                }
            }
        }
        _ => Err(Diagnostic::new(
            "unknown-expression",
            format!("Неизвестное выражение {k}"),
        )),
    }
}
pub fn references(v: &V) -> Vec<String> {
    fn walk<'a>(v: &'a V, seen: &mut std::collections::HashSet<&'a str>, out: &mut Vec<String>) {
        if let Some(s) = v["expr"].as_str() {
            let s = s.strip_prefix('!').unwrap_or(s);
            if s.contains('.')
                && s.split('.').all(|s| {
                    let mut cs = s.chars();
                    cs.next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
                && seen.insert(s)
            {
                out.push(s.into())
            }
        }
        if let Some(o) = v.as_object() {
            for (k, v) in o {
                if k != "expr" {
                    walk(v, seen, out)
                }
            }
        }
        if let Some(a) = v.as_array() {
            for v in a {
                walk(v, seen, out)
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk(v, &mut seen, &mut out);
    out
}
