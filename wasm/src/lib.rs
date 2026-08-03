use themoretheless_tokenizer::web_bridge::tokenization_json;
use wasm_bindgen::prelude::*;

/// Runs the Rust tokenizer in the browser and returns a compact JSON payload.
#[wasm_bindgen]
pub fn tokenize_json(source: &str, mode: &str, layer: &str) -> String {
    tokenization_json(source, mode, layer)
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
}
