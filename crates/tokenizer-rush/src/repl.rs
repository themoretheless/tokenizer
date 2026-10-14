//! Interactive REPL session management and pretty printing for Rush.
use crate::runtime::ops::format_value_for_display;
use crate::{
    CancellationToken, ExecutionLimits, Program, RuntimeError, ScriptInstance, Value,
    sys::{MODULE_SOURCE as SYS_MODULE_SOURCE, SysHost, SysLimits},
};
use std::mem::ManuallyDrop;

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
///
/// `instance` is wrapped in `ManuallyDrop` so that its destructor runs strictly
/// *before* `allocated_sources`, `sys_ptr`, and `token_ptr` are deallocated in `reset()` and `Drop`.
pub struct ReplSession {
    instance: ManuallyDrop<ScriptInstance<'static, 'static>>,
    sys_ptr: *mut Program<'static>,
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
        let sys_program =
            Program::compile(SYS_MODULE_SOURCE).map_err(|e| format!("sys: {}", e.message))?;
        let sys_ptr = Box::into_raw(Box::new(sys_program));
        let sys_ref: &'static Program<'static> = unsafe { &*sys_ptr };

        let token_box = Box::new(CancellationToken::default());
        let token_ptr = Box::into_raw(token_box);
        let token_ref: &'static CancellationToken = unsafe { &*token_ptr };

        match Self::create_instance(token_ref, sys_ref) {
            Ok(instance) => Ok(Self {
                instance: ManuallyDrop::new(instance),
                sys_ptr,
                token_ptr,
                allocated_sources: Vec::new(),
                multiline_buffer: String::new(),
            }),
            Err(e) => {
                unsafe {
                    drop(Box::from_raw(sys_ptr));
                    drop(Box::from_raw(token_ptr));
                }
                Err(e)
            }
        }
    }

    fn create_instance(
        token: &'static CancellationToken,
        sys_ref: &'static Program<'static>,
    ) -> Result<ScriptInstance<'static, 'static>, String> {
        let sys = SysHost::with_args(SysLimits::default(), Vec::new());
        let empty_program = Program::compile("").map_err(|e| format!("init: {}", e.message))?;
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
        let sys_ref: &'static Program<'static> = unsafe { &*self.sys_ptr };
        let new_instance = Self::create_instance(token_ref, sys_ref)?;
        // Drop the old instance BEFORE freeing its borrowed allocated_sources.
        let old_instance = std::mem::replace(&mut *self.instance, new_instance);
        drop(old_instance);
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
        unsafe {
            // Drop ScriptInstance first while borrowed sources, sys_ptr, and token_ptr are still valid.
            ManuallyDrop::drop(&mut self.instance);
            for ptr in self.allocated_sources.drain(..) {
                drop(Box::from_raw(ptr));
            }
            if !self.sys_ptr.is_null() {
                drop(Box::from_raw(self.sys_ptr));
            }
            if !self.token_ptr.is_null() {
                drop(Box::from_raw(self.token_ptr));
            }
        }
    }
}

/// Check whether the current input buffer has balanced delimiters, complete strings, and closed block comments.
pub fn is_input_complete(input: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut quote_char = '"';

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
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
            continue;
        }
        if c == '/' {
            if chars.peek() == Some(&'/') {
                chars.next();
                in_line_comment = true;
                continue;
            }
            if chars.peek() == Some(&'*') {
                chars.next();
                in_block_comment = true;
                continue;
            }
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
    }

    depth == 0 && !in_string && !in_block_comment
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
///
/// Delegates to the shared `runtime::ops::format_value_for_display` at depth 1
/// so top-level strings are quoted while all other values share identical formatting.
pub fn pretty_print_value(value: &Value<'_>) -> String {
    let mut out = String::new();
    format_value_for_display(value, &mut out, 1);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repl_block_comment_completeness() {
        assert!(!is_input_complete("/* unclosed comment"));
        assert!(is_input_complete(
            "/* { unclosed brace inside comment */ 42"
        ));
    }

    #[test]
    fn repl_reset_and_drop_memory_safety() {
        let mut session = ReplSession::new().unwrap();
        assert_eq!(session.eval_line("let x = 10"), ReplOutcome::Void);
        assert_eq!(session.eval_line("x + 5"), ReplOutcome::Value("15".into()));
        session.reset().unwrap();
        assert!(matches!(session.eval_line("x"), ReplOutcome::Error(_)));
    }
}
