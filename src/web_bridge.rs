//! JSON serialization shared by the local and WebAssembly playground adapters.

use std::fmt::Write as _;

use themoretheless_tokenizer_core::{HostError, HostTokenization, TokenLayer};

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
