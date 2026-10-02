use themoretheless_tokenizer::web_bridge::{catalog, tokenization, tokenization_json};
use wasm_bindgen::prelude::*;

/// Runs the Rust tokenizer in the browser and returns a compact JSON payload.
///
/// Prefer [`tokenize`] with an explicit language id.
#[wasm_bindgen]
pub fn tokenize_json(source: &str, mode: &str, layer: &str) -> String {
    tokenization_json(source, mode, layer)
}

/// Checked rename edits, using the same bindings and nominal symbols as Rush diagnostics.
#[wasm_bindgen]
pub fn rename_rush(source: &str, offset: u32, replacement: &str) -> String {
    themoretheless_tokenizer::web_bridge::rename_rush(source, offset as usize, replacement)
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

/// Execute the functional Rush runtime and return a JSON result or diagnostic.
#[wasm_bindgen]
pub fn run_rush(source: &str) -> String {
    themoretheless_tokenizer::web_bridge::execute_rush(source)
}

#[cfg(test)]
mod runtime_tests {
    use super::run_rush;
    #[test]
    fn execution_returns_graphics_and_failures_as_json() {
        let svg = run_rush("polygon([vec2(0,0),vec2(1,0),vec2(0,1)])");
        assert!(svg.contains("\"kind\":\"svg\""));
        assert!(svg.contains("<svg"));
        assert!(run_rush("1 / 0").contains("\"ok\":false"));
        assert!(run_rush("while true {}").contains("Execution limit exceeded"));
    }
}

#[cfg(test)]
mod data_limit_tests {
    use super::run_rush;

    #[test]
    fn playground_bounds_materialized_collections() {
        assert!(run_rush("range(0,10001)").contains("Collection item limit exceeded"));
        assert!(
            run_rush("range_iter(0,1000000) | collect(10001)")
                .contains("Collection item limit exceeded")
        );
        assert!(run_rush("range_iter(0,1000000) | collect(2)").contains("\"ok\":true"));
    }
}

#[cfg(test)]
mod output_limit_tests {
    use super::run_rush;
    #[test]
    fn repeated_values_cannot_expand_text_output_without_bound() {
        let source = format!(
            "let text = \"{}\"; range(0,2000) | map(x => text)",
            "a".repeat(1000)
        );
        let result = run_rush(&source);
        assert!(
            result.contains("Text output byte limit exceeded"),
            "{result}"
        );
        assert!(result.len() < 200);
        assert!(run_rush("range(0,10)").contains("\"ok\":true"));
    }
}

#[cfg(test)]
mod obj_limit_tests {
    #[test]
    fn large_finite_coordinates_cannot_overflow_obj_output_budget() {
        let result =
            super::run_rush("grid_mesh(range(0,35),range(0,35),(x,y) => vec3(1e308,1e308,1e308))");
        assert!(
            result.contains("OBJ output byte limit exceeded"),
            "{result}"
        );
        assert!(result.len() < 200);
    }
}

#[cfg(test)]
mod svg_limit_tests {
    #[test]
    fn large_svg_stops_at_output_limit() {
        let result = super::run_rush("range(0,2000) | map(x => vec2(1e308,1e308)) | polygon");
        assert!(
            result.contains("SVG output byte limit exceeded"),
            "{result}"
        );
        assert!(result.len() < 200);
    }
}
