//! AST usage analysis, dead-code/unused-key lints, and typecheck integration.

use serde_json::{Value as V, json};
use std::collections::HashSet;

use super::ast::{name, nonempty, path};
use super::lexer::{R, tokenize_borrowed};
use super::parser::parse_with_span_table;

/// Usage sets gathered by walking the AST (`expr` nodes, bindings, events,
/// forwards, applies) instead of matching raw source strings, so string
/// literals never count and interpolated/call expressions are analyzed.
#[derive(Default)]
struct Usage {
    props: HashSet<String>,
    states: HashSet<String>,
    styles: HashSet<String>,
    tokens: HashSet<String>,
    i18n: HashSet<String>,
}
/// Record a resolved reference path (`props.x`, `state.x`, `tokens.a.b`,
/// `i18n.k` or a bare identifier) into the usage sets. Only syntactically
/// valid identifier paths are recorded, mirroring `evaluate::references`.
fn record_reference(u: &mut Usage, raw: &str) {
    let s = raw.strip_prefix('!').unwrap_or(raw);
    if let Some(rest) = s.strip_prefix("props.") {
        if let Some(head) = rest.split('.').next().filter(|h| name(h)) {
            u.props.insert(head.into());
        }
    } else if let Some(rest) = s.strip_prefix("state.") {
        if let Some(head) = rest.split('.').next().filter(|h| name(h)) {
            u.states.insert(head.into());
        }
    } else if let Some(rest) = s.strip_prefix("tokens.") {
        if path(rest) {
            u.tokens.insert(rest.into());
        }
    } else if let Some(rest) = s.strip_prefix("i18n.") {
        if path(rest) {
            u.i18n.insert(rest.into());
        }
    } else if name(s) {
        u.props.insert(s.into());
    }
}
fn collect_usage(v: &V, u: &mut Usage, collect_style_decls: bool) {
    if let Some(s) = v["expr"].as_str() {
        record_reference(u, s);
    }
    if let Some(forwards) = v["forward"].as_array() {
        for f in forwards {
            if let Some(name) = f.as_str() {
                u.props.insert(name.into());
            }
        }
    }
    if let Some(applies) = v["apply"].as_array() {
        for a in applies {
            if let Some(sn) = a.as_str() {
                u.styles.insert(sn.into());
            }
        }
    }
    if let Some(bindings) = v["bindings"].as_object() {
        for bval in bindings.values() {
            if let Some(s) = bval.as_str() {
                record_reference(u, s);
            }
        }
    }
    if let Some(events) = v["events"].as_object() {
        for eval in events.values() {
            if let Some(s) = eval.as_str() {
                record_reference(u, s);
            }
        }
    }
    if let Some(action) = v["action"].as_str() {
        record_reference(u, action);
    }
    if let Some(a) = v.as_array() {
        for item in a {
            collect_usage(item, u, collect_style_decls);
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
            collect_usage(val, u, collect_style_decls);
        }
    }
}
/// A `tokens`/`i18n` key declaration re-derived from the source (the
/// transport AST stores no ranges for dictionary keys). `start`/`end` are
/// UTF-16 offsets of the key itself; `key` is the full dotted path including
/// the group, e.g. `color.brand` or `en.greeting`.
struct DictDecl {
    i18n: bool,
    key: String,
    start: usize,
    end: usize,
}
fn dict_declarations(source: &str) -> R<Vec<DictDecl>> {
    let (tokens, _, _) = tokenize_borrowed(source)?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let i18n = match tokens[i].text {
            "tokens" => false,
            "i18n" => true,
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i + 1;
        let group = if tokens.get(j).map(|t| t.text) == Some("{") {
            None
        } else if tokens.get(j).is_some_and(|t| name(t.text))
            && tokens.get(j + 1).map(|t| t.text) == Some("{")
        {
            let g = tokens[j].text;
            j += 1;
            Some(g)
        } else {
            i += 1;
            continue;
        };
        j += 1; // past "{"
        while j < tokens.len() && tokens[j].text != "}" {
            let key_tok = tokens[j];
            if !path(key_tok.text) || tokens.get(j + 1).map(|t| t.text) != Some(":") {
                break;
            }
            j += 2;
            // The value runs until ";" at bracket depth 0 (strings are single
            // tokens; brackets are balanced because the source parsed).
            let mut depth = 0usize;
            while j < tokens.len() {
                let t = tokens[j].text;
                if depth == 0 && (t == ";" || t == "}") {
                    break;
                }
                match t {
                    "{" | "[" | "(" => depth += 1,
                    "}" | "]" | ")" => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            if j >= tokens.len() || tokens[j].text != ";" {
                break;
            }
            out.push(DictDecl {
                i18n,
                key: match group {
                    Some(g) => format!("{g}.{}", key_tok.text),
                    None => key_tok.text.into(),
                },
                start: key_tok.start,
                end: key_tok.end,
            });
            j += 1; // past ";"
        }
        i += 1;
    }
    Ok(out)
}
/// Does the `tokens.`/`i18n.` reference tail `r` keep the declared key `k`
/// alive? Covers exact full paths (`tokens.color.brand`), group heads
/// (`tokens.color`), member access into an object-valued key
/// (`tokens.font.size` for `font`) and bare copies of grouped keys
/// (`tokens.brand`, and the locale fallback `i18n.greeting` matching
/// `en.greeting` in every locale group).
fn dict_key_used(r: &str, k: &str) -> bool {
    r == k
        || (k.starts_with(r) && k[r.len()..].starts_with('.'))
        || (r.starts_with(k) && r[k.len()..].starts_with('.'))
        || k.rsplit_once('.').is_some_and(|(_, tail)| tail == r)
}
pub fn lint_document(source: &str) -> R<V> {
    lint_document_with_context(source, &crate::typecheck::TypeContext::default())
}
pub fn lint_document_with_context(source: &str, context: &crate::typecheck::TypeContext) -> R<V> {
    let (doc, expression_spans) = parse_with_span_table(source)?;
    let mut warnings = Vec::new();
    let mut usage = Usage::default();

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

    for section in [
        "nodes",
        "matches",
        "shortcuts",
        "styles",
        "i18n",
        "tokens",
        "slots",
        "overrides",
        "states",
        "entries",
    ] {
        collect_usage(&doc[section], &mut usage, section == "styles");
    }
    if let Some(defaults) = doc["defaults"].as_object() {
        for (k, v) in defaults {
            let is_let = doc["lets"].get(k).is_some();
            if is_let {
                collect_usage(v, &mut usage, false);
            } else {
                // A prop default does not keep other props alive (the prop
                // itself may be dead), but it does use states/styles/tokens/i18n.
                let mut inner = Usage::default();
                collect_usage(v, &mut inner, false);
                usage.states.extend(inner.states);
                usage.styles.extend(inner.styles);
                usage.tokens.extend(inner.tokens);
                usage.i18n.extend(inner.i18n);
            }
        }
    }
    // State defaults are real expression contexts: references there count.
    if let Some(state_defs) = doc["stateDefinitions"].as_object() {
        for def in state_defs.values() {
            collect_usage(&def["default"], &mut usage, false);
        }
    }

    if let Some(lets) = doc["lets"].as_object() {
        for name in lets.keys() {
            if !usage.props.contains(name.as_str()) {
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
            if !usage.props.contains(name.as_str()) {
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
            if !usage.states.contains(name.as_str()) {
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
            if !usage.styles.contains(name.as_str()) {
                warnings.push(json!({
                    "code": "unused-style",
                    "name": name,
                    "message": format!("Неиспользуемый стиль style {name}")
                }));
            }
        }
    }

    // Dead token/i18n keys. Keys of a shared module file (no component or
    // design body) are its public API, so the lints only fire with a body.
    let has_body = !doc["designBody"].is_null()
        || nonempty(&doc["nodes"])
        || nonempty(&doc["defaults"])
        || nonempty(&doc["propDefinitions"])
        || nonempty(&doc["stateDefinitions"]);
    if has_body {
        for decl in dict_declarations(source)? {
            let used = if decl.i18n {
                &usage.i18n
            } else {
                &usage.tokens
            };
            if used.iter().any(|r| dict_key_used(r, &decl.key)) {
                continue;
            }
            let (code, message) = if decl.i18n {
                (
                    "unused-i18n-key",
                    format!("Неиспользуемый ключ i18n {}", decl.key),
                )
            } else {
                (
                    "unused-token",
                    format!("Неиспользуемый токен tokens {}", decl.key),
                )
            };
            warnings.push(json!({
                "code": code,
                "name": decl.key,
                "from": decl.start,
                "to": decl.end,
                "message": message
            }));
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

    warnings.extend(crate::typecheck::analyze(&doc, &expression_spans, context).diagnostics);
    warnings.sort_by_key(|warning| warning["from"].as_u64().unwrap_or(u64::MAX));
    Ok(json!(warnings))
}
