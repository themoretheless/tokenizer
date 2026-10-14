//! JSON serialization shared by the local and WebAssembly playground adapters.

use std::fmt::Write as _;

use themoretheless_tokenizer_core::{
    Capability, Family, HostError, HostTokenization, TokenLayer, presets_of,
};

use crate::plugins::builtin_registry;

use crate::plugins::analyze_host;

/// Compute a validated, document-local Rush rename without executing the source.
pub fn rename_rush(source: &str, offset: usize, replacement: &str) -> String {
    #[cfg(feature = "rush")]
    {
        use themoretheless_tokenizer_rush::{Program, rename_project_symbol};
        let edits = Program::compile(source)
            .map_err(|e| e.message)
            .and_then(|p| rename_project_symbol(&[("main", &p)], "main", offset, replacement));
        match edits {
            Ok(edits) => {
                let mut output = String::from("{\"ok\":true,\"edits\":[");
                for (i, edit) in edits.iter().enumerate() {
                    if i > 0 {
                        output.push(',');
                    }
                    let _ = write!(
                        output,
                        "{{\"start\":{},\"end\":{},\"replacement\":{}}}",
                        edit.span.start,
                        edit.span.end,
                        json_string(&edit.replacement)
                    );
                }
                output.push_str("]}");
                output
            }
            Err(error) => format!("{{\"ok\":false,\"error\":{}}}", json_string(&error)),
        }
    }
    #[cfg(not(feature = "rush"))]
    {
        let _ = (source, offset, replacement);
        String::from("{\"ok\":false,\"error\":\"Rush support is disabled\"}")
    }
}

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
    output.push(']');
    #[cfg(feature = "rush")]
    if language == "rush" {
        let names = themoretheless_tokenizer_rush::analyze_names(source, &[]);
        let calls = themoretheless_tokenizer_rush::analyze_calls(source);
        output.push_str(",\"executionDiagnostics\":[");
        let mut emitted = Vec::new();
        for diagnostic in names.diagnostics.iter().chain(&calls.diagnostics) {
            let key = (diagnostic.code, diagnostic.span);
            if emitted.contains(&key)
                || result
                    .diagnostics
                    .iter()
                    .any(|d| d.code.as_ref() == diagnostic.code && d.span == diagnostic.span.into())
            {
                continue;
            }
            if !emitted.is_empty() {
                output.push(',');
            }
            push_diagnostic(
                &mut output,
                diagnostic.code,
                diagnostic.message,
                diagnostic.span.start,
                diagnostic.span.end,
            );
            emitted.push(key);
        }
        output.push(']');
        let editor = themoretheless_tokenizer_rush::analyze_editor_details(source);
        let references = editor.references;
        let bindings = editor.bindings;
        output.push_str(",\"memberCompletions\":[");
        for (index, completion) in editor.member_completions.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            let _ = write!(
                output,
                "{{\"start\":{},\"end\":{},\"members\":[",
                completion.name_span.start, completion.name_span.end
            );
            for (index, member) in completion.members.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&json_string(member));
            }
            output.push_str("]}");
        }
        output.push(']');
        output.push_str(",\"builtins\":[");
        for (index, (name, builtin)) in themoretheless_tokenizer_rush::builtin_catalog()
            .iter()
            .enumerate()
        {
            if index != 0 {
                output.push(',');
            }
            let arity = builtin.arity();
            let _ = write!(
                output,
                "{{\"name\":{},\"minArgs\":{},\"maxArgs\":{}}}",
                json_string(name),
                arity.start(),
                arity.end()
            );
        }
        output.push(']');
        output.push_str(",\"bindings\":[");
        for (index, binding) in bindings.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let _ = write!(
                output,
                "{{\"name\":{},\"kind\":\"binding\",\"start\":{},\"end\":{},\"depth\":{},\"definition\":{{\"start\":{},\"end\":{}}}}}",
                json_string(&binding.name),
                binding.visible.start,
                binding.visible.end,
                binding.depth,
                binding.definition.start,
                binding.definition.end
            );
            output.pop();
            output.push_str(",\"members\":[");
            for (index, member) in binding.members.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&json_string(member));
            }
            output.push_str("],\"memberPaths\":{");
            for (index, (path, members)) in binding.member_paths.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&json_string(path));
                output.push_str(":[");
                for (index, member) in members.iter().enumerate() {
                    if index > 0 {
                        output.push(',');
                    }
                    output.push_str(&json_string(member));
                }
                output.push(']');
            }
            output.push_str("}}");
        }
        output.push(']');
        output.push_str(",\"references\":[");
        for (index, reference) in references.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            let _ = write!(
                output,
                "{{\"start\":{},\"end\":{},\"definition\":",
                reference.usage.start, reference.usage.end
            );
            if let Some(definition) = reference.definition {
                let _ = write!(
                    output,
                    "{{\"start\":{},\"end\":{}}}",
                    definition.start, definition.end
                );
            } else {
                output.push_str("null");
            }
            output.push('}');
        }
        output.push(']');
    }
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

/// Execute Rush with a fixed budget for playground adapters.
#[must_use]
pub fn execute_rush(source: &str) -> String {
    #[cfg(feature = "rush")]
    {
        use themoretheless_tokenizer_rush::{CancellationToken, ExecutionLimits, Program, Value};
        let limits = ExecutionLimits {
            max_collection_items: 10_000,
            max_string_bytes: 65_536,
            ..ExecutionLimits::new(100_000)
        };
        let result = Program::compile(source).and_then(|program| {
            program.run_with_limits(limits, &CancellationToken::default(), &[], &[], &[])
        });
        match result {
            Ok(value) => {
                let (kind, output) = match value {
                    Value::Polygon(polygon) => {
                        match bounded_rush_output(|output| polygon.write_svg(output)) {
                            Ok(svg) => ("svg", svg),
                            Err("SVG output write failed") => {
                                return rush_export_error(
                                    "output-limit",
                                    "SVG output byte limit exceeded",
                                );
                            }
                            Err(message) => return rush_export_error("export-error", message),
                        }
                    }
                    Value::Mesh(mesh) => match bounded_rush_output(|output| mesh.write_obj(output))
                    {
                        Ok(obj) => ("obj", obj),
                        Err(_) => {
                            return rush_export_error(
                                "output-limit",
                                "OBJ output byte limit exceeded",
                            );
                        }
                    },
                    value => match bounded_rush_output(|output| write!(output, "{value:?}")) {
                        Ok(text) => ("text", text),
                        Err(_) => {
                            return rush_export_error(
                                "output-limit",
                                "Text output byte limit exceeded",
                            );
                        }
                    },
                };
                format!(
                    "{{\"ok\":true,\"kind\":{},\"output\":{}}}",
                    json_string(kind),
                    json_string(&output)
                )
            }
            Err(error) => format!(
                "{{\"ok\":false,\"error\":{},\"start\":{},\"end\":{}}}",
                json_string(&error.message),
                error.span.start,
                error.span.end
            ),
        }
    }
    #[cfg(not(feature = "rush"))]
    {
        let _ = source;
        protocol_error("unsupported-language", "Rush is not enabled in this build")
    }
}

#[cfg(feature = "rush")]
fn bounded_rush_output<E>(
    write: impl FnOnce(&mut StringOutput) -> Result<(), E>,
) -> Result<String, E> {
    let mut output = StringOutput(String::new());
    write(&mut output)?;
    Ok(output.0)
}

#[cfg(feature = "rush")]
struct StringOutput(String);
#[cfg(feature = "rush")]
impl std::fmt::Write for StringOutput {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        const MAX_BYTES: usize = 1_048_576;
        if text.len() > MAX_BYTES.saturating_sub(self.0.len()) {
            return Err(std::fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}

#[cfg(feature = "rush")]
fn rush_export_error(code: &str, message: &str) -> String {
    format!(
        "{{\"ok\":false,\"code\":{},\"error\":{}}}",
        json_string(code),
        json_string(message)
    )
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

/// Stateful browser/application boundary for scheduled Rush functions.
#[cfg(feature = "rush")]
pub struct RushSchedule {
    scheduler: themoretheless_tokenizer_rush::OwnedCoroutineScheduler,
}
#[cfg(feature = "rush")]
impl RushSchedule {
    fn limits() -> themoretheless_tokenizer_rush::ExecutionLimits {
        themoretheless_tokenizer_rush::ExecutionLimits {
            max_collection_items: 10_000,
            max_string_bytes: 65_536,
            ..themoretheless_tokenizer_rush::ExecutionLimits::new(100_000)
        }
    }
    pub fn new(source: &str) -> Result<Self, String> {
        let scheduler =
            themoretheless_tokenizer_rush::OwnedCoroutineScheduler::new(source, Self::limits())
                .map_err(|e| e.message)?;
        Ok(Self { scheduler })
    }
    pub fn spawn(&mut self, name: &str) -> Result<String, String> {
        self.scheduler
            .spawn(name, &[], Self::limits())
            .map(|id| id.get().to_string())
            .map_err(|e| e.message)
    }
    pub fn emit(&mut self, name: &str) -> Result<usize, String> {
        self.scheduler.emit(name).map_err(|e| e.message)
    }
    pub fn cancel_all(&mut self) {
        self.scheduler.cancel_all()
    }
    pub fn next_deadline_ms(&self) -> Option<f64> {
        self.scheduler
            .next_deadline()
            .map(|t| t.as_secs_f64() * 1000.)
    }
    pub fn poll(&mut self, milliseconds: f64) -> Result<String, String> {
        use themoretheless_tokenizer_rush::{ScheduledState, WakeRequest};
        let now = std::time::Duration::try_from_secs_f64(milliseconds / 1000.)
            .map_err(|_| "Time must be finite and nonnegative".to_owned())?;
        let steps = self
            .scheduler
            .poll(now, Self::limits())
            .map_err(|e| e.message)?;
        let mut output = String::from("{\"steps\":[");
        for (index, step) in steps.into_iter().enumerate() {
            if index > 0 {
                output.push(',')
            }
            let state = match step.state {
                ScheduledState::Waiting(WakeRequest::After(delay)) => format!(
                    "\"state\":\"waiting\",\"afterMs\":{}",
                    delay.as_secs_f64() * 1000.
                ),
                ScheduledState::Waiting(WakeRequest::Event(event)) => {
                    format!("\"state\":\"waiting\",\"event\":{}", json_string(&event))
                }
                ScheduledState::Complete(value) => {
                    let value = bounded_rush_output(|out| write!(out, "{value:?}"))
                        .map_err(|_| "Text output byte limit exceeded".to_owned())?;
                    format!("\"state\":\"complete\",\"output\":{}", json_string(&value))
                }
                ScheduledState::Failed(error) => format!(
                    "\"state\":\"failed\",\"error\":{}",
                    json_string(&error.message)
                ),
            };
            write!(
                output,
                "{{\"id\":{}, {state}}}",
                json_string(&step.id.get().to_string())
            )
            .unwrap();
        }
        write!(output, "],\"empty\":{}}}", self.scheduler.is_empty()).unwrap();
        Ok(output)
    }
}
