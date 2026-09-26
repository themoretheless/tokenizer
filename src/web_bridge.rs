//! JSON serialization shared by the local and WebAssembly playground adapters.

use std::fmt::Write as _;

use themoretheless_tokenizer_core::{
    Capability, Family, HostError, HostTokenization, TokenLayer, presets_of,
};

use crate::plugins::builtin_registry;

use crate::plugins::analyze_host;

/// Tokenizes source for the development playground and returns a JSON payload.
///
/// `language` is a language id (`json`, `url`, …). `mode` is the dialect
/// (`strict`, `jsonc`, `default`, …). `layer` is `syntax` or `semantic`.
#[must_use]
pub fn tokenization(language: &str, source: &str, mode: &str, layer: &str) -> String {
    let Some(layer) = TokenLayer::parse(layer) else {
        return protocol_error("invalid-layer", &format!("unknown layer `{layer}`"));
    };
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

fn success_payload(
    language: &str,
    mode: &str,
    layer: &str,
    source: &str,
    result: &HostTokenization,
) -> String {
    let mut output = format!(
        "{{\"language\":{},\"mode\":{},\"layer\":{},\"valid\":{},\"sourceBytes\":{},\"tokens\":[",
        json_string(language),
        json_string(mode),
        json_string(layer),
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
    output.push_str("]}");
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
    let mut out = String::from("{\"count\":");
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

fn host_error_payload(error: &HostError) -> String {
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

fn protocol_error(code: &str, message: &str) -> String {
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

fn push_diagnostic(output: &mut String, code: &str, message: &str, start: usize, end: usize) {
    output.push_str("{\"code\":");
    push_json_string(output, code);
    output.push_str(",\"message\":");
    push_json_string(output, message);
    let _ = write!(output, ",\"start\":{start},\"end\":{end}}}");
}

fn json_string(value: &str) -> String {
    let mut output = String::new();
    push_json_string(&mut output, value);
    output
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            character if character <= '\u{1f}' => {
                let _ = write!(output, "\\u{:04x}", character as u32);
            }
            character => output.push(character),
        }
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
}
