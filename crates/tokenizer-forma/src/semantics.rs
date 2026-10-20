//! Component expression scope and typed contracts, independent of UI rendering.
use crate::evaluate;
use serde_json::{Value as V, json};
type R<T> = Result<T, String>;
fn own(v: &V, k: &str) -> bool {
    v.as_object().is_some_and(|o| o.contains_key(k))
}
#[inline]
fn read_path<'a>(
    mut value: &'a V,
    parts: impl IntoIterator<Item = &'a str>,
    path: &str,
    optional: bool,
) -> R<V> {
    for part in parts {
        let next = if let Some(a) = value.as_array() {
            part.parse::<usize>().ok().and_then(|i| a.get(i))
        } else {
            value.as_object().and_then(|o| o.get(part))
        };
        match next {
            Some(v) => value = v,
            None if optional => return Ok(V::Null),
            None => return Err(format!("Нет значения {path}")),
        }
    }
    Ok(value.clone())
}
pub fn evaluate_value(
    value: &V,
    props: &V,
    state: &V,
    stack: &[String],
    expand: bool,
    env: &V,
) -> R<V> {
    if stack.len() > 128 {
        return Err("Превышен предел вложенности".into());
    }
    if value.get("type").is_some() {
        if !expand {
            return Ok(value.clone());
        }
        let mut out = value.clone();
        out["props"] = evaluate_properties(value, props, state, env)?;
        out["children"] = V::Array(
            value["children"]
                .as_array()
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|c| evaluate_value(c, props, state, stack, true, env))
                .collect::<R<_>>()?,
        );
        return Ok(out);
    }
    if let Some(subject) = value.get("match") {
        let actual = evaluate_value(subject, props, state, stack, expand, env)?;
        for b in value["branches"]
            .as_array()
            .ok_or("Некорректные ветки match")?
        {
            if pattern(&b["pattern"], &actual, props, state, env)? {
                if let Some(g) = b.get("guard") {
                    let cond = evaluate_value(g, props, state, stack, false, env)?;
                    if !evaluate::truth(&cond)? {
                        continue;
                    }
                }
                return evaluate_value(&b["value"], props, state, stack, expand, env);
            }
        }
        return Err("match: нет подходящей ветки; добавьте _ => …".into());
    }
    if let Some(a) = value.as_array() {
        return Ok(V::Array(
            a.iter()
                .map(|v| evaluate_value(v, props, state, stack, expand, env))
                .collect::<R<_>>()?,
        ));
    }
    if let Some(o) = value.as_object()
        && !own(value, "expr")
        && !own(value, "expression")
        && !own(value, "interpolation")
    {
        let mut out = serde_json::Map::with_capacity(o.len());
        for (k, v) in o {
            out.insert(
                k.clone(),
                evaluate_value(v, props, state, stack, expand, env)?,
            );
        }
        return Ok(V::Object(out));
    }
    evaluate::evaluate(value, &mut |raw, optional| {
        if let Some(tail) = raw.strip_prefix("viewport.") {
            let vp = &env["viewport"];
            if vp.is_object() {
                return read_path(vp, tail.split('.'), raw, optional);
            }
            return match tail {
                "width" => Ok(json!(360)),
                "height" => Ok(json!(220)),
                "compact" => Ok(V::Bool(true)),
                _ if optional => Ok(V::Null),
                _ => Err(format!("Нет значения {raw}")),
            };
        }
        let (head_local, rest_local) = raw.split_once('.').unwrap_or((raw, ""));
        if !matches!(head_local, "props" | "state" | "tokens" | "i18n" | "base")
            && let Some(value) = env["locals"].get(head_local)
        {
            return if rest_local.is_empty() {
                Ok(value.clone())
            } else {
                read_path(value, rest_local.split('.'), raw, optional)
            };
        }
        if let Some(tail) = raw.strip_prefix("props.") {
            let (key, rest) = if tail == "font.size" {
                ("fontSize", None)
            } else if own(props, tail) {
                (tail, None)
            } else {
                match tail.split_once('.') {
                    Some((head, sub)) => (head, Some(sub)),
                    None => (tail, None),
                }
            };
            if let Some(local_val) = env["locals"].get(key) {
                return match rest {
                    Some(sub) => read_path(local_val, sub.split('.'), raw, optional),
                    None => Ok(local_val.clone()),
                };
            }
            let Some(value) = props.get(key) else {
                if optional {
                    return Ok(V::Null);
                }
                return Err(format!("Неизвестное свойство {raw}"));
            };
            if stack.iter().any(|s| s == key) {
                let mut cycle = stack.to_vec();
                cycle.push(key.into());
                return Err(format!("Циклическая ссылка props: {}", cycle.join(" → ")));
            }
            let mut next = Vec::with_capacity(stack.len() + 1);
            next.extend_from_slice(stack);
            next.push(key.into());
            let evaluated = evaluate_value(value, props, state, &next, expand, env)?;
            return match rest {
                Some(sub) => read_path(&evaluated, sub.split('.'), raw, optional),
                None => Ok(evaluated),
            };
        }
        if let Some(tail) = raw.strip_prefix("state.") {
            if let Ok(val) = read_path(state, tail.split('.'), raw, false) {
                return Ok(val);
            }
            if env["stateDefaults"].is_object()
                && let Ok(val) = read_path(&env["stateDefaults"], tail.split('.'), raw, false)
            {
                return evaluate_value(&val, props, state, stack, expand, env);
            }
            return read_path(
                state,
                tail.split('.'),
                raw,
                optional || env["allowMissingState"] == true,
            );
        }
        if let Some(tail) = raw.strip_prefix("tokens.") {
            let tokens = &env["tokens"];
            if own(tokens, tail) {
                return evaluate_value(&tokens[tail], props, state, stack, expand, env);
            }
            return read_path(tokens, tail.split('.'), raw, optional);
        }
        if let Some(tail) = raw.strip_prefix("i18n.") {
            let i18n = &env["i18n"];
            if let Some(locale) = env["locale"].as_str()
                && let Some(loc_dict) = i18n.get(locale)
                && loc_dict.is_object()
            {
                if own(loc_dict, tail) {
                    return evaluate_value(&loc_dict[tail], props, state, stack, expand, env);
                }
                if let Ok(val) = read_path(loc_dict, tail.split('.'), raw, false) {
                    return evaluate_value(&val, props, state, stack, expand, env);
                }
            }
            if own(i18n, tail) {
                return evaluate_value(&i18n[tail], props, state, stack, expand, env);
            }
            let val = read_path(i18n, tail.split('.'), raw, optional)?;
            return evaluate_value(&val, props, state, stack, expand, env);
        }
        if raw.starts_with("base.") {
            return Err(format!("{raw} допустим только внутри override"));
        }
        let (head, rest) = raw.split_once('.').unwrap_or((raw, ""));
        if let Some(variants) = env["enums"].get(head).and_then(V::as_array) {
            if rest.is_empty()
                || rest.contains('.')
                || !variants.iter().any(|v| v.as_str() == Some(rest))
            {
                return Err(format!("Неизвестный вариант {raw}"));
            }
            return Ok(json!(raw));
        }
        Ok(json!({"expr":raw}))
    })
}
fn pattern(p: &V, v: &V, props: &V, state: &V, env: &V) -> R<bool> {
    evaluate::matches_pattern(p, v, &mut |p| {
        evaluate_value(p, props, state, &[], false, env)
    })
}
pub fn selected_properties(groups: &V, props: &V, state: &V, env: &V) -> R<V> {
    let mut out = json!({});
    if let Some(groups) = groups.as_array() {
        for g in groups {
            let actual = evaluate_value(&g["match"], props, state, &[], false, env)?;
            let mut found = false;
            for b in g["branches"].as_array().ok_or("Некорректные ветки match")? {
                if pattern(&b["pattern"], &actual, props, state, env)? {
                    if let Some(guard) = b.get("guard") {
                        let cond = evaluate_value(guard, props, state, &[], false, env)?;
                        if !evaluate::truth(&cond)? {
                            continue;
                        }
                    }
                    for (k, v) in b["props"]
                        .as_object()
                        .ok_or("Некорректные свойства match")?
                    {
                        if own(&out, k) {
                            return Err(format!("Несколько групп match задают {k}"));
                        }
                        out[k] = v.clone()
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                return Err("match: нет подходящей ветки; добавьте _ => …".into());
            }
        }
    }
    Ok(out)
}
pub fn selected_sources(groups: &V, props: &V, state: &V, env: &V) -> R<V> {
    let mut out = json!({});
    if let Some(groups) = groups.as_array() {
        for g in groups {
            let actual = evaluate_value(&g["match"], props, state, &[], false, env)?;
            for b in g["branches"].as_array().ok_or("Некорректные ветки match")? {
                if pattern(&b["pattern"], &actual, props, state, env)? {
                    if let Some(guard) = b.get("guard") {
                        let cond = evaluate_value(guard, props, state, &[], false, env)?;
                        if !evaluate::truth(&cond)? {
                            continue;
                        }
                    }
                    if let Some(o) = b["propertySources"].as_object() {
                        for (k, v) in o {
                            out[k] = v.clone()
                        }
                    }
                    break;
                }
            }
        }
    }
    Ok(out)
}
pub fn evaluate_properties(node: &V, props: &V, state: &V, env: &V) -> R<V> {
    let mut values = serde_json::Map::new();
    if let Some(applies) = node["apply"].as_array() {
        for style_val in applies {
            let style_name = style_val.as_str().ok_or("Некорректный apply")?;
            let Some(style_props) = env["styles"].get(style_name).and_then(V::as_object) else {
                return Err(format!("Неизвестный стиль {style_name}"));
            };
            for (k, v) in style_props {
                values.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(keys) = node["forward"].as_array() {
        for k in keys {
            let k = k.as_str().ok_or("Некорректный forward")?;
            if !own(props, k) {
                return Err(format!("forward: неизвестное свойство props.{k}"));
            }
            values.insert(k.to_owned(), json!({"expr":format!("props.{k}")}));
        }
    }
    if let V::Object(matched) = selected_properties(&node["matches"], props, state, env)? {
        for (k, v) in matched {
            values.insert(k, v);
        }
    }
    if let Some(o) = node["props"].as_object() {
        for (k, v) in o {
            values.insert(k.clone(), v.clone());
        }
    }
    if let Some(bindings) = node["bindings"].as_object() {
        for (k, v) in bindings {
            if values.contains_key(k) {
                return Err(format!(
                    "Свойство {k} одновременно имеет значение и двустороннюю привязку"
                ));
            }
            values.insert(k.clone(), json!({"expr":v}));
        }
    }
    let mut out = serde_json::Map::with_capacity(values.len());
    for (k, v) in &values {
        out.insert(k.clone(), evaluate_value(v, props, state, &[], false, env)?);
    }
    Ok(V::Object(out))
}
fn split_top_level(s: &str, sep: u8) -> Option<Vec<&str>> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut last = 0;
    let mut parts = Vec::new();
    for (idx, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            _ if b == sep && depth == 0 => {
                parts.push(s[last..idx].trim());
                last = idx + 1;
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(s[last..].trim());
        Some(parts)
    }
}
pub fn validate_type_name(ty: &str, enums: &V) -> R<()> {
    let ty = ty.trim();
    if let Some(parts) = split_top_level(ty, b'|') {
        for p in parts {
            validate_type_name(p, enums)?;
        }
        return Ok(());
    }
    let name = ty.strip_suffix('?').unwrap_or(ty).trim();
    if let Some(inner) = name.strip_prefix("List<").and_then(|s| s.strip_suffix('>')) {
        return validate_type_name(inner, enums);
    }
    if let Some(inner) = name.strip_prefix("Map<").and_then(|s| s.strip_suffix('>')) {
        return validate_type_name(inner, enums);
    }
    if let Some(inner) = name
        .strip_prefix("Record<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let Some(parts) = split_top_level(inner, b',') else {
            return Err(format!("Неизвестный тип {ty}"));
        };
        if parts.len() != 2 {
            return Err(format!("Неизвестный тип {ty}"));
        }
        validate_type_name(parts[0], enums)?;
        return validate_type_name(parts[1], enums);
    }
    if ![
        "String", "Bool", "Boolean", "Number", "Int", "Float", "Color", "Length", "Duration",
        "Angle", "Asset", "List", "Object", "Map", "Any",
    ]
    .contains(&name)
        && !own(enums, name)
    {
        return Err(format!("Неизвестный тип {ty}"));
    }
    Ok(())
}
fn unit(s: &str, suffix: &str) -> bool {
    s.strip_suffix(suffix).is_some_and(|n| {
        !n.is_empty()
            && n.split('.').count() <= 2
            && n.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    })
}
fn unit_signed(s: &str, suffix: &str) -> bool {
    let s = s
        .strip_prefix('-')
        .or_else(|| s.strip_prefix('+'))
        .unwrap_or(s);
    unit(s, suffix)
}
pub fn matches_type(value: &V, ty: &str, enums: &V) -> bool {
    let ty = ty.trim();
    if let Some(parts) = split_top_level(ty, b'|') {
        return parts.into_iter().any(|p| matches_type(value, p, enums));
    }
    if let Some(ty) = ty.strip_suffix('?') {
        return value.is_null() || matches_type(value, ty.trim(), enums);
    }
    if let Some(inner) = ty.strip_prefix("List<").and_then(|s| s.strip_suffix('>')) {
        return value
            .as_array()
            .is_some_and(|a| a.iter().all(|v| matches_type(v, inner, enums)));
    }
    if let Some(inner) = ty.strip_prefix("Map<").and_then(|s| s.strip_suffix('>')) {
        return value
            .as_object()
            .is_some_and(|o| o.values().all(|v| matches_type(v, inner, enums)));
    }
    if let Some(inner) = ty.strip_prefix("Record<").and_then(|s| s.strip_suffix('>')) {
        let Some(parts) = split_top_level(inner, b',') else {
            return false;
        };
        if parts.len() != 2 {
            return false;
        }
        let val_ty = parts[1];
        return value
            .as_object()
            .is_some_and(|o| o.values().all(|v| matches_type(v, val_ty, enums)));
    }
    if let Some(a) = enums[ty].as_array() {
        return value.as_str().is_some_and(|s| {
            s.strip_prefix(ty)
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|variant| a.iter().any(|v| v.as_str() == Some(variant)))
        });
    }
    if let Some(fields) = enums[ty].as_object() {
        return value.as_object().is_some_and(|obj| {
            fields.iter().all(|(field, fty)| {
                let Some(fty) = fty.as_str() else {
                    return false;
                };
                match obj.get(field) {
                    Some(v) => matches_type(v, fty, enums),
                    None => fty.ends_with('?'),
                }
            })
        });
    }
    let raw = value.get("expr").unwrap_or(value);
    match ty {
        "Any" => true,
        "String" | "Asset" => value.is_string(),
        "Bool" | "Boolean" => value.is_boolean(),
        "Number" | "Float" => value.as_f64().is_some_and(f64::is_finite),
        "Int" => value
            .as_f64()
            .is_some_and(|n| n.is_finite() && n.fract() == 0. && n.abs() <= 9007199254740991.),
        "Color" => raw.as_str().is_some_and(|s| {
            s.strip_prefix('#').is_some_and(|s| {
                [3, 6, 8].contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
            })
        }),
        "Length" => {
            raw.as_f64().is_some_and(|n| n.is_finite() && n >= 0.)
                || raw.as_str().is_some_and(|s| {
                    ["px", "%", "rem", "em", "vw", "vh", "ch", "fr"]
                        .iter()
                        .any(|u| unit(s, u))
                })
        }
        "Duration" => raw.as_str().is_some_and(|s| unit(s, "ms") || unit(s, "s")),
        "Angle" => {
            raw.as_f64().is_some_and(f64::is_finite)
                || raw
                    .as_str()
                    .is_some_and(|s| ["deg", "rad", "turn"].iter().any(|u| unit_signed(s, u)))
        }
        "List" => value.is_array(),
        "Object" | "Map" => value.is_object(),
        _ => false,
    }
}
fn check_constraints(val: &V, constraints: &V, path_label: &str) -> R<()> {
    if val.is_null() {
        return Ok(());
    }
    if let Some(range) = constraints["range"].as_array()
        && range.len() == 2
    {
        let min_v = evaluate::evaluate(&range[0], &mut |_, _| Ok(V::Null))?;
        let max_v = evaluate::evaluate(&range[1], &mut |_, _| Ok(V::Null))?;
        if let (Some(min), Some(max)) = (min_v.as_f64(), max_v.as_f64()) {
            if let Some(n) = val.as_f64() {
                if n < min || n > max {
                    return Err(format!(
                        "{path_label}: значение {n} вне допустимого диапазона {min}..={max}"
                    ));
                }
            } else if let Some(s) = val.as_str() {
                let len = s.chars().count() as f64;
                if len < min || len > max {
                    return Err(format!(
                        "{path_label}: длина строки {len} вне допустимого диапазона {min}..={max}"
                    ));
                }
            } else if let Some(a) = val.as_array() {
                let len = a.len() as f64;
                if len < min || len > max {
                    return Err(format!(
                        "{path_label}: длина списка {len} вне допустимого диапазона {min}..={max}"
                    ));
                }
            }
        }
    }
    if let Some(pat) = constraints["pattern"].as_str() {
        let s = evaluate::text(val)?;
        if !evaluate::matches_constraint_pattern(&s, pat) {
            return Err(format!(
                "{path_label}: значение не соответствует шаблону '{pat}'"
            ));
        }
    }
    Ok(())
}
pub fn validate_contract(defs: &V, values: &V, enums: &V, label: &str) -> R<()> {
    if let Some(o) = defs.as_object() {
        for (name, d) in o {
            let ty = d["type"].as_str().ok_or("Некорректный тип")?;
            validate_type_name(ty, enums)?;
            if !own(values, name) {
                if d["required"] == true {
                    return Err(format!("{label}: обязательное свойство {name} не задано"));
                }
                continue;
            }
            if !matches_type(&values[name], ty, enums) {
                return Err(format!("{label}.{name}: ожидался {ty}"));
            }
            if d["constraints"].is_object() {
                check_constraints(&values[name], &d["constraints"], &format!("{label}.{name}"))?;
            }
            let base_ty = ty.strip_suffix('?').unwrap_or(ty);
            if let Some(struct_cons) = enums["_structConstraints"]
                .get(base_ty)
                .and_then(V::as_object)
                && let Some(obj) = values[name].as_object()
            {
                for (field, fcons) in struct_cons {
                    if let Some(fval) = obj.get(field) {
                        check_constraints(fval, fcons, &format!("{label}.{name}.{field}"))?;
                    }
                }
            }
        }
    }
    Ok(())
}
/// Expand conditional/keyed markup with a bounded output size. The host hook
/// supplies source annotations and controls traversal at component boundaries.
pub fn expand_structure<F>(
    nodes: &V,
    props: &V,
    state: &V,
    environment: &V,
    options: &V,
    hook: &mut F,
) -> R<V>
where
    F: FnMut(&V, &V) -> R<V>,
{
    struct Expansion<'a, F> {
        props: &'a V,
        state: &'a V,
        options: &'a V,
        hook: &'a mut F,
        count: usize,
    }
    impl<F: FnMut(&V, &V) -> R<V>> Expansion<'_, F> {
        fn walk(&mut self, nodes: &V, env: &V, prefix: &str, depth: usize) -> R<Vec<V>> {
            if depth > 64 {
                return Err("Превышен предел глубины разметки".into());
            }
            let empty_nodes = V::Array(Vec::new());
            let mut out = Vec::new();
            let mut current_env = std::borrow::Cow::Borrowed(env);
            if !current_env.is_object() {
                current_env = std::borrow::Cow::Owned(json!({"locals": {}}));
            } else if !current_env["locals"].is_object() {
                current_env.to_mut()["locals"] = json!({});
            }
            for (index, node) in nodes
                .as_array()
                .ok_or("Ожидался список узлов")?
                .iter()
                .enumerate()
            {
                self.count += 1;
                if self.count > 16384 {
                    return Err("Превышен предел размера разметки".into());
                }
                if node["type"] == "Let" {
                    let name = node["name"].as_str().ok_or("Некорректный let")?;
                    let val = evaluate_value(
                        &node["value"],
                        self.props,
                        self.state,
                        &[],
                        false,
                        &current_env,
                    )?;
                    current_env.to_mut()["locals"][name] = val;
                    continue;
                }
                if node["type"] == "If" {
                    let c = evaluate_value(
                        &node["condition"],
                        self.props,
                        self.state,
                        &[],
                        false,
                        &current_env,
                    )?;
                    if !c.is_boolean() {
                        return Err("if ожидает Bool".into());
                    }
                    out.extend(self.walk(
                        if c == true {
                            &node["children"]
                        } else {
                            node.get("elseChildren").unwrap_or(&empty_nodes)
                        },
                        &current_env,
                        prefix,
                        depth + 1,
                    )?);
                    continue;
                }
                if node["type"] == "Match" {
                    let actual = evaluate_value(
                        &node["match"],
                        self.props,
                        self.state,
                        &[],
                        false,
                        &current_env,
                    )?;
                    let mut found = false;
                    for b in node["branches"]
                        .as_array()
                        .ok_or("Некорректные ветки match")?
                    {
                        if pattern(&b["pattern"], &actual, self.props, self.state, &current_env)? {
                            if let Some(guard) = b.get("guard") {
                                let cond = evaluate_value(
                                    guard,
                                    self.props,
                                    self.state,
                                    &[],
                                    false,
                                    &current_env,
                                )?;
                                if !evaluate::truth(&cond)? {
                                    continue;
                                }
                            }
                            out.extend(self.walk(
                                b.get("children").unwrap_or(&empty_nodes),
                                &current_env,
                                prefix,
                                depth + 1,
                            )?);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Err("match: нет подходящей ветки; добавьте _ => …".into());
                    }
                    continue;
                }
                if node["type"] == "For" {
                    let item = node["item"].as_str().ok_or("Некорректный for")?;
                    if own(&current_env["locals"], item) {
                        return Err(format!("Вложенный for повторяет имя {item}"));
                    }
                    let idx_name = node["index"].as_str();
                    if let Some(idx) = idx_name
                        && own(&current_env["locals"], idx)
                    {
                        return Err(format!("Вложенный for повторяет имя {idx}"));
                    }
                    let collection = evaluate_value(
                        &node["collection"],
                        self.props,
                        self.state,
                        &[],
                        false,
                        &current_env,
                    )?;
                    let collection = collection.as_array().ok_or("for ожидает коллекцию")?;
                    let mut keys = std::collections::HashSet::with_capacity(collection.len());
                    let mut local = current_env.as_ref().clone();
                    if !local["_formaBindings"].is_object() {
                        local["_formaBindings"] = json!({});
                    }
                    let node_start = node["start"].as_u64().unwrap_or(index as u64);
                    let mut matched_any = false;
                    for (item_index, v) in collection.iter().enumerate() {
                        local["locals"][item] = v.clone();
                        if let Some(idx) = idx_name {
                            local["locals"][idx] = json!(item_index);
                        }
                        if let Some(where_expr) = node.get("where") {
                            let cond = evaluate_value(
                                where_expr,
                                self.props,
                                self.state,
                                &[],
                                false,
                                &local,
                            )?;
                            if !evaluate::truth(&cond)? {
                                continue;
                            }
                        }
                        matched_any = true;
                        local["_formaBindings"][item] =
                            json!({"collection":node["collection"],"index":item_index});
                        let key = evaluate_value(
                            &node["key"],
                            self.props,
                            self.state,
                            &[],
                            false,
                            &local,
                        )?;
                        if !key.is_string() && !key.as_f64().is_some_and(f64::is_finite) {
                            return Err("Ключ for должен быть строкой или конечным числом".into());
                        }
                        let identity = if key.is_number() {
                            evaluate::text(&key)?
                        } else {
                            key.to_string()
                        };
                        if !keys.insert(identity.clone()) {
                            return Err(format!("Повторный ключ for: {identity}"));
                        }
                        out.extend(self.walk(
                            &node["children"],
                            &local,
                            &format!("{prefix}@{node_start}:{identity}/"),
                            depth + 1,
                        )?)
                    }
                    if !matched_any {
                        out.extend(self.walk(
                            node.get("emptyChildren").unwrap_or(&empty_nodes),
                            &current_env,
                            prefix,
                            depth + 1,
                        )?)
                    }
                    continue;
                }
                let values = if self.options["evaluateProps"] == false {
                    node["props"].clone()
                } else {
                    evaluate_properties(node, self.props, self.state, &current_env)?
                };
                let mut args = json!({});
                if let Some(o) = node["eventArgs"].as_object() {
                    for (k, a) in o {
                        args[k] = V::Array(
                            a.as_array()
                                .ok_or("Некорректные аргументы события")?
                                .iter()
                                .map(|v| {
                                    evaluate_value(
                                        v,
                                        self.props,
                                        self.state,
                                        &[],
                                        false,
                                        &current_env,
                                    )
                                })
                                .collect::<R<_>>()?,
                        )
                    }
                }
                let annotation = (self.hook)(node, &current_env)?;
                let is_slider_num = node["type"] == "Slider" && values["value"].is_number();
                let mut result = if let Some(node_obj) = node.as_object() {
                    let mut map = serde_json::Map::with_capacity(node_obj.len() + 4);
                    for (k, v) in node_obj {
                        if k != "props" && k != "children" && k != "eventArgs" {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                    V::Object(map)
                } else {
                    node.clone()
                };
                result["props"] = values;
                result["eventArgs"] = args;
                result["eventArgExpressions"] = node.get("eventArgs").cloned().unwrap_or(json!({}));
                result["environment"] = current_env.as_ref().clone();
                if let Some(v) = annotation.get("propertyOrigins") {
                    result["propertyOrigins"] = v.clone()
                }
                if is_slider_num {
                    let path = node["props"]["value"]["expr"].as_str();
                    result["normalizedRange"] = json!(
                        node["bindings"]["value"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                            || path.is_some_and(|s| s.starts_with("state.")
                                || own(&current_env["locals"], s.split('.').next().unwrap_or("")))
                    )
                }
                if !prefix.is_empty() {
                    let key_str =
                        if let Some(k) = result["props"].get("key").filter(|v| !v.is_null()) {
                            evaluate::text(k)?
                        } else if let Some(start_v) = node.get("start") {
                            evaluate::text(start_v)?
                        } else {
                            index.to_string()
                        };
                    result["props"]["key"] = json!(format!("{prefix}{key_str}"))
                }
                result["children"] =
                    if self.options["recursive"] == false || annotation["recursive"] == false {
                        node["children"].clone()
                    } else {
                        V::Array(self.walk(
                            node.get("children").unwrap_or(&empty_nodes),
                            &current_env,
                            prefix,
                            depth + 1,
                        )?)
                    };
                out.push(result);
            }
            Ok(out)
        }
    }
    let mut ex = Expansion {
        props,
        state,
        options,
        hook,
        count: 0,
    };
    Ok(V::Array(ex.walk(nodes, environment, "", 0)?))
}
pub fn validate_design(component: &V, design: &V) -> R<()> {
    if design["name"] != component["name"] {
        return Err(format!(
            "Дизайн {} не соответствует {}",
            design["name"].as_str().unwrap_or(""),
            component["name"].as_str().unwrap_or("")
        ));
    }
    fn collect(nodes: &V, keys: &mut std::collections::HashMap<String, String>) {
        if let Some(a) = nodes.as_array() {
            for n in a {
                if let (Some(key), Some(ty)) = (n["props"]["key"].as_str(), n["type"].as_str()) {
                    keys.insert(key.into(), ty.into());
                }
                for k in ["children", "elseChildren", "emptyChildren"] {
                    collect(&n[k], keys)
                }
                if let Some(branches) = n["branches"].as_array() {
                    for b in branches {
                        collect(&b["children"], keys);
                    }
                }
            }
        }
    }
    let mut keys = std::collections::HashMap::new();
    collect(&component["nodes"], &mut keys);
    let check = |d: &V, where_: &str| -> R<()> {
        if let Some(o) = d["overrides"].as_object() {
            for key in o.keys() {
                let Some(ty) = keys.get(key) else {
                    return Err(format!("Неизвестный дизайн-key {key}"));
                };
                let got = d["overrideTypes"][key].as_str().unwrap_or("");
                if got != ty {
                    return Err(format!(
                        "Тип дизайн-key {key}: ожидался {ty}, получен {got}{where_}"
                    ));
                }
            }
        }
        Ok(())
    };
    check(design, "")?;
    if let Some(a) = design["states"].as_array() {
        for s in a {
            check(
                s,
                &format!(" в состоянии {}", s["name"].as_str().unwrap_or("")),
            )?
        }
    }
    Ok(())
}
pub fn design_state_patch(design: &V, state: Option<&str>) -> R<V> {
    let mut out = design.get("overrides").cloned().unwrap_or(json!({}));
    if let Some(name) = state {
        let state = design["states"]
            .as_array()
            .and_then(|a| a.iter().find(|s| s["name"] == name))
            .ok_or_else(|| format!("Состояние {name} не объявлено"))?;
        if let Some(o) = state["overrides"].as_object() {
            for (k, props) in o {
                if !out[k].is_object() {
                    out[k] = json!({})
                }
                if let Some(p) = props.as_object() {
                    for (k2, v) in p {
                        out[k][k2] = v.clone()
                    }
                }
            }
        }
    }
    Ok(out)
}
