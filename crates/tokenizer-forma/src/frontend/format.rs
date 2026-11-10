//! Canonical Forma source serializer and trivia-preserving formatter.

use serde_json::Value as V;

use super::lexer::{Comment, R, tokenize_borrowed};
use super::parser::parse;

pub(super) fn unit_literal(s: &str) -> bool {
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
