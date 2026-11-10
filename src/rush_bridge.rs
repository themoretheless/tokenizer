//! Rush-specific playground analysis, refactoring, bounded execution, and
//! coroutine scheduling adapters extracted from [`crate::web_bridge`].

#[cfg(feature = "rush")]
use std::fmt::Write as _;

#[cfg(feature = "rush")]
use themoretheless_tokenizer_core::TokenLayer;

#[cfg(not(feature = "rush"))]
use crate::web_bridge::protocol_error;
#[cfg(feature = "rush")]
use crate::web_bridge::{
    host_error_payload, json_string, push_diagnostic, push_json_string, write_tokenization_prefix,
};

/// Tokenizes Rush source and emits the full playground JSON payload (tokens,
/// grammar diagnostics, execution diagnostics, member completions, builtins,
/// bindings, and resolved references) in a single lex + parse pass.
#[cfg(feature = "rush")]
pub(crate) fn tokenization_rush(mode: &str, layer: TokenLayer, source: &str) -> String {
    use std::borrow::Cow;
    use themoretheless_tokenizer_core::{HostAnalysisOptions, InputLimits};

    let default_dialect = themoretheless_tokenizer_rush::DESCRIPTOR
        .default_dialect
        .as_str();
    let dialect = if mode.is_empty() || mode == default_dialect {
        Cow::Borrowed(default_dialect)
    } else {
        Cow::Owned(mode.to_owned())
    };
    let opts = HostAnalysisOptions {
        dialect,
        limits: InputLimits::conservative(),
    };

    let analysis = match themoretheless_tokenizer_rush::analyze_playground(source, &opts, layer) {
        Ok(a) => a,
        Err(error) => return host_error_payload(&error),
    };

    let mut output =
        write_tokenization_prefix("rush", mode, layer.as_str(), source, &analysis.host);

    output.push_str(",\"executionDiagnostics\":[");
    for (index, diagnostic) in analysis.execution_diagnostics.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_diagnostic(
            &mut output,
            diagnostic.code,
            diagnostic.message,
            diagnostic.span.start,
            diagnostic.span.end,
        );
    }
    output.push(']');

    output.push_str(",\"memberCompletions\":[");
    for (index, completion) in analysis.member_completions.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let _ = write!(
            output,
            "{{\"start\":{},\"end\":{},\"members\":[",
            completion.name_span.start, completion.name_span.end
        );
        for (m_idx, member) in completion.members.iter().enumerate() {
            if m_idx > 0 {
                output.push(',');
            }
            push_json_string(&mut output, member);
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
        output.push_str("{\"name\":");
        push_json_string(&mut output, name);
        let _ = write!(
            output,
            ",\"minArgs\":{},\"maxArgs\":{}}}",
            arity.start(),
            arity.end()
        );
    }
    output.push(']');

    output.push_str(",\"bindings\":[");
    for (index, binding) in analysis.bindings.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str("{\"name\":");
        push_json_string(&mut output, &binding.name);
        let _ = write!(
            output,
            ",\"kind\":\"binding\",\"start\":{},\"end\":{},\"depth\":{},\"definition\":{{\"start\":{},\"end\":{}}},\"members\":[",
            binding.visible.start,
            binding.visible.end,
            binding.depth,
            binding.definition.start,
            binding.definition.end
        );
        for (m_idx, member) in binding.members.iter().enumerate() {
            if m_idx > 0 {
                output.push(',');
            }
            push_json_string(&mut output, member);
        }
        output.push_str("],\"memberPaths\":{");
        for (p_idx, (path, members)) in binding.member_paths.iter().enumerate() {
            if p_idx > 0 {
                output.push(',');
            }
            push_json_string(&mut output, path);
            output.push_str(":[");
            for (m_idx, member) in members.iter().enumerate() {
                if m_idx > 0 {
                    output.push(',');
                }
                push_json_string(&mut output, member);
            }
            output.push(']');
        }
        output.push_str("}}");
    }
    output.push(']');

    output.push_str(",\"references\":[");
    for (index, reference) in analysis.references.iter().enumerate() {
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
    output.push_str("]}");
    output
}

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
    let mut output = StringOutput(String::with_capacity(256));
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
                output.push(',');
            }
            let state = match step.state {
                ScheduledState::Waiting(WakeRequest::After(delay)) => format!(
                    "\"state\":\"waiting\",\"afterMs\":{}",
                    delay.as_secs_f64() * 1000.
                ),
                ScheduledState::Waiting(WakeRequest::Event(event)) => {
                    format!("\"state\":\"waiting\",\"event\":{}", json_string(&event))
                }
                ScheduledState::Waiting(WakeRequest::Task(id)) => {
                    format!("\"state\":\"waiting\",\"task\":{}", id.get())
                }
                ScheduledState::Waiting(WakeRequest::All(ids)) => {
                    let list = ids
                        .iter()
                        .map(|id| id.get().to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("\"state\":\"waiting\",\"taskAll\":[{list}]")
                }
                ScheduledState::Waiting(WakeRequest::Race(ids)) => {
                    let list = ids
                        .iter()
                        .map(|id| id.get().to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("\"state\":\"waiting\",\"taskRace\":[{list}]")
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
