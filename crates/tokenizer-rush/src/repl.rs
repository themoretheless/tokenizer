//! Interactive REPL session management and pretty printing for Rush.
use crate::{
    CancellationToken, ExecutionLimits, Program, RuntimeError, ScriptInstance, Value,
    sys::{MODULE_SOURCE as SYS_MODULE_SOURCE, SysHost, SysLimits},
};
use std::fmt::Write as _;

/// Outcome of evaluating an input chunk in the REPL.
#[derive(Debug, PartialEq)]
pub enum ReplOutcome {
    /// An expression was evaluated to a value.
    Value(String),
    /// A statement (e.g. declaration or import) executed with no return value.
    Void,
    /// The input is incomplete (e.g. unclosed bracket or block).
    Incomplete,
    /// A REPL directive was executed.
    Command(ReplCommand),
    /// An error occurred (syntax or runtime).
    Error(String),
}

/// Built-in REPL directive.
#[derive(Debug, PartialEq)]
pub enum ReplCommand {
    Help,
    Reset,
    Exit,
    Vars(Vec<(String, String)>),
}

/// Interactive REPL session holding persistent runtime state.
pub struct ReplSession {
    instance: ScriptInstance<'static, 'static>,
    token_ptr: *mut CancellationToken,
    allocated_sources: Vec<*mut str>,
    multiline_buffer: String,
}

impl Default for ReplSession {
    fn default() -> Self {
        Self::new().expect("Failed to initialize default REPL session")
    }
}

impl ReplSession {
    /// Create a new interactive REPL session with standard environment and `sys` module.
    pub fn new() -> Result<Self, String> {
        let token_box = Box::new(CancellationToken::default());
        let token_ptr = Box::into_raw(token_box);
        let token_ref: &'static CancellationToken = unsafe { &*token_ptr };

        let instance = Self::create_instance(token_ref)?;

        Ok(Self {
            instance,
            token_ptr,
            allocated_sources: Vec::new(),
            multiline_buffer: String::new(),
        })
    }

    fn create_instance(
        token: &'static CancellationToken,
    ) -> Result<ScriptInstance<'static, 'static>, String> {
        let sys = SysHost::with_args(SysLimits::default(), Vec::new());
        let sys_program =
            Program::compile(SYS_MODULE_SOURCE).map_err(|e| format!("sys: {}", e.message))?;
        let empty_program = Program::compile("").map_err(|e| format!("init: {}", e.message))?;

        // Leak the sys_program for static lifetime in REPL session
        let sys_boxed = Box::new(sys_program);
        let sys_ref: &'static Program<'static> = Box::leak(sys_boxed);
        let modules: [(&'static str, &'static Program<'static>); 1] = [("sys", sys_ref)];

        let limits = ExecutionLimits::new(1_000_000);
        let instance = empty_program
            .instantiate(limits, token, &[], &[], &sys.registrations, &modules)
            .map_err(|e| format!("instantiate: {}", e.message))?;
        Ok(instance)
    }

    /// Reset the session environment to a clean state.
    pub fn reset(&mut self) -> Result<(), String> {
        let token_ref: &'static CancellationToken = unsafe { &*self.token_ptr };
        self.instance = Self::create_instance(token_ref)?;
        self.multiline_buffer.clear();
        for ptr in self.allocated_sources.drain(..) {
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
        Ok(())
    }

    /// Current multiline accumulation buffer (empty if at primary prompt).
    pub fn is_accumulating(&self) -> bool {
        !self.multiline_buffer.is_empty()
    }

    /// Process a single line of input. Handles multi-line continuation, REPL commands,
    /// and code evaluation.
    pub fn eval_line(&mut self, line: &str) -> ReplOutcome {
        // If buffer was empty, check for top-level REPL commands
        if self.multiline_buffer.is_empty() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return ReplOutcome::Void;
            }
            if let Some(cmd) = self.parse_command(trimmed) {
                return cmd;
            }
        }

        // Accumulate into buffer
        if !self.multiline_buffer.is_empty() {
            self.multiline_buffer.push('\n');
        }
        self.multiline_buffer.push_str(line);

        // Check if input is complete
        if !is_input_complete(&self.multiline_buffer) {
            return ReplOutcome::Incomplete;
        }

        let full_input = std::mem::take(&mut self.multiline_buffer);
        self.eval_source(&full_input)
    }

    /// Force evaluation of a source string directly.
    pub fn eval_source(&mut self, source: &str) -> ReplOutcome {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            return ReplOutcome::Void;
        }

        let leaked_str: &'static str = Box::leak(trimmed.to_string().into_boxed_str());
        self.allocated_sources
            .push(leaked_str as *const str as *mut str);

        match self.instance.eval_chunk(leaked_str) {
            Ok(Some(val)) => ReplOutcome::Value(pretty_print_value(&val)),
            Ok(None) => ReplOutcome::Void,
            Err(err) => ReplOutcome::Error(format_repl_error(&err, leaked_str)),
        }
    }

    fn parse_command(&mut self, cmd: &str) -> Option<ReplOutcome> {
        match cmd {
            ":help" | "help" => Some(ReplOutcome::Command(ReplCommand::Help)),
            ":exit" | ":quit" | "exit" | "quit" => Some(ReplOutcome::Command(ReplCommand::Exit)),
            ":reset" => {
                if let Err(e) = self.reset() {
                    Some(ReplOutcome::Error(e))
                } else {
                    Some(ReplOutcome::Command(ReplCommand::Reset))
                }
            }
            ":vars" => {
                let vars = self
                    .instance
                    .list_variables()
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), pretty_print_value(&v)))
                    .collect();
                Some(ReplOutcome::Command(ReplCommand::Vars(vars)))
            }
            _ => None,
        }
    }
}

impl Drop for ReplSession {
    fn drop(&mut self) {
        if !self.token_ptr.is_null() {
            unsafe {
                drop(Box::from_raw(self.token_ptr));
            }
        }
        for ptr in self.allocated_sources.drain(..) {
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

/// Check whether the current input buffer has balanced delimiters and complete strings.
pub fn is_input_complete(input: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut in_comment = false;
    let mut quote_char = '"';
    let mut prev_char = ' ';

    for c in input.chars() {
        if in_comment {
            if c == '\n' {
                in_comment = false;
            }
            prev_char = c;
            continue;
        }
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == quote_char {
                in_string = false;
            }
            prev_char = c;
            continue;
        }
        if c == '/' && prev_char == '/' {
            in_comment = true;
            prev_char = c;
            continue;
        }
        match c {
            '"' | '\'' => {
                in_string = true;
                quote_char = c;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ => {}
        }
        prev_char = c;
    }

    depth == 0 && !in_string
}

fn format_repl_error(error: &RuntimeError, source: &str) -> String {
    if let Some(loc) = &error.location {
        let lines: Vec<&str> = source.lines().collect();
        if loc.line > 0 && loc.line <= lines.len() {
            let line_text = lines[loc.line - 1];
            let col = loc.column.saturating_sub(1);
            let caret = format!("{}^", " ".repeat(col));
            return format!(
                "Error (line {}, col {}): {}\n{}\n{}",
                loc.line, loc.column, error.message, line_text, caret
            );
        }
        format!(
            "Error (line {}, col {}): {}",
            loc.line, loc.column, error.message
        )
    } else {
        format!("Error: {}", error.message)
    }
}

/// Pretty print a Rush `Value` for display in the interactive REPL.
pub fn pretty_print_value(value: &Value<'_>) -> String {
    let mut out = String::new();
    format_value_display(value, &mut out, 0);
    out
}

fn format_value_display(value: &Value<'_>, out: &mut String, depth: usize) {
    if depth > 16 {
        out.push_str("...");
        return;
    }
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) {
                let _ = write!(out, "{:.0}", n);
            } else {
                let _ = write!(out, "{}", n);
            }
        }
        Value::Angle(a) => {
            let _ = write!(out, "{} rad", a);
        }
        Value::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(c),
                }
            }
            out.push('"');
        }
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_display(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::Tuple(items) => {
            out.push('(');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_display(item, out, depth + 1);
            }
            if items.len() == 1 {
                out.push(',');
            }
            out.push(')');
        }
        Value::Record(fields) => {
            out.push_str("{ ");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(k);
                out.push_str(": ");
                format_value_display(v, out, depth + 1);
            }
            out.push_str(" }");
        }
        Value::Vector(comps) => {
            match comps.len() {
                2 => out.push_str("vec2("),
                3 => out.push_str("vec3("),
                4 => out.push_str("vec4("),
                _ => out.push_str("vec("),
            }
            for (i, c) in comps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "{}", c);
            }
            out.push(')');
        }
        Value::Variant(tag, payload) => {
            out.push_str(tag);
            if !payload.is_empty() {
                out.push('(');
                for (i, item) in payload.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_value_display(item, out, depth + 1);
                }
                out.push(')');
            }
        }
        Value::UserData(data) => {
            out.push_str(&data.type_name);
            if let Some(var) = &data.variant {
                out.push('.');
                out.push_str(var);
            }
            out.push('(');
            for (i, item) in data.values.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_display(item, out, depth + 1);
            }
            out.push(')');
        }
        Value::Function(f) => {
            if let Some(name) = f.name() {
                let _ = write!(out, "<function {}>", name);
            } else {
                out.push_str("<function>");
            }
        }
        Value::BytecodeFunction(f) => {
            if let Some(name) = f.function.name.as_deref() {
                let _ = write!(out, "<function {}>", name);
            } else {
                out.push_str("<function>");
            }
        }
        Value::Builtin(b) => {
            let _ = write!(out, "<builtin {:?}>", b);
        }
        Value::Host(h) => {
            let _ = write!(out, "<host function {}>", h.name);
        }
        Value::HostObject(o) => {
            let _ = write!(out, "<host object {:?}>", o);
        }
        Value::Range { start, end, step } => {
            let _ = write!(out, "range({}, {}, {})", start, end, step);
        }
        Value::Mesh(_) => out.push_str("<Mesh>"),
        Value::Polygon(_) => out.push_str("<Polygon>"),
        Value::Matrix(_) => out.push_str("<Matrix4>"),
        Value::Quaternion(_) => out.push_str("<Quaternion>"),
        Value::Sequence(_) | Value::BytecodeSequence(_) => out.push_str("<sequence>"),
        Value::BytecodeIterator(_) => out.push_str("<iterator>"),
    }
}
