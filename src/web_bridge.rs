//! JSON serialization shared by the local and WebAssembly playground adapters.

use std::fmt::Write as _;

use crate::json::{
    LexDiagnostic, LexerOptions, ParseDiagnostic, ParseOptions, SemanticToken, lex_with,
    tokenize_with,
};

/// Tokenizes source for the development playground and returns a JSON payload.
#[must_use]
pub fn tokenization_json(source: &str, mode: &str, layer: &str) -> String {
    let jsonc = mode == "jsonc";
    if layer == "syntax" {
        syntax_json(source, jsonc)
    } else {
        semantic_json(source, jsonc)
    }
}

fn syntax_json(source: &str, jsonc: bool) -> String {
    let options = if jsonc {
        LexerOptions::jsonc()
    } else {
        LexerOptions::strict()
    };
    let result = lex_with(source, options);
    let mut output = response_start(
        source,
        if jsonc { "jsonc" } else { "strict" },
        "syntax",
        !result.has_errors(),
    );
    for (index, token) in result.tokens().iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let kind = debug_name(token.kind);
        push_token(
            &mut output,
            index,
            &kind,
            token.span.start,
            token.span.end,
            token.text(source).unwrap_or(""),
            token.has_error(),
        );
    }
    output.push_str("],\"diagnostics\":[");
    for (index, diagnostic) in result.diagnostics().iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_lex_diagnostic(&mut output, diagnostic);
    }
    output.push_str("]}");
    output
}

fn semantic_json(source: &str, jsonc: bool) -> String {
    let options = if jsonc {
        ParseOptions::jsonc()
    } else {
        ParseOptions::strict()
    };
    let result = tokenize_with(source, options);
    let mut output = response_start(
        source,
        if jsonc { "jsonc" } else { "strict" },
        "semantic",
        result.is_valid(),
    );
    for (index, token) in result.tokens.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_semantic_token(&mut output, index, token, source);
    }
    output.push_str("],\"diagnostics\":[");
    for (index, diagnostic) in result.diagnostics.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_parse_diagnostic(&mut output, diagnostic);
    }
    output.push_str("]}");
    output
}

fn response_start(source: &str, mode: &str, layer: &str, valid: bool) -> String {
    format!(
        "{{\"mode\":\"{mode}\",\"layer\":\"{layer}\",\"valid\":{valid},\"sourceBytes\":{},\"tokens\":[",
        source.len()
    )
}

fn push_semantic_token(output: &mut String, index: usize, token: &SemanticToken, source: &str) {
    let kind = debug_name(token.kind);
    push_token(
        output,
        index,
        &kind,
        token.span.start,
        token.span.end,
        token.text(source).unwrap_or(""),
        kind == "invalid",
    );
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

fn push_lex_diagnostic(output: &mut String, diagnostic: &LexDiagnostic) {
    push_diagnostic(
        output,
        diagnostic.kind.code(),
        &diagnostic.kind.to_string(),
        diagnostic.span.start,
        diagnostic.span.end,
    );
}

fn push_parse_diagnostic(output: &mut String, diagnostic: &ParseDiagnostic) {
    push_diagnostic(
        output,
        diagnostic.kind.code(),
        &diagnostic.kind.to_string(),
        diagnostic.span.start,
        diagnostic.span.end,
    );
}

fn push_diagnostic(output: &mut String, code: &str, message: &str, start: usize, end: usize) {
    output.push_str("{\"code\":");
    push_json_string(output, code);
    output.push_str(",\"message\":");
    push_json_string(output, message);
    let _ = write!(output, ",\"start\":{start},\"end\":{end}}}");
}

fn debug_name(value: impl std::fmt::Debug) -> String {
    let value = format!("{value:?}");
    let mut result = String::with_capacity(value.len() + 4);
    for (index, character) in value.chars().enumerate() {
        if character.is_ascii_uppercase() && index != 0 {
            result.push('-');
        }
        result.push(character.to_ascii_lowercase());
    }
    result
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
        assert!(output.contains("\"kind\":\"property\""));
        assert!(output.contains("Тбилиси"));
        assert!(output.contains("\"sourceBytes\":25"));
    }

    #[test]
    fn escapes_source_fragments() {
        let output = tokenization_json("[\n\"a\\tb\"]", "strict", "syntax");
        assert!(output.starts_with('{') && output.ends_with('}'));
        assert!(output.contains("\\n"));
        assert!(output.contains("\\\\t"));
    }
}
