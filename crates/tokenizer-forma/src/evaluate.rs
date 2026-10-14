//! Bounded pure expression evaluation. References are resolved by the host;
//! there is no host-language eval or access to arbitrary object properties.
use serde_json::{Value as V, json};
type R<T> = Result<T, String>;
pub fn truth(v: &V) -> R<bool> {
    v.as_bool()
        .ok_or_else(|| "Логическое выражение ожидает Bool".into())
}
fn number(v: &V) -> R<f64> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| "Арифметика ожидает конечное число".into())
}
fn numeric(n: f64) -> R<V> {
    if n.is_finite() {
        Ok(json!(n))
    } else {
        Err("Арифметика ожидает конечное число".into())
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
        _ => Err("Строковое преобразование ожидает скалярное значение".into()),
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
pub fn matches_pattern<F>(p: &V, v: &V, evaluate: &mut F) -> R<bool>
where
    F: FnMut(&V) -> R<V>,
{
    if p["expr"] == "_" {
        return Ok(true);
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
        return Err("Превышен предел вложенности выражения".into());
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
        for branch in v["branches"].as_array().ok_or("Некорректные ветки match")? {
            if matches_pattern(&branch["pattern"], &actual, &mut |p| {
                run(p, false, next, resolver)
            })? {
                return run(&branch["value"], false, next, resolver);
            }
        }
        return Err("match: нет подходящей ветки; добавьте _ => …".into());
    }
    let Some(e) = v.get("expression") else {
        let mut o = serde_json::Map::new();
        for (k, v) in v.as_object().unwrap() {
            o.insert(k.clone(), run(v, false, next, resolver)?);
        }
        return Ok(V::Object(o));
    };
    let k = e["kind"].as_str().ok_or("Некорректное выражение")?;
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
                    Err("Нельзя прочитать свойство пустого значения; используйте ?.".into())
                };
            }
            let p = if e["property"].is_string() {
                e["property"].clone()
            } else {
                run(&e["property"], false, next, resolver)?
            };
            let key = text(&p)?;
            if ["__proto__", "prototype", "constructor"].contains(&key.as_str()) {
                return Err(format!("Недоступное свойство {key}"));
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
                object.as_object().and_then(|o| o.get(&key))
            };
            match value {
                Some(v) => Ok(v.clone()),
                None if optional || opt => Ok(V::Null),
                _ => Err(format!("Неизвестное свойство {key}")),
            }
        }
        "call" => {
            let name = e["name"].as_str().ok_or("Некорректный вызов")?;
            let args = e["args"]
                .as_array()
                .ok_or("Некорректные аргументы")?
                .iter()
                .map(|v| run(v, false, next, resolver))
                .collect::<R<Vec<_>>>()?;
            let count = args.len();
            let valid = match name {
                "min" | "max" => count >= 1,
                "clamp" => count == 3,
                _ => count == 1,
            };
            if !valid {
                return Err(format!("{name}: неверное число аргументов"));
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
                            return Err("Number ожидает число или непустую числовую строку".into());
                        }
                        s.parse::<f64>()
                            .map_err(|_| "Number ожидает число или непустую числовую строку")?
                    } else {
                        return Err("Number ожидает число или непустую числовую строку".into());
                    };
                    numeric(n)
                }
                "len" => {
                    if let Some(s) = args[0].as_str() {
                        Ok(json!(s.encode_utf16().count()))
                    } else if let Some(a) = args[0].as_array() {
                        Ok(json!(a.len()))
                    } else {
                        Err("len ожидает строку или массив".into())
                    }
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
                        return Err("clamp: минимум превышает максимум".into());
                    }
                    numeric(a.max(lo).min(hi))
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
                _ => Err(format!("Неизвестная чистая функция {name}")),
            }
        }
        "binary" => {
            let op = e["operator"].as_str().ok_or("Некорректный оператор")?;
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
                    let o = compare(&a, &b).ok_or("Сравнение ожидает два числа или две строки")?;
                    Ok(json!(match op {
                        "<" => o.is_lt(),
                        "<=" => !o.is_gt(),
                        ">" => o.is_gt(),
                        _ => !o.is_lt(),
                    }))
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
                        _ => return Err(format!("Неизвестный оператор {op}")),
                    })
                }
            }
        }
        _ => Err(format!("Неизвестное выражение {k}")),
    }
}
pub fn references(v: &V) -> Vec<String> {
    fn walk(v: &V, out: &mut Vec<String>) {
        if let Some(s) = v["expr"].as_str() {
            let s = s.strip_prefix('!').unwrap_or(s);
            if s.contains('.')
                && s.split('.').all(|s| {
                    let mut cs = s.chars();
                    cs.next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
                && !out.iter().any(|x| x == s)
            {
                out.push(s.into())
            }
        }
        if let Some(o) = v.as_object() {
            for (k, v) in o {
                if k != "expr" {
                    walk(v, out)
                }
            }
        }
        if let Some(a) = v.as_array() {
            for v in a {
                walk(v, out)
            }
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}
