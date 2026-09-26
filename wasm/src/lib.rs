use themoretheless_tokenizer::web_bridge::{catalog, tokenization, tokenization_json};
use wasm_bindgen::prelude::*;

/// Runs the Rust tokenizer in the browser and returns a compact JSON payload.
///
/// Prefer [`tokenize`] with an explicit language id.
#[wasm_bindgen]
pub fn tokenize_json(source: &str, mode: &str, layer: &str) -> String {
    tokenization_json(source, mode, layer)
}

/// Multi-language host entry: `language` is `json`, `url`, …
#[wasm_bindgen]
pub fn tokenize(language: &str, source: &str, mode: &str, layer: &str) -> String {
    tokenization(language, source, mode, layer)
}

/// Registry catalog: every enabled engine with family, presets, capability
/// bits and dialects, as JSON.
#[wasm_bindgen]
pub fn wasm_catalog() -> String {
    catalog()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_unicode_byte_spans() {
        let output = tokenize_json(r#"{"city":"Тбилиси"}"#, "strict", "semantic");
        assert!(output.contains("\"valid\":true"));
        assert!(output.contains("\"sourceBytes\":25"));
        assert!(output.contains("\"kind\":\"property\""));
    }

    #[test]
    fn catalog_reports_the_format_family() {
        let output = wasm_catalog();
        assert!(output.contains("\"engines\":[{"));
        assert!(output.contains("\"family\":\"format\""));
    }

    #[test]
    fn multi_language_json_path() {
        let output = tokenize("json", r#"{"a":1}"#, "strict", "syntax");
        assert!(output.contains("\"language\":\"json\""));
        assert!(output.contains("\"valid\":true"));
    }
}
