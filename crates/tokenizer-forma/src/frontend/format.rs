//! Canonical Forma source serializer and trivia-preserving formatter.

use serde_json::Value as V;

use super::lexer::{Comment, R, tokenize_borrowed};
use super::parser::parse;

pub(super) fn unit_literal(s: &str) -> bool {
    if !s
        .as_bytes()
        .last()
        .is_some_and(|b| *b == b'%' || b.is_ascii_alphabetic())
        || !s
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit() || matches!(*b, b'-' | b'+' | b'.'))
    {
        return false;
    }
    [
        "turn", "vmin", "vmax", "rem", "deg", "rad", "px", "ms", "em", "vw", "vh", "ch", "fr", "s",
        "%",
    ]
    .iter()
    .any(|unit| {
        s.strip_suffix(unit)
            .is_some_and(|v| v.parse::<f64>().is_ok())
    })
}

fn push_escaped(out: &mut String, s: &str) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut start = 0;
    while i < bytes.len() {
        let esc = match bytes[i] {
            b'\\' => Some("\\\\"),
            b'\'' => Some("\\'"),
            b'\n' => Some("\\n"),
            b'\r' => Some("\\r"),
            b'\t' => Some("\\t"),
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                out.push_str(&s[start..i]);
                out.push_str("\\${");
                i += 2;
                start = i;
                continue;
            }
            _ => None,
        };
        if let Some(rep) = esc {
            out.push_str(&s[start..i]);
            out.push_str(rep);
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < s.len() {
        out.push_str(&s[start..]);
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    push_escaped(&mut out, s);
    out.push('\'');
    out
}

pub fn serialize_value(v: &V) -> String {
    if let Some(s) = v["expr"].as_str() {
        return s.into();
    }
    if let Some(a) = v["interpolation"].as_array() {
        let mut out = String::from("'");
        for item in a {
            if let Some(s) = item.as_str() {
                push_escaped(&mut out, s);
            } else {
                out.push_str("${");
                out.push_str(&serialize_value(item));
                out.push('}');
            }
        }
        out.push('\'');
        return out;
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
        fn serialize_pattern(p: &V) -> String {
            if let Some(alts) = p["or"].as_array() {
                return alts
                    .iter()
                    .map(serialize_pattern)
                    .collect::<Vec<_>>()
                    .join(" | ");
            }
            if let Some(op) = p["comparison"].as_str() {
                return format!("{op} {}", serialize_value(&p["value"]));
            }
            if let Some(a) = p.as_array() {
                return format!(
                    "({})",
                    a.iter()
                        .map(serialize_pattern)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            serialize_value(p)
        }
        return format!(
            "match {} {{ {} }}",
            tuple(subject),
            v["branches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    let guard = b
                        .get("guard")
                        .map(|g| format!(" if {}", serialize_value(g)))
                        .unwrap_or_default();
                    format!(
                        "{}{} => {};",
                        serialize_pattern(&b["pattern"]),
                        guard,
                        serialize_value(&b["value"])
                    )
                })
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

/// Emit a trivia comment with the current indentation style (2 spaces per
/// level). Line comments always terminate the line; block comments are kept
/// verbatim and only move to their own line when they are not trailing code.
fn format_comment(
    out: &mut String,
    source: &str,
    comment: &Comment,
    indent: usize,
    at_line_start: &mut bool,
    last_byte_end: Option<usize>,
    next_tok: &str,
) {
    let same_line =
        last_byte_end.is_some_and(|end| !source[end..comment.byte_start].contains('\n'));
    if same_line && *at_line_start && out.ends_with('\n') {
        // The preceding token already terminated the line (";\n", "{\n",
        // "}\n"): pull the comment back onto it.
        out.pop();
        *at_line_start = false;
    }
    let line_comment = comment.text.starts_with("//");
    if same_line && !*at_line_start {
        if !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(comment.text);
        if line_comment {
            out.push('\n');
            *at_line_start = true;
        } else if !next_tok.is_empty()
            && !matches!(
                next_tok,
                ";" | "," | ")" | "]" | "}" | ":" | "." | "?." | "(" | "{"
            )
        {
            out.push(' ');
        }
    } else {
        if !*at_line_start {
            out.push('\n');
        }
        for _ in 0..indent {
            out.push_str("  ");
        }
        out.push_str(comment.text);
        out.push('\n');
        *at_line_start = true;
    }
}

pub fn format_source(source: &str) -> R<String> {
    let _ = parse(source)?;
    let (tokens, comments, _) = tokenize_borrowed(source)?;
    let mut out = String::with_capacity(source.len() + 64);
    let mut indent = 0usize;
    let mut generic_depth = 0usize;
    let mut ternary_depth = 0usize;
    let mut at_line_start = true;
    let mut last_byte_end: Option<usize> = None;
    let mut ci = 0usize;
    for (idx, tok) in tokens.iter().enumerate() {
        let t = tok.text;
        let next = tokens.get(idx + 1).map(|x| x.text).unwrap_or("");
        while ci < comments.len() && comments[ci].start < tok.start {
            format_comment(
                &mut out,
                source,
                &comments[ci],
                indent,
                &mut at_line_start,
                last_byte_end,
                t,
            );
            last_byte_end = Some(comments[ci].byte_end);
            ci += 1;
        }
        last_byte_end = Some(tok.byte_end);
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
        let prev = idx
            .checked_sub(1)
            .and_then(|p| tokens.get(p))
            .map(|x| x.text)
            .unwrap_or("");
        let is_generic_open = t == "<" && matches!(prev, "List" | "Map" | "Record");
        if is_generic_open {
            generic_depth += 1;
        }
        let closing_generic = t == ">" && generic_depth > 0;
        if closing_generic {
            generic_depth -= 1;
        }
        let prev_closes_generic = prev == ">"
            && generic_depth == 0
            && idx >= 2
            && tokens[..idx]
                .iter()
                .rev()
                .find(|tok| matches!(tok.text, "<" | ";" | "{" | "}"))
                .is_some_and(|tok| tok.text == "<");
        let is_nullable_type_q =
            t == "?" && (prev_closes_generic || prev.starts_with(|c: char| c.is_ascii_uppercase()));
        if t == "?" && !is_nullable_type_q {
            ternary_depth += 1;
        } else if t == ":" && ternary_depth > 0 {
            ternary_depth -= 1;
        } else if matches!(t, ";" | "{" | "}") {
            ternary_depth = 0;
            generic_depth = 0;
        }
        let is_attr_hash = t == "#" && next == "[";
        let is_unary_sign = matches!(t, "-" | "+")
            && matches!(
                prev,
                "" | "("
                    | "["
                    | "{"
                    | ":"
                    | "="
                    | "=>"
                    | "?"
                    | ","
                    | ";"
                    | ".."
                    | "..="
                    | "+"
                    | "-"
                    | "*"
                    | "/"
                    | "%"
                    | "=="
                    | "!="
                    | "==="
                    | "!=="
                    | "<"
                    | "<="
                    | ">"
                    | ">="
                    | "&&"
                    | "||"
                    | "??"
            );
        let no_space_after = t == "("
            || t == "["
            || is_attr_hash
            || is_unary_sign
            || t == "@"
            || t == "!"
            || t == "."
            || t == "?."
            || t == ".."
            || t == "..="
            || is_generic_open;
        let next_opens_generic = next == "<" && matches!(t, "List" | "Map" | "Record");
        let next_closes_generic = next == ">" && generic_depth > 0;
        let next_is_nullable_type_suffix =
            next == "?" && (closing_generic || t.starts_with(|c: char| c.is_ascii_uppercase()));
        let next_is_property_colon = next == ":" && ternary_depth == 0;
        let no_space_before_next = next == ")"
            || next == "]"
            || next == ";"
            || next == ","
            || next_is_property_colon
            || next == "."
            || next == "?."
            || next == ".."
            || next == "..="
            || next == "("
            || next_opens_generic
            || next_closes_generic
            || next_is_nullable_type_suffix;
        let force_space_before_paren = matches!(
            t,
            "if" | "for" | "match" | "return" | "where" | "=" | "=>" | "->" | "<->"
        ) || (!is_unary_sign
            && matches!(
                t,
                "+" | "-"
                    | "*"
                    | "/"
                    | "%"
                    | "=="
                    | "!="
                    | "==="
                    | "!=="
                    | "<"
                    | "<="
                    | ">"
                    | ">="
                    | "&&"
                    | "||"
                    | "??"
                    | "?"
                    | ":"
            ));
        if !no_space_after
            && (!no_space_before_next || (next == "(" && force_space_before_paren))
            && !next.is_empty()
            && next != "{"
        {
            out.push(' ');
        }
    }
    while ci < comments.len() {
        format_comment(
            &mut out,
            source,
            &comments[ci],
            indent,
            &mut at_line_start,
            last_byte_end,
            "",
        );
        last_byte_end = Some(comments[ci].byte_end);
        ci += 1;
    }
    Ok(out.trim_end().to_string() + "\n")
}
