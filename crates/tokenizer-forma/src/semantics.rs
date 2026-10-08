//! Component expression scope and typed contracts, independent of UI rendering.
use crate::evaluate;
use serde_json::{Value as V, json};
type R<T> = Result<T, String>;
fn own(v: &V, k: &str) -> bool {
    v.as_object().is_some_and(|o| o.contains_key(k))
}
fn read_path(mut value: V, parts: &[&str], path: &str, optional: bool) -> R<V> {
    for part in parts {
        let next = if let Some(a) = value.as_array() {
            part.parse::<usize>().ok().and_then(|i| a.get(i))
        } else {
            value.as_object().and_then(|o| o.get(*part))
        };
        value = match next {
            Some(v) => v.clone(),
            None if optional => return Ok(V::Null),
            None => return Err(format!("Нет значения {path}")),
        };
    }
    Ok(value)
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
                .unwrap_or(&vec![])
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
        let mut out = serde_json::Map::new();
        for (k, v) in o {
            out.insert(
                k.clone(),
                evaluate_value(v, props, state, stack, expand, env)?,
            );
        }
        return Ok(V::Object(out));
    }
    evaluate::evaluate(value, &mut |raw, optional| {
        if let Some(tail) = raw.strip_prefix("props.") {
            let parts: Vec<_> = if tail == "font.size" {
                vec!["fontSize"]
            } else if own(props, tail) {
                vec![tail]
            } else {
                tail.split('.').collect()
            };
            let key = parts[0];
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
            let mut next = stack.to_vec();
            next.push(key.into());
            return read_path(
                evaluate_value(value, props, state, &next, expand, env)?,
                &parts[1..],
                raw,
                optional,
            );
        }
        if let Some(tail) = raw.strip_prefix("state.") {
            return read_path(
                state.clone(),
                &tail.split('.').collect::<Vec<_>>(),
                raw,
                optional || env["allowMissingState"] == true,
            );
        }
        if raw.starts_with("base.") {
            return Err(format!("{raw} допустим только внутри override"));
        }
        let parts: Vec<_> = raw.split('.').collect();
        if let Some(value) = env["locals"].get(parts[0]) {
            return read_path(value.clone(), &parts[1..], raw, optional);
        }
        if let Some(variants) = env["enums"].get(parts[0]) {
            if parts.len() != 2
                || !variants
                    .as_array()
                    .is_some_and(|a| a.contains(&json!(parts[1])))
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
    let mut values = json!({});
    if let Some(keys) = node["forward"].as_array() {
        for k in keys {
            let k = k.as_str().ok_or("Некорректный forward")?;
            if !own(props, k) {
                return Err(format!("forward: неизвестное свойство props.{k}"));
            }
            values[k] = json!({"expr":format!("props.{k}")})
        }
    }
    for map in [
        selected_properties(&node["matches"], props, state, env)?,
        node["props"].clone(),
    ] {
        if let Some(o) = map.as_object() {
            for (k, v) in o {
                values[k] = v.clone()
            }
        }
    }
    if let Some(bindings) = node["bindings"].as_object() {
        for (k, v) in bindings {
            if own(&values, k) {
                return Err(format!(
                    "Свойство {k} одновременно имеет значение и двустороннюю привязку"
                ));
            }
            values[k] = json!({"expr":v})
        }
    }
    let mut out = json!({});
    for (k, v) in values.as_object().unwrap() {
        out[k] = evaluate_value(v, props, state, &[], false, env)?
    }
    Ok(out)
}
pub fn validate_type_name(ty: &str, enums: &V) -> R<()> {
    let name = ty.strip_suffix('?').unwrap_or(ty);
    if ![
        "String", "Bool", "Boolean", "Number", "Int", "Float", "Color", "Length", "Duration",
        "Asset",
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
pub fn matches_type(value: &V, ty: &str, enums: &V) -> bool {
    if let Some(ty) = ty.strip_suffix('?') {
        return value.is_null() || matches_type(value, ty, enums);
    }
    if let Some(a) = enums[ty].as_array() {
        return value.as_str().is_some_and(|s| {
            a.iter()
                .any(|v| v.as_str().is_some_and(|v| s == format!("{ty}.{v}")))
        });
    }
    let raw = value.get("expr").unwrap_or(value);
    match ty {
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
                || raw.as_str().is_some_and(|s| unit(s, "px") || unit(s, "%"))
        }
        "Duration" => raw.as_str().is_some_and(|s| unit(s, "ms")),
        _ => false,
    }
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
            let mut out = Vec::new();
            let empty_nodes = json!([]);
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
                if node["type"] == "If" {
                    let c = evaluate_value(
                        &node["condition"],
                        self.props,
                        self.state,
                        &[],
                        false,
                        env,
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
                        env,
                        prefix,
                        depth + 1,
                    )?);
                    continue;
                }
                if node["type"] == "For" {
                    let item = node["item"].as_str().ok_or("Некорректный for")?;
                    if own(&env["locals"], item) {
                        return Err(format!("Вложенный for повторяет имя {item}"));
                    }
                    let collection = evaluate_value(
                        &node["collection"],
                        self.props,
                        self.state,
                        &[],
                        false,
                        env,
                    )?;
                    let collection = collection.as_array().ok_or("for ожидает коллекцию")?;
                    if collection.is_empty() {
                        out.extend(self.walk(
                            node.get("emptyChildren").unwrap_or(&empty_nodes),
                            env,
                            prefix,
                            depth + 1,
                        )?)
                    }
                    let mut keys = std::collections::HashSet::new();
                    for (item_index, v) in collection.iter().enumerate() {
                        let mut local = env.clone();
                        if !local["locals"].is_object() {
                            local["locals"] = json!({})
                        }
                        local["locals"][item] = v.clone();
                        if !local["_formaBindings"].is_object() {
                            local["_formaBindings"] = json!({})
                        }
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
                            &format!(
                                "{prefix}@{}:{identity}/",
                                node["start"].as_u64().unwrap_or(index as u64)
                            ),
                            depth + 1,
                        )?)
                    }
                    continue;
                }
                let values = if self.options["evaluateProps"] == false {
                    node["props"].clone()
                } else {
                    evaluate_properties(node, self.props, self.state, env)?
                };
                let mut args = json!({});
                if let Some(o) = node["eventArgs"].as_object() {
                    for (k, a) in o {
                        args[k] = V::Array(
                            a.as_array()
                                .ok_or("Некорректные аргументы события")?
                                .iter()
                                .map(|v| evaluate_value(v, self.props, self.state, &[], false, env))
                                .collect::<R<_>>()?,
                        )
                    }
                }
                let mut result = node.clone();
                result["props"] = values.clone();
                result["eventArgs"] = args;
                result["eventArgExpressions"] = node.get("eventArgs").cloned().unwrap_or(json!({}));
                result["environment"] = env.clone();
                let annotation = (self.hook)(node, env)?;
                if let Some(v) = annotation.get("propertyOrigins") {
                    result["propertyOrigins"] = v.clone()
                }
                if node["type"] == "Slider" && values["value"].is_number() {
                    let path = node["props"]["value"]["expr"].as_str();
                    result["normalizedRange"] = json!(
                        node["bindings"]["value"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                            || path.is_some_and(|s| s.starts_with("state.")
                                || own(&env["locals"], s.split('.').next().unwrap_or("")))
                    )
                }
                if !prefix.is_empty() {
                    let key = values
                        .get("key")
                        .filter(|v| !v.is_null())
                        .cloned()
                        .unwrap_or_else(|| node.get("start").cloned().unwrap_or(json!(index)));
                    result["props"]["key"] = json!(format!("{prefix}{}", evaluate::text(&key)?))
                }
                result["children"] =
                    if self.options["recursive"] == false || annotation["recursive"] == false {
                        node["children"].clone()
                    } else {
                        V::Array(self.walk(
                            node.get("children").unwrap_or(&empty_nodes),
                            env,
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
