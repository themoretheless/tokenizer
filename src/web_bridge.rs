//! JSON serialization shared by the local and WebAssembly playground adapters.

use std::fmt::Write as _;

use themoretheless_tokenizer_core::{
    Capability, Family, HostError, HostTokenization, TokenLayer, presets_of,
};

use crate::plugins::{analyze_host, builtin_registry};

#[cfg(feature = "rush")]
pub use crate::rush_bridge::RushSchedule;
pub use crate::rush_bridge::{execute_rush, rename_rush};

/// Tokenizes source for the development playground and returns a JSON payload.
///
/// `language` is a language id (`json`, `url`, …). `mode` is the dialect
/// (`strict`, `jsonc`, `default`, …). `layer` is `syntax` or `semantic`.
#[must_use]
pub fn tokenization(language: &str, source: &str, mode: &str, layer: &str) -> String {
    let Some(layer) = TokenLayer::parse(layer) else {
        return protocol_error("invalid-layer", &format!("unknown layer `{layer}`"));
    };
    #[cfg(feature = "rush")]
    if language == "rush" {
        return crate::rush_bridge::tokenization_rush(mode, layer, source);
    }
    match analyze_host(language, source, mode, layer) {
        Ok(result) => success_payload(language, mode, layer.as_str(), source, &result),
        Err(error) => host_error_payload(&error),
    }
}

/// Backward-compatible JSON-only entry used by existing WASM exports.
#[must_use]
pub fn tokenization_json(source: &str, mode: &str, layer: &str) -> String {
    tokenization("json", source, mode, layer)
}

pub(crate) fn write_tokenization_prefix(
    language: &str,
    mode: &str,
    layer: &str,
    source: &str,
    result: &HostTokenization,
) -> String {
    let mut output = String::with_capacity(
        source
            .len()
            .saturating_mul(4)
            .saturating_add(result.tokens.len().saturating_mul(48))
            .max(256),
    );
    output.push_str("{\"language\":");
    push_json_string(&mut output, language);
    output.push_str(",\"mode\":");
    push_json_string(&mut output, mode);
    output.push_str(",\"layer\":");
    push_json_string(&mut output, layer);
    let _ = write!(
        output,
        ",\"valid\":{},\"sourceBytes\":{},\"tokens\":[",
        result.valid,
        source.len()
    );
    for (index, token) in result.tokens.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let text = source.get(token.span.start..token.span.end).unwrap_or("");
        push_token(
            &mut output,
            index,
            token.kind.as_ref(),
            token.span.start,
            token.span.end,
            text,
            token.error,
        );
    }
    output.push_str("],\"diagnostics\":[");
    for (index, diagnostic) in result.diagnostics.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_diagnostic(
            &mut output,
            diagnostic.code.as_ref(),
            diagnostic.message.as_ref(),
            diagnostic.span.start,
            diagnostic.span.end,
        );
    }
    output.push(']');
    output
}

fn success_payload(
    language: &str,
    mode: &str,
    layer: &str,
    source: &str,
    result: &HostTokenization,
) -> String {
    let mut output = write_tokenization_prefix(language, mode, layer, source, result);
    output.push('}');
    output
}

/// Registry catalog as JSON: every enabled engine with its family, curated
/// presets, capability bits and dialects.
///
/// The playground picker renders from this instead of restating the Rust-side
/// lists.
#[must_use]
pub fn catalog() -> String {
    let engines = builtin_registry();
    let mut out = String::with_capacity(engines.len().saturating_mul(256).max(512));
    out.push_str("{\"count\":");
    let _ = write!(out, "{},\"engines\":[", engines.len());
    for (index, engine) in engines.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        let descriptor = engine.descriptor();
        out.push_str("{\"id\":");
        push_json_string(&mut out, descriptor.language.as_str());
        out.push_str(",\"displayName\":");
        push_json_string(&mut out, descriptor.display_name);
        out.push_str(",\"family\":");
        push_json_string(&mut out, Family::of(descriptor.language).as_str());
        out.push_str(",\"presets\":[");
        for (preset_index, preset) in presets_of(descriptor.language).iter().enumerate() {
            if preset_index != 0 {
                out.push(',');
            }
            push_json_string(&mut out, preset.as_str());
        }
        out.push_str("],\"capabilities\":[");
        let mut first_capability = true;
        for capability in CAPABILITY_ORDER {
            if descriptor.capabilities.contains(capability.bits()) {
                if !first_capability {
                    out.push(',');
                }
                first_capability = false;
                push_json_string(&mut out, capability.as_str());
            }
        }
        out.push_str("],\"defaultDialect\":");
        push_json_string(&mut out, descriptor.default_dialect.as_str());
        out.push_str(",\"dialects\":[");
        for (dialect_index, dialect) in descriptor.dialects.iter().enumerate() {
            if dialect_index != 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            push_json_string(&mut out, dialect.id.as_str());
            out.push_str(",\"displayName\":");
            push_json_string(&mut out, dialect.display_name);
            out.push('}');
        }
        out.push_str("],\"aliases\":");
        push_string_array(&mut out, descriptor.aliases);
        out.push_str(",\"extensions\":");
        push_string_array(&mut out, descriptor.extensions);
        out.push_str(",\"mimeTypes\":");
        push_string_array(&mut out, descriptor.mime_types);
        out.push_str(",\"engineVersion\":");
        push_json_string(&mut out, descriptor.engine_version);
        out.push('}');
    }
    out.push_str("]}");
    out
}

const CAPABILITY_ORDER: [Capability; 7] = [
    Capability::Lex,
    Capability::Parse,
    Capability::Semantic,
    Capability::Cst,
    Capability::Navigate,
    Capability::Visitor,
    Capability::Validate,
];

fn push_string_array(out: &mut String, values: &[&'static str]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        push_json_string(out, value);
    }
    out.push(']');
}

pub(crate) fn host_error_payload(error: &HostError) -> String {
    let (code, message) = match error {
        HostError::UnknownLanguage { language } => {
            ("unknown-language", format!("unknown language `{language}`"))
        }
        HostError::UnknownDialect { dialect } => {
            ("unknown-dialect", format!("unknown dialect `{dialect}`"))
        }
        HostError::UnsupportedCapability { capability } => (
            "unsupported-capability",
            format!("unsupported capability `{capability}`"),
        ),
        HostError::Capability(error) => ("capability", error.to_string()),
        HostError::InputTooLarge { max, actual } => (
            "input-too-large",
            format!("input is {actual} bytes; max is {max}"),
        ),
        HostError::InvalidOffset { offset, source_len } => (
            "invalid-offset",
            format!("offset {offset} is outside source of length {source_len}"),
        ),
        HostError::InvalidOptions { message } => ("invalid-options", message.clone()),
        other => ("host-error", other.to_string()),
    };
    protocol_error(code, &message)
}

#[cfg_attr(feature = "rush", allow(dead_code))]
pub(crate) fn protocol_error(code: &str, message: &str) -> String {
    format!(
        "{{\"error\":true,\"code\":{},\"message\":{}}}",
        json_string(code),
        json_string(message)
    )
}

fn push_token(
    output: &mut String,
    index: usize,
    kind: &str,
    start: usize,
    end: usize,
    text: &str,
    error: bool,
) {
    let _ = write!(output, "{{\"index\":{index},\"kind\":");
    push_json_string(output, kind);
    let _ = write!(output, ",\"start\":{start},\"end\":{end},\"text\":");
    push_json_string(output, text);
    let _ = write!(output, ",\"error\":{error}}}");
}

pub(crate) fn push_diagnostic(
    output: &mut String,
    code: &str,
    message: &str,
    start: usize,
    end: usize,
) {
    output.push_str("{\"code\":");
    push_json_string(output, code);
    output.push_str(",\"message\":");
    push_json_string(output, message);
    let _ = write!(output, ",\"start\":{start},\"end\":{end}}}");
}

pub(crate) fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    push_json_string(&mut output, value);
    output
}

pub(crate) fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    if !value.bytes().any(|b| b < 0x20 || b == b'"' || b == b'\\') {
        output.push_str(value);
        output.push('"');
        return;
    }
    let mut chunk_start = 0;
    for (idx, character) in value.char_indices() {
        let escaped = match character {
            '"' => Some("\\\""),
            '\\' => Some("\\\\"),
            '\n' => Some("\\n"),
            '\r' => Some("\\r"),
            '\t' => Some("\\t"),
            '\u{08}' => Some("\\b"),
            '\u{0c}' => Some("\\f"),
            character if character <= '\u{1f}' => {
                if chunk_start < idx {
                    output.push_str(&value[chunk_start..idx]);
                }
                let _ = write!(output, "\\u{:04x}", character as u32);
                chunk_start = idx + character.len_utf8();
                continue;
            }
            _ => None,
        };
        if let Some(esc) = escaped {
            if chunk_start < idx {
                output.push_str(&value[chunk_start..idx]);
            }
            output.push_str(esc);
            chunk_start = idx + character.len_utf8();
        }
    }
    if chunk_start < value.len() {
        output.push_str(&value[chunk_start..]);
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unicode_text_and_byte_spans() {
        let output = tokenization_json(r#"{"city":"Тбилиси"}"#, "strict", "semantic");
        assert!(output.contains("\"valid\":true"));
        assert!(output.contains("\"language\":\"json\""));
        assert!(output.contains("\"kind\":\"property\""));
        assert!(output.contains("Тбилиси"));
    }

    #[test]
    fn catalog_reports_family_and_preset_per_engine() {
        let output = catalog();
        assert!(output.contains("\"id\":\"json\""));
        assert!(output.contains("\"family\":\"format\""));
        assert!(output.contains("\"presets\":[\"formats\"]"));
        assert!(output.contains("\"capabilities\":[\"lex\",\"parse\",\"semantic\",\"cst\",\"navigate\",\"visitor\",\"validate\"]"));
    }

    #[test]
    fn escapes_source_fragments() {
        let output = tokenization_json("\"a\\\"b\\n\"", "strict", "syntax");
        assert!(output.contains("\\\""));
        assert!(output.contains("\\n"));
    }

    #[cfg(feature = "url")]
    #[test]
    fn tokenizes_url_language() {
        let output = tokenization("url", "https://example.com", "default", "syntax");
        assert!(output.contains("\"language\":\"url\""));
        assert!(output.contains("u-scheme"));
    }

    #[cfg(feature = "rush")]
    #[test]
    fn tokenizes_rush_playground_in_single_pass() {
        let output = tokenization(
            "rush",
            "const value = missing\nsin(1,2)",
            "default",
            "semantic",
        );
        assert!(output.contains("\"valid\":true"));
        assert!(output.contains("\"unknown-name\""));
        assert!(output.contains("\"argument-count\""));
        assert!(output.contains("\"builtins\":["));
    }
}
