//! Native JSON parser and serializer for Rush runtime.
use crate::{RecordMap, Value};
use themoretheless_tokenizer_json::Value as JsonValue;

/// Parse a JSON string into a Rush `Value`.
pub fn parse(source: &str) -> Result<Value<'static>, String> {
    let parsed = themoretheless_tokenizer_json::parse(source);
    if let Some(err) = parsed.diagnostics().first() {
        return Err(format!("{}", err.kind));
    }
    let Some(root) = parsed.into_value() else {
        return Err("Empty or invalid JSON input".into());
    };
    convert_json_value(root)
}

fn convert_json_value(value: JsonValue<'_>) -> Result<Value<'static>, String> {
    match value {
        JsonValue::Null(_) => Ok(Value::Null),
        JsonValue::Boolean(b) => Ok(Value::Bool(b.value())),
        JsonValue::Number(num) => {
            let n = num
                .as_f64()
                .map_err(|e| format!("Invalid JSON number: {e:?}"))?;
            Ok(Value::Number(n))
        }
        JsonValue::String(s) => {
            let decoded = s
                .decoded()
                .ok_or_else(|| "Malformed JSON string escape".to_string())?;
            Ok(Value::String(decoded.to_owned()))
        }
        JsonValue::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for elem in arr.elements() {
                items.push(convert_json_value(elem.clone())?);
            }
            Ok(Value::List(items))
        }
        JsonValue::Object(obj) => {
            let mut fields = Vec::with_capacity(obj.len());
            for member in obj.members() {
                let key = member
                    .key()
                    .decoded()
                    .ok_or_else(|| "Malformed JSON object key".to_string())?;
                let val = convert_json_value(member.value().clone())?;
                fields.push((key.to_owned(), val));
            }
            Ok(Value::Record(RecordMap::from_iter(fields)))
        }
        _ => Err("Unknown JSON value".to_string()),
    }
}

/// Serialize a Rush `Value` into a JSON string.
pub fn stringify(value: &Value<'_>) -> Result<String, String> {
    let mut out = String::with_capacity(64);
    encode_value(value, &mut out)?;
    Ok(out)
}

#[inline]
fn write_json_number(n: f64, out: &mut String) {
    use std::fmt::Write as _;
    if n.fract() == 0.0 && n >= (i64::MIN as f64) && n <= (i64::MAX as f64) {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{}", n);
    }
}

fn encode_value(value: &Value<'_>, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{i}");
        }
        Value::Number(n) => {
            if !n.is_finite() {
                return Err("Non-finite number cannot be serialized to JSON".into());
            }
            write_json_number(*n, out);
        }
        Value::Angle(a) => {
            if !a.is_finite() {
                return Err("Non-finite angle cannot be serialized to JSON".into());
            }
            use std::fmt::Write as _;
            let _ = write!(out, "{}", a);
        }
        Value::String(s) => escape_json_str(s, out),
        Value::List(items) | Value::Tuple(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_value(item, out)?;
            }
            out.push(']');
        }
        Value::Vector(components) => {
            out.push('[');
            for (i, c) in components.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !c.is_finite() {
                    return Err("Non-finite vector component cannot be serialized to JSON".into());
                }
                use std::fmt::Write as _;
                let _ = write!(out, "{}", c);
            }
            out.push(']');
        }
        Value::Record(fields) => {
            out.push('{');
            for (i, (key, val)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape_json_str(key, out);
                out.push(':');
                encode_value(val, out)?;
            }
            out.push('}');
        }
        Value::UserData(data) if data.variant.is_none() => {
            if let Some(record @ Value::Record(_)) = data.values.first() {
                encode_value(record, out)?;
            } else {
                return Err(format!(
                    "UserData '{}' cannot be serialized to JSON",
                    data.type_name
                ));
            }
        }
        Value::Variant(tag, payload) => {
            out.push_str("{\"tag\":");
            escape_json_str(tag, out);
            out.push_str(",\"values\":[");
            for (i, val) in payload.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_value(val, out)?;
            }
            out.push_str("]}");
        }
        other => {
            return Err(format!("Value cannot be serialized to JSON: {other:?}"));
        }
    }
    Ok(())
}

fn escape_json_str(s: &str, out: &mut String) {
    out.push('"');
    let bytes = s.as_bytes();
    if !bytes.iter().any(|&b| b < b' ' || b == b'"' || b == b'\\') {
        out.push_str(s);
        out.push('"');
        return;
    }
    let mut last = 0;
    for (idx, c) in s.char_indices() {
        let escaped = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '\x08' => "\\b",
            '\x0C' => "\\f",
            c if c < ' ' => {
                if last < idx {
                    out.push_str(&s[last..idx]);
                }
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
                last = idx + c.len_utf8();
                continue;
            }
            _ => continue,
        };
        if last < idx {
            out.push_str(&s[last..idx]);
        }
        out.push_str(escaped);
        last = idx + c.len_utf8();
    }
    if last < s.len() {
        out.push_str(&s[last..]);
    }
    out.push('"');
}
