#![no_std]
#![deny(unsafe_code)]

//! rush script interpreter core — the executable semantics of the rush language.
//!
//! Pure structure: line classification, indentation blocks, the bounded
//! variable table, `and`/`or` chaining, `fn` definitions/calls and
//! `for name in ...` loops. Execution is the caller's job — this module
//! yields command segments and absorbs their exit status.
//! `no_std`, zero allocation, zero unsafe, all state fixed-size.
//!
//! Stage 3 control flow model: `fn` and `for` bodies are *captured* from
//! the source while deeper than their header, then replayed through a
//! bounded pending-line stack, so loops and calls need no source rewind and
//! no heap. Nested `fn`/`for` inside a captured body is rejected
//! (`NestedControl`); `if`/`else` nest freely.

use core::str;

pub const MAX_VARS: usize = 16;
pub const MAX_NAME: usize = 16;
pub const MAX_VALUE: usize = 64;
pub const MAX_DEPTH: usize = 8;
/// Longest command segment (one script line, matching `MAX_LINE`).
pub const MAX_SEGMENT: usize = 512;
/// Function definitions per script.
pub const MAX_FNS: usize = 8;
/// Formal parameters per definition.
pub const MAX_PARAMS: usize = 4;
/// Captured `fn`/`for` body size.
pub const MAX_BODY: usize = 768;
/// Expanded `for` word list size.
pub const MAX_LOOP_WORDS: usize = 256;
/// Pending-line stack depth (nested calls/loops).
pub const MAX_CALLS: usize = 4;
/// Longest replayable body line.
pub const MAX_BODY_LINE: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RushError {
    /// Block nesting deeper than `MAX_DEPTH`.
    TooDeep,
    /// More than `MAX_VARS` distinct variables.
    TooManyVariables,
    /// Variable name longer than `MAX_NAME` bytes.
    NameTooLong,
    /// Variable value longer than `MAX_VALUE` bytes after expansion.
    ValueTooLong,
    /// `name = value` where the name is not `[A-Za-z_][A-Za-z0-9_]*` or the
    /// value carries a dangling `$`.
    BadAssignment,
    /// `if` line that does not end in `:` or whose condition is malformed.
    BadCondition,
    /// `else:` without a matching open `if` at the same indent.
    ElseWithoutIf,
    /// Command line longer than `MAX_SEGMENT`.
    SegmentTooLong,
    /// More than `MAX_FNS` function definitions.
    TooManyFns,
    /// A captured `fn`/`for` body over `MAX_BODY` bytes, or one body line
    /// over `MAX_BODY_LINE`.
    BodyTooLong,
    /// `fn` header that is not `fn name [param...]:`.
    BadFnDef,
    /// `for` header that is not `for name in word...:`.
    BadFor,
    /// Call/loop replay stack deeper than `MAX_CALLS`.
    CallTooDeep,
    /// `fn`/`for` header inside a captured body.
    NestedControl,
    /// A loop word list over `MAX_LOOP_WORDS` bytes after expansion.
    LoopWordsTooLong,
    /// `match` header that is not `match <subject>:`.
    BadMatch,
    /// A line inside a `match` body that is not `<patterns> => <command>`.
    BadMatchArm,
    /// `foreach` header that is not `foreach name in <source>:`.
    BadForeach,
}

/// A captured `foreach` loop waiting for the host to run its source
/// command and hand the resulting words back via `provide_source_words`.
#[derive(Clone, Copy)]
struct ForEach {
    var: [u8; MAX_NAME],
    var_len: u8,
    source: [u8; MAX_LOOP_WORDS],
    source_len: u16,
    body: [u8; MAX_BODY],
    body_len: u16,
}

impl ForEach {
    const fn new() -> Self {
        ForEach {
            var: [0; MAX_NAME],
            var_len: 0,
            source: [0; MAX_LOOP_WORDS],
            source_len: 0,
            body: [0; MAX_BODY],
            body_len: 0,
        }
    }
}

/// What the caller should do after a `step`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step<'a> {
    /// Nothing to run (blank, comment, skipped block, assignment, control).
    Skip,
    /// Execute this command segment, then call `note_status`. Borrows the
    /// interpreter's internal chain buffer.
    Run(&'a str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChainOp {
    And,
    Or,
}

#[derive(Clone, Copy)]
struct Var {
    name: [u8; MAX_NAME],
    name_len: u8,
    value: [u8; MAX_VALUE],
    value_len: u8,
}

impl Var {
    const fn new() -> Self {
        Var {
            name: [0; MAX_NAME],
            name_len: 0,
            value: [0; MAX_VALUE],
            value_len: 0,
        }
    }
    fn name(&self) -> &str {
        str::from_utf8(self.name.get(..usize::from(self.name_len)).unwrap_or(&[])).unwrap_or("")
    }
    fn value(&self) -> &str {
        str::from_utf8(self.value.get(..usize::from(self.value_len)).unwrap_or(&[])).unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    If,
    Else,
    Match,
}

#[derive(Clone, Copy)]
struct Frame {
    indent: u16,
    kind: FrameKind,
    active: bool,
    taken: bool,
    parent_active: bool,
    /// `match` subject, expanded once at the header (unused by if/else).
    subject: [u8; MAX_BODY_LINE],
    subject_len: u16,
}

const EMPTY_FRAME: Frame = Frame {
    indent: 0,
    kind: FrameKind::If,
    active: false,
    taken: false,
    parent_active: true,
    subject: [0; MAX_BODY_LINE],
    subject_len: 0,
};

#[derive(Clone, Copy)]
struct FnDef {
    name: [u8; MAX_NAME],
    name_len: u8,
    params: [[u8; MAX_NAME]; MAX_PARAMS],
    param_lens: [u8; MAX_PARAMS],
    param_count: u8,
    body: [u8; MAX_BODY],
    body_len: u16,
}

impl FnDef {
    const fn new() -> Self {
        FnDef {
            name: [0; MAX_NAME],
            name_len: 0,
            params: [[0; MAX_NAME]; MAX_PARAMS],
            param_lens: [0; MAX_PARAMS],
            param_count: 0,
            body: [0; MAX_BODY],
            body_len: 0,
        }
    }
    fn name(&self) -> &str {
        str::from_utf8(self.name.get(..usize::from(self.name_len)).unwrap_or(&[])).unwrap_or("")
    }
}

/// A replayable line source: a function body (run once) or a loop body
/// (re-run per word, rebinding `var`).
#[derive(Clone, Copy)]
struct PendingFrame {
    buf: [u8; MAX_BODY],
    len: u16,
    pc: u16,
    is_loop: bool,
    var: [u8; MAX_NAME],
    var_len: u8,
    words: [u8; MAX_LOOP_WORDS],
    words_len: u16,
    word_index: u16,
}

impl PendingFrame {
    const fn new() -> Self {
        PendingFrame {
            buf: [0; MAX_BODY],
            len: 0,
            pc: 0,
            is_loop: false,
            var: [0; MAX_NAME],
            var_len: 0,
            words: [0; MAX_LOOP_WORDS],
            words_len: 0,
            word_index: 0,
        }
    }
    fn text(&self) -> &str {
        str::from_utf8(self.buf.get(..usize::from(self.len)).unwrap_or(&[])).unwrap_or("")
    }
    fn words_text(&self) -> &str {
        str::from_utf8(self.words.get(..usize::from(self.words_len)).unwrap_or(&[])).unwrap_or("")
    }
}

/// Capture-in-progress for a `fn`/`for` body.
#[derive(Clone, Copy)]
struct Capture {
    /// Indent of the header line.
    indent: u16,
    /// Indent of the first body line (all body lines are stored relative).
    base_indent: u16,
    buf: [u8; MAX_BODY],
    len: u16,
    is_fn: bool,
    /// `foreach`: the body waits for host-provided source words.
    foreach: bool,
    active: bool,
    /// `fn`: name and params. `for`/`foreach`: loop variable.
    name: [u8; MAX_NAME],
    name_len: u8,
    params: [[u8; MAX_NAME]; MAX_PARAMS],
    param_lens: [u8; MAX_PARAMS],
    param_count: u8,
    /// `for`: expanded word list. `foreach`: raw source command.
    words: [u8; MAX_LOOP_WORDS],
    words_len: u16,
}

impl Capture {
    const fn new() -> Self {
        Capture {
            indent: 0,
            base_indent: 0,
            buf: [0; MAX_BODY],
            len: 0,
            is_fn: false,
            foreach: false,
            active: false,
            name: [0; MAX_NAME],
            name_len: 0,
            params: [[0; MAX_NAME]; MAX_PARAMS],
            param_lens: [0; MAX_PARAMS],
            param_count: 0,
            words: [0; MAX_LOOP_WORDS],
            words_len: 0,
        }
    }
}

pub struct Interpreter {
    vars: [Var; MAX_VARS],
    var_count: usize,
    frames: [Frame; MAX_DEPTH],
    depth: usize,
    executed: u32,
    /// Remaining chain segments of the current line (copied, owned).
    chain: [u8; MAX_SEGMENT],
    chain_len: usize,
    chain_at: usize,
    chain_op: Option<ChainOp>,
    chain_skip: bool,
    fns: [FnDef; MAX_FNS],
    fn_count: usize,
    pending: [PendingFrame; MAX_CALLS],
    pending_len: usize,
    capture: Option<Capture>,
    foreach: Option<ForEach>,
    pushback: [u8; MAX_BODY_LINE],
    pushback_len: u16,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub const fn new() -> Self {
        Interpreter {
            vars: [Var::new(); MAX_VARS],
            var_count: 0,
            frames: [EMPTY_FRAME; MAX_DEPTH],
            depth: 0,
            executed: 0,
            chain: [0; MAX_SEGMENT],
            chain_len: 0,
            chain_at: 0,
            chain_op: None,
            chain_skip: false,
            fns: [FnDef::new(); MAX_FNS],
            fn_count: 0,
            pending: [PendingFrame::new(); MAX_CALLS],
            pending_len: 0,
            capture: None,
            foreach: None,
            pushback: [0; MAX_BODY_LINE],
            pushback_len: 0,
        }
    }

    /// Commands actually dispatched so far.
    pub fn executed(&self) -> u32 {
        self.executed
    }

    /// Look up a script variable.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars[..self.var_count]
            .iter()
            .find(|var| var.name() == name)
            .map(Var::value)
    }

    /// Iterate over `(name, value)` pairs (for building a parser environment).
    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars[..self.var_count]
            .iter()
            .map(|var| (var.name(), var.value()))
    }

    /// Pre-seed a variable (USER/HOME/STATUS from the session). Seeds beyond
    /// the table bound are silently dropped: the session trio comes first.
    pub fn seed(&mut self, name: &str, value: &str) {
        let _ = self.set_var(name, value);
    }

    /// Record the exit status of the last `Step::Run` segment: updates the
    /// `STATUS` variable and resolves short-circuit state.
    pub fn note_status(&mut self, ok: bool) {
        self.executed += 1;
        let _ = self.set_var("STATUS", if ok { "0" } else { "1" });
        self.chain_skip = match self.chain_op {
            Some(ChainOp::And) => !ok,
            Some(ChainOp::Or) => ok,
            None => false,
        };
    }

    /// True while the interpreter still has work without new input: a chain
    /// tail, replayed `fn`/`for` body lines, a stashed dedent line, or a
    /// `foreach` waiting for its source words.
    pub fn busy(&self) -> bool {
        self.chain_pending()
            || self.pending_len > 0
            || self.pushback_len > 0
            || self.foreach.is_some()
    }

    /// True while a chain tail is still buffered.
    pub fn chain_pending(&self) -> bool {
        self.chain_at < self.chain_len
    }

    /// End of source: closes an open capture (EOF ends the body). After
    /// this, drain `step("")` while `busy()`, then call `finish`.
    pub fn end_input(&mut self) -> Result<(), RushError> {
        if self.capture.is_some() {
            self.finish_capture()?;
        }
        Ok(())
    }

    /// A pending `foreach`, as `(variable, source command)`: the host must
    /// run the source, then call `provide_source_words` (or `fail_source`).
    pub fn source_request(&self) -> Option<(&str, &str)> {
        let each = self.foreach.as_ref()?;
        let var = str::from_utf8(each.var.get(..usize::from(each.var_len)).unwrap_or(&[]))
            .unwrap_or("");
        let source = str::from_utf8(
            each.source.get(..usize::from(each.source_len)).unwrap_or(&[]),
        )
        .unwrap_or("");
        Some((var, source))
    }

    /// Host answer to `source_request`: the whitespace-separated words the
    /// source produced. The body replays once per word, exactly like `for`.
    pub fn provide_source_words(&mut self, words: &str) -> Result<(), RushError> {
        let Some(each) = self.foreach.take() else {
            return Ok(());
        };
        if words.len() > MAX_LOOP_WORDS {
            return Err(RushError::LoopWordsTooLong);
        }
        if words.split_whitespace().next().is_none() || each.body_len == 0 {
            return Ok(());
        }
        if self.pending_len >= MAX_CALLS {
            return Err(RushError::CallTooDeep);
        }
        let frame = &mut self.pending[self.pending_len];
        *frame = PendingFrame::new();
        frame.buf = each.body;
        frame.len = each.body_len;
        frame.is_loop = true;
        frame.var = each.var;
        frame.var_len = each.var_len;
        if let Some(slot) = frame.words.get_mut(..words.len()) {
            slot.copy_from_slice(words.as_bytes());
        }
        frame.words_len = words.len() as u16;
        self.pending_len += 1;
        self.bind_loop_word()
    }

    /// Host could not run the source: drop the pending `foreach`.
    pub fn fail_source(&mut self) {
        self.foreach = None;
    }

    /// Feed the next physical line of the script.
    pub fn step(&mut self, line: &str) -> Result<Step<'_>, RushError> {
        if self.chain_pending() {
            return Ok(self.next_segment());
        }
        if self.pending_len > 0 {
            let mut scratch = [0u8; MAX_BODY_LINE];
            if let Some(len) = self.take_pending_line(&mut scratch)? {
                let text = str::from_utf8(scratch.get(..len).unwrap_or(&[])).unwrap_or("");
                return self.process_line(text, true);
            }
        }
        if self.pushback_len > 0 {
            let len = usize::from(self.pushback_len);
            self.pushback_len = 0;
            let mut scratch = [0u8; MAX_BODY_LINE];
            if let (Some(dst), Some(src)) = (
                scratch.get_mut(..len),
                self.pushback.get(..len),
            ) {
                dst.copy_from_slice(src);
            }
            let text = str::from_utf8(scratch.get(..len).unwrap_or(&[])).unwrap_or("");
            return self.process_line(text, false);
        }
        // A waiting `foreach` is serviced by the host, not by input lines.
        if self.foreach.is_some() {
            return Ok(Step::Skip);
        }
        self.process_line(line, false)
    }

    /// End of script: indentation blocks close implicitly at EOF
    /// (Python-style), so this only reports the executed count.
    pub fn finish(&mut self) -> Result<u32, RushError> {
        self.depth = 0;
        Ok(self.executed)
    }

    // ─── line processing ────────────────────────────────────────────────

    fn process_line(&mut self, line: &str, nested: bool) -> Result<Step<'_>, RushError> {
        let text = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = text.trim_start_matches([' ', '\t']);

        // Capture mode consumes deeper lines verbatim.
        if self.capture.is_some() {
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return Ok(Step::Skip);
            }
            let indent = indent_width(text);
            if indent > self.capture_indent() {
                self.capture_line(text)?;
                return Ok(Step::Skip);
            }
            self.finish_capture()?;
            // Stash the dedent line: pending replays (loop iterations,
            // calls) must drain before it runs.
            let bytes = text.as_bytes();
            let len = bytes.len().min(MAX_BODY_LINE);
            if let Some(slot) = self.pushback.get_mut(..len) {
                slot.copy_from_slice(bytes.get(..len).unwrap_or(&[]));
            }
            self.pushback_len = len as u16;
            return Ok(Step::Skip);
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(Step::Skip);
        }
        let indent = indent_width(text);

        // `else:` attaches to the matching open `if` before any popping.
        if trimmed == "else:" {
            return self.step_else(indent);
        }

        // A line at or left of a frame's indent closes that frame.
        while self.depth > 0 && indent <= usize::from(self.frames[self.depth - 1].indent) {
            self.depth -= 1;
        }

        let active = self.is_active();

        // A body line of an open `match` frame is an arm.
        if self.depth > 0 && self.frames[self.depth - 1].kind == FrameKind::Match {
            return self.step_match_arm(trimmed);
        }

        if let Some(condition) = trimmed
            .strip_prefix("if ")
            .or_else(|| trimmed.strip_prefix("if\t"))
        {
            let Some(condition) = condition.trim_end().strip_suffix(':') else {
                return Err(RushError::BadCondition);
            };
            let Some(taken) = eval_condition(condition.trim(), self) else {
                return Err(RushError::BadCondition);
            };
            if self.depth >= MAX_DEPTH {
                return Err(RushError::TooDeep);
            }
            self.frames[self.depth] = Frame {
                indent: indent as u16,
                kind: FrameKind::If,
                active: active && taken,
                taken,
                parent_active: active,
                subject: [0; MAX_BODY_LINE],
                subject_len: 0,
            };
            self.depth += 1;
            return Ok(Step::Skip);
        }

        // `match subject:` pushes an extended frame; the subject expands
        // once and arms are single-line `patterns => command` body lines.
        if let Some(subject) = trimmed
            .strip_prefix("match ")
            .or_else(|| trimmed.strip_prefix("match\t"))
        {
            return self.start_match(subject.trim_end(), indent, active);
        }

        // `fn` and `for` headers open captures even in dead branches (the
        // body must be consumed), but replay only when active.
        if trimmed.starts_with("fn ") || trimmed.starts_with("fn\t") {
            return self.start_fn_capture(trimmed, indent, active, nested);
        }
        if trimmed.starts_with("for ") || trimmed.starts_with("for\t") {
            return self.start_for_capture(trimmed, indent, active, nested);
        }
        if trimmed.starts_with("foreach ") || trimmed.starts_with("foreach\t") {
            return self.start_foreach_capture(trimmed, indent, active, nested);
        }

        if !active {
            return Ok(Step::Skip);
        }

        if let Some((name, value)) = parse_assignment(trimmed) {
            self.assign_expand(name, value)?;
            return Ok(Step::Skip);
        }

        // Function call: first word is a defined name.
        if let Some(call) = self.find_fn(trimmed) {
            self.start_call(call, trimmed)?;
            return Ok(Step::Skip);
        }

        // Plain command line: stage it (and any chain tail).
        self.begin_chain(trimmed)?;
        Ok(self.next_segment())
    }

    // ─── blocks ─────────────────────────────────────────────────────────

    fn step_else(&mut self, indent: usize) -> Result<Step<'static>, RushError> {
        let Some(top) = self.depth.checked_sub(1) else {
            return Err(RushError::ElseWithoutIf);
        };
        let frame = self.frames[top];
        if frame.kind != FrameKind::If || usize::from(frame.indent) != indent {
            return Err(RushError::ElseWithoutIf);
        }
        self.frames[top].kind = FrameKind::Else;
        self.frames[top].active = frame.parent_active && !frame.taken;
        self.frames[top].taken = true;
        Ok(Step::Skip)
    }

    /// `match subject:` — expand the subject once and push the frame.
    fn start_match(
        &mut self,
        header: &str,
        indent: usize,
        active: bool,
    ) -> Result<Step<'static>, RushError> {
        let Some(expr) = header.strip_suffix(':') else {
            return Err(RushError::BadMatch);
        };
        let expr = expr.trim();
        if expr.is_empty() {
            return Err(RushError::BadMatch);
        }
        if self.depth >= MAX_DEPTH {
            return Err(RushError::TooDeep);
        }
        let mut frame = Frame {
            indent: indent as u16,
            kind: FrameKind::Match,
            active,
            taken: false,
            parent_active: active,
            subject: [0; MAX_BODY_LINE],
            subject_len: 0,
        };
        if active {
            let len = self.expand_to(expr, &mut frame.subject)?;
            frame.subject_len = len as u16;
        }
        self.frames[self.depth] = frame;
        self.depth += 1;
        Ok(Step::Skip)
    }

    /// One `patterns => command` line inside a `match` body.
    fn step_match_arm(&mut self, trimmed: &str) -> Result<Step<'_>, RushError> {
        let frame = self.frames[self.depth - 1];
        let Some((patterns, command)) = trimmed.split_once("=>") else {
            return Err(RushError::BadMatchArm);
        };
        let command = command.trim();
        if patterns.trim().is_empty() || command.is_empty() {
            return Err(RushError::BadMatchArm);
        }
        if !frame.active || frame.taken {
            return Ok(Step::Skip);
        }
        let subject = str::from_utf8(
            frame.subject.get(..usize::from(frame.subject_len)).unwrap_or(&[]),
        )
        .unwrap_or("");
        if !match_patterns(patterns, subject) {
            return Ok(Step::Skip);
        }
        self.frames[self.depth - 1].taken = true;
        self.begin_chain(command)?;
        Ok(self.next_segment())
    }

    fn is_active(&self) -> bool {
        self.depth == 0 || self.frames[self.depth - 1].active
    }

    // ─── variables ──────────────────────────────────────────────────────

    fn set_var(&mut self, name: &str, value: &str) -> Result<(), RushError> {
        if name.len() > MAX_NAME {
            return Err(RushError::NameTooLong);
        }
        if value.len() > MAX_VALUE {
            return Err(RushError::ValueTooLong);
        }
        let index = match self.vars[..self.var_count]
            .iter()
            .position(|var| var.name() == name)
        {
            Some(index) => index,
            None => {
                if self.var_count >= MAX_VARS {
                    return Err(RushError::TooManyVariables);
                }
                let index = self.var_count;
                self.var_count += 1;
                index
            }
        };
        let var = &mut self.vars[index];
        var.name = [0; MAX_NAME];
        var.name_len = name.len() as u8;
        if let Some(slot) = var.name.get_mut(..name.len()) {
            slot.copy_from_slice(name.as_bytes());
        }
        var.value = [0; MAX_VALUE];
        var.value_len = value.len() as u8;
        if let Some(slot) = var.value.get_mut(..value.len()) {
            slot.copy_from_slice(value.as_bytes());
        }
        Ok(())
    }

    /// Expand `$name` occurrences from the table into `out`; returns the
    /// length. Unknown names expand to empty, matching `parse`.
    fn expand_to(&self, input: &str, out: &mut [u8]) -> Result<usize, RushError> {
        if !input.contains('$') {
            let Some(slot) = out.get_mut(..input.len()) else {
                return Err(RushError::ValueTooLong);
            };
            slot.copy_from_slice(input.as_bytes());
            return Ok(input.len());
        }
        let mut len = 0usize;
        let mut rest = input;
        while let Some(at) = rest.find('$') {
            let literal = &rest[..at];
            let after = &rest[at + 1..];
            let name_len = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            if name_len == 0 {
                return Err(RushError::BadAssignment);
            }
            let expansion = self.get(&after[..name_len]).unwrap_or("");
            for chunk in [literal, expansion] {
                let end = len + chunk.len();
                let Some(slot) = out.get_mut(len..end) else {
                    return Err(RushError::ValueTooLong);
                };
                slot.copy_from_slice(chunk.as_bytes());
                len = end;
            }
            rest = &after[name_len..];
        }
        let end = len + rest.len();
        let Some(slot) = out.get_mut(len..end) else {
            return Err(RushError::ValueTooLong);
        };
        slot.copy_from_slice(rest.as_bytes());
        Ok(end)
    }

    /// Assignment with `$name` expansion from the table.
    fn assign_expand(&mut self, name: &str, value: &str) -> Result<(), RushError> {
        let mut buf = [0u8; MAX_VALUE];
        let len = self.expand_to(value, &mut buf)?;
        let text = str::from_utf8(buf.get(..len).unwrap_or(&[]))
            .map_err(|_| RushError::ValueTooLong)?;
        self.set_var(name, text)
    }

    // ─── fn / for capture and replay ────────────────────────────────────

    fn capture_indent(&self) -> usize {
        self.capture.map_or(0, |cap| usize::from(cap.indent))
    }

    fn start_fn_capture(&mut self, trimmed: &str, indent: usize, active: bool, nested: bool) -> Result<Step<'static>, RushError> {
        if nested {
            return Err(RushError::NestedControl);
        }
        let header = trimmed[2..].trim();
        let Some(header) = header.strip_suffix(':') else {
            return Err(RushError::BadFnDef);
        };
        let mut words = header.split_whitespace();
        let Some(name) = words.next() else {
            return Err(RushError::BadFnDef);
        };
        if !is_ident(name) || self.fn_count >= MAX_FNS && active {
            return Err(if is_ident(name) {
                RushError::TooManyFns
            } else {
                RushError::BadFnDef
            });
        }
        let mut cap = Capture::new();
        cap.indent = indent as u16;
        cap.is_fn = true;
        cap.active = active;
        if name.len() > MAX_NAME {
            return Err(RushError::BadFnDef);
        }
        if let Some(slot) = cap.name.get_mut(..name.len()) {
            slot.copy_from_slice(name.as_bytes());
        }
        cap.name_len = name.len() as u8;
        for param in words {
            if !is_ident(param) || param.len() > MAX_NAME {
                return Err(RushError::BadFnDef);
            }
            let index = usize::from(cap.param_count);
            if index >= MAX_PARAMS {
                return Err(RushError::BadFnDef);
            }
            if let Some(slot) = cap.params[index].get_mut(..param.len()) {
                slot.copy_from_slice(param.as_bytes());
            }
            cap.param_lens[index] = param.len() as u8;
            cap.param_count += 1;
        }
        self.capture = Some(cap);
        Ok(Step::Skip)
    }

    fn start_for_capture(&mut self, trimmed: &str, indent: usize, active: bool, nested: bool) -> Result<Step<'static>, RushError> {
        if nested {
            return Err(RushError::NestedControl);
        }
        let header = trimmed[3..].trim();
        let Some(header) = header.strip_suffix(':') else {
            return Err(RushError::BadFor);
        };
        let Some((var, words)) = header.split_once(" in ") else {
            return Err(RushError::BadFor);
        };
        let var = var.trim();
        if !is_ident(var) || var.len() > MAX_NAME {
            return Err(RushError::BadFor);
        }
        let mut cap = Capture::new();
        cap.indent = indent as u16;
        cap.active = active;
        if let Some(slot) = cap.name.get_mut(..var.len()) {
            slot.copy_from_slice(var.as_bytes());
        }
        cap.name_len = var.len() as u8;
        // Expand the word list now: it is evaluated once, at the header.
        let mut buf = [0u8; MAX_LOOP_WORDS];
        let len = self
            .expand_to(words.trim(), &mut buf)
            .map_err(|_| RushError::LoopWordsTooLong)?;
        if let Some(slot) = cap.words.get_mut(..len) {
            slot.copy_from_slice(buf.get(..len).unwrap_or(&[]));
        }
        cap.words_len = len as u16;
        self.capture = Some(cap);
        Ok(Step::Skip)
    }

    /// `foreach name in <source>:` — like `for`, but the word list comes
    /// from the host running `<source>` after the body is captured.
    fn start_foreach_capture(&mut self, trimmed: &str, indent: usize, active: bool, nested: bool) -> Result<Step<'static>, RushError> {
        if nested {
            return Err(RushError::NestedControl);
        }
        let header = trimmed[7..].trim();
        let Some(header) = header.strip_suffix(':') else {
            return Err(RushError::BadForeach);
        };
        let Some((var, source)) = header.split_once(" in ") else {
            return Err(RushError::BadForeach);
        };
        let var = var.trim();
        let source = source.trim();
        if !is_ident(var) || var.len() > MAX_NAME || source.is_empty() {
            return Err(RushError::BadForeach);
        }
        if source.len() > MAX_LOOP_WORDS {
            return Err(RushError::LoopWordsTooLong);
        }
        let mut cap = Capture::new();
        cap.indent = indent as u16;
        cap.active = active;
        cap.foreach = true;
        if let Some(slot) = cap.name.get_mut(..var.len()) {
            slot.copy_from_slice(var.as_bytes());
        }
        cap.name_len = var.len() as u8;
        if let Some(slot) = cap.words.get_mut(..source.len()) {
            slot.copy_from_slice(source.as_bytes());
        }
        cap.words_len = source.len() as u16;
        self.capture = Some(cap);
        Ok(Step::Skip)
    }

    fn capture_line(&mut self, text: &str) -> Result<(), RushError> {
        let Some(cap) = self.capture.as_mut() else {
            return Ok(());
        };
        let indent = indent_width(text);
        if cap.len == 0 {
            cap.base_indent = indent as u16;
        }
        let relative = indent.saturating_sub(usize::from(cap.base_indent));
        if text.len() > MAX_BODY_LINE {
            return Err(RushError::BodyTooLong);
        }
        // Store the line re-indented relative to the body base.
        let body = text.trim_start_matches([' ', '\t']);
        let prefix: usize = relative;
        let total = prefix + body.len() + 1;
        let start = usize::from(cap.len);
        let Some(slot) = cap.buf.get_mut(start..start + total) else {
            return Err(RushError::BodyTooLong);
        };
        for cell in slot.iter_mut().take(prefix) {
            *cell = b' ';
        }
        slot[prefix..prefix + body.len()].copy_from_slice(body.as_bytes());
        slot[prefix + body.len()] = b'\n';
        cap.len = (start + total) as u16;
        Ok(())
    }

    fn finish_capture(&mut self) -> Result<(), RushError> {
        let Some(cap) = self.capture.take() else {
            return Ok(());
        };
        if !cap.active {
            return Ok(());
        }
        if cap.is_fn {
            if self.fn_count >= MAX_FNS {
                return Err(RushError::TooManyFns);
            }
            let def = &mut self.fns[self.fn_count];
            *def = FnDef::new();
            def.name = cap.name;
            def.name_len = cap.name_len;
            def.params = cap.params;
            def.param_lens = cap.param_lens;
            def.param_count = cap.param_count;
            def.body = cap.buf;
            def.body_len = cap.len;
            self.fn_count += 1;
            return Ok(());
        }
        if cap.foreach {
            // Hand the loop to the host: it runs the source command and
            // answers with `provide_source_words`.
            let mut each = ForEach::new();
            each.var = cap.name;
            each.var_len = cap.name_len;
            each.source = cap.words;
            each.source_len = cap.words_len;
            each.body = cap.buf;
            each.body_len = cap.len;
            self.foreach = Some(each);
            return Ok(());
        }
        // `for`: replay the body once per word.
        if cap.words_len == 0 || cap.len == 0 {
            return Ok(());
        }
        if self.pending_len >= MAX_CALLS {
            return Err(RushError::CallTooDeep);
        }
        let frame = &mut self.pending[self.pending_len];
        *frame = PendingFrame::new();
        frame.buf = cap.buf;
        frame.len = cap.len;
        frame.is_loop = true;
        frame.var = cap.name;
        frame.var_len = cap.name_len;
        frame.words = cap.words;
        frame.words_len = cap.words_len;
        self.pending_len += 1;
        // Bind the first word immediately.
        self.bind_loop_word()
    }

    /// Bind the current word of the top loop frame to its variable.
    fn bind_loop_word(&mut self) -> Result<(), RushError> {
        if self.pending_len == 0 {
            return Ok(());
        }
        let frame = self.pending[self.pending_len - 1];
        let var = str::from_utf8(frame.var.get(..usize::from(frame.var_len)).unwrap_or(&[]))
            .unwrap_or("");
        let word = match nth_word(frame.words_text(), usize::from(frame.word_index)) {
            Some(word) => word,
            None => return Ok(()),
        };
        let mut var_buf = [0u8; MAX_NAME];
        let mut word_buf = [0u8; MAX_VALUE];
        let var_len = var.len().min(MAX_NAME);
        let word_len = word.len().min(MAX_VALUE);
        if let (Some(v), Some(w)) = (
            var_buf.get_mut(..var_len),
            word_buf.get_mut(..word_len),
        ) {
            v.copy_from_slice(var.as_bytes());
            w.copy_from_slice(word.as_bytes());
        }
        let var = str::from_utf8(var_buf.get(..var_len).unwrap_or(&[])).unwrap_or("");
        let word = str::from_utf8(word_buf.get(..word_len).unwrap_or(&[])).unwrap_or("");
        self.set_var(var, word)
    }

    /// Pull the next replayable line from the pending stack into `out`.
    fn take_pending_line(&mut self, out: &mut [u8; MAX_BODY_LINE]) -> Result<Option<usize>, RushError> {
        loop {
            if self.pending_len == 0 {
                return Ok(None);
            }
            let frame = &mut self.pending[self.pending_len - 1];
            if usize::from(frame.pc) < usize::from(frame.len) {
                // Copy the line out first, then advance pc (borrows end).
                let (line_len, consumed) = {
                    let text = frame.text();
                    let rest = text.get(usize::from(frame.pc)..).unwrap_or("");
                    let nl = rest.find('\n').unwrap_or(rest.len());
                    let line = rest.get(..nl).unwrap_or("");
                    let Some(slot) = out.get_mut(..line.len()) else {
                        return Err(RushError::BodyTooLong);
                    };
                    slot.copy_from_slice(line.as_bytes());
                    (line.len(), (nl + 1).min(rest.len()))
                };
                frame.pc += consumed as u16;
                if line_len == 0 {
                    continue;
                }
                return Ok(Some(line_len));
            }
            if frame.is_loop {
                frame.word_index += 1;
                let has_more = nth_word(frame.words_text(), usize::from(frame.word_index)).is_some();
                if has_more {
                    frame.pc = 0;
                    self.bind_loop_word()?;
                    continue;
                }
            }
            self.pending_len -= 1;
        }
    }

    fn find_fn(&self, trimmed: &str) -> Option<usize> {
        let name = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .split(['(', '|'])
            .next()
            .unwrap_or("");
        if name.is_empty() {
            return None;
        }
        self.fns[..self.fn_count]
            .iter()
            .position(|def| def.name() == name)
    }

    fn start_call(&mut self, fn_index: usize, trimmed: &str) -> Result<(), RushError> {
        if self.pending_len >= MAX_CALLS {
            return Err(RushError::CallTooDeep);
        }
        // Collect argument words first (borrows the line, not self).
        let mut args: [[u8; MAX_VALUE]; MAX_PARAMS] = [[0; MAX_VALUE]; MAX_PARAMS];
        let mut arg_lens = [0u8; MAX_PARAMS];
        let mut argc = 0usize;
        for word in trimmed.split_whitespace().skip(1) {
            if argc >= MAX_PARAMS {
                break;
            }
            let len = word.len().min(MAX_VALUE);
            if let Some(slot) = args[argc].get_mut(..len) {
                slot.copy_from_slice(word.as_bytes());
            }
            arg_lens[argc] = len as u8;
            argc += 1;
        }
        let def = self.fns[fn_index];
        let frame = &mut self.pending[self.pending_len];
        *frame = PendingFrame::new();
        frame.buf = def.body;
        frame.len = def.body_len;
        self.pending_len += 1;
        // Bind parameters.
        for param in 0..usize::from(def.param_count) {
            let name = str::from_utf8(
                def.params[param]
                    .get(..usize::from(def.param_lens[param]))
                    .unwrap_or(&[]),
            )
            .unwrap_or("");
            let value = if param < argc {
                str::from_utf8(args[param].get(..usize::from(arg_lens[param])).unwrap_or(&[]))
                    .unwrap_or("")
            } else {
                ""
            };
            let mut name_buf = [0u8; MAX_NAME];
            let mut value_buf = [0u8; MAX_VALUE];
            let name_len = name.len().min(MAX_NAME);
            let value_len = value.len().min(MAX_VALUE);
            if let (Some(n), Some(v)) = (
                name_buf.get_mut(..name_len),
                value_buf.get_mut(..value_len),
            ) {
                n.copy_from_slice(name.as_bytes());
                v.copy_from_slice(value.as_bytes());
            }
            let name = str::from_utf8(name_buf.get(..name_len).unwrap_or(&[])).unwrap_or("");
            let value = str::from_utf8(value_buf.get(..value_len).unwrap_or(&[])).unwrap_or("");
            self.set_var(name, value)?;
        }
        Ok(())
    }

    // ─── and/or chains ──────────────────────────────────────────────────

    fn begin_chain(&mut self, line: &str) -> Result<(), RushError> {
        if line.len() > MAX_SEGMENT {
            return Err(RushError::SegmentTooLong);
        }
        self.chain = [0; MAX_SEGMENT];
        if let Some(slot) = self.chain.get_mut(..line.len()) {
            slot.copy_from_slice(line.as_bytes());
        }
        self.chain_len = line.len();
        self.chain_at = 0;
        self.chain_op = None;
        self.chain_skip = false;
        Ok(())
    }

    fn chain_text(&self) -> &str {
        str::from_utf8(self.chain.get(..self.chain_len).unwrap_or(&[])).unwrap_or("")
    }

    fn next_segment(&mut self) -> Step<'_> {
        loop {
            if self.chain_at >= self.chain_len {
                return Step::Skip;
            }
            let (head_off, head_len, next_at, next_op) = {
                let text = self.chain_text();
                let Some(rest) = text.get(self.chain_at..) else {
                    return Step::Skip;
                };
                let head_off = self.chain_at + (rest.len() - rest.trim_start().len());
                match split_chain(rest) {
                    Some((head, op, tail_at)) => (
                        head_off,
                        head.trim().len(),
                        self.chain_at + tail_at,
                        Some(op),
                    ),
                    None => (head_off, rest.trim().len(), self.chain_len, None),
                }
            };
            self.chain_at = next_at;
            self.chain_op = next_op;
            if self.chain_skip {
                self.chain_skip = false;
                continue;
            }
            if head_len == 0 {
                return Step::Skip;
            }
            let Some(bytes) = self.chain.get(head_off..head_off + head_len) else {
                return Step::Skip;
            };
            return match str::from_utf8(bytes) {
                Ok(segment) => Step::Run(segment),
                Err(_) => Step::Skip,
            };
        }
    }
}

fn is_ident(text: &str) -> bool {
    let mut chars = text.bytes();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return false,
    }
    chars.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn nth_word(text: &str, index: usize) -> Option<&str> {
    text.split_whitespace().nth(index)
}

/// Does any `|`-separated alternative match the subject? `_` is the
/// wildcard; surrounding single or double quotes are stripped.
fn match_patterns(patterns: &str, subject: &str) -> bool {
    patterns.split('|').any(|alt| {
        let alt = alt.trim();
        let alt = alt
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .or_else(|| {
                alt.strip_prefix('\'')
                    .and_then(|inner| inner.strip_suffix('\''))
            })
            .unwrap_or(alt);
        alt == "_" || alt == subject
    })
}

fn indent_width(line: &str) -> usize {
    let mut width = 0usize;
    for byte in line.bytes() {
        match byte {
            b' ' => width += 1,
            b'\t' => width += 4,
            _ => break,
        }
    }
    width
}

/// `name = value` at line start (not `==`/`!=`/`<=`/`>=`, not a call).
/// Returns the raw (unexpanded) value, trimmed.
fn parse_assignment(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    if line.as_bytes().get(eq + 1) == Some(&b'=') {
        return None;
    }
    if eq > 0
        && matches!(
            line.as_bytes().get(eq.wrapping_sub(1)),
            Some(b'!' | b'<' | b'>' | b'=')
        )
    {
        return None;
    }
    let name = line.get(..eq)?.trim_end();
    if !is_ident(name) {
        return None;
    }
    let value = line.get(eq + 1..)?.trim();
    Some((name, value))
}

/// Split `a and b` / `a && b` into `(head, op, tail_offset)`, honouring
/// single/double quotes. Word operators must be standalone words.
fn split_chain(line: &str) -> Option<(&str, ChainOp, usize)> {
    let bytes = line.as_bytes();
    let mut quote = 0u8;
    let mut at = 0usize;
    while at < bytes.len() {
        let byte = bytes[at];
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
            at += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = byte;
                at += 1;
            }
            b'&' if bytes.get(at + 1) == Some(&b'&') => {
                let head = line.get(..at)?.trim_end();
                if head.is_empty() {
                    return None;
                }
                return Some((head, ChainOp::And, at + 2));
            }
            b'|' if bytes.get(at + 1) == Some(&b'|') => {
                let head = line.get(..at)?.trim_end();
                if head.is_empty() {
                    return None;
                }
                return Some((head, ChainOp::Or, at + 2));
            }
            b' ' | b'\t' => {
                let end = at + 1;
                let word_end = end
                    + bytes
                        .get(end..)?
                        .iter()
                        .take_while(|b| !b.is_ascii_whitespace())
                        .count();
                let word = line.get(end..word_end)?;
                let bounded_after = line
                    .get(word_end..)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']));
                let op = match (word, bounded_after) {
                    ("and", true) => Some(ChainOp::And),
                    ("or", true) => Some(ChainOp::Or),
                    _ => None,
                };
                if let Some(op) = op {
                    let head = line.get(..at)?.trim_end();
                    if head.is_empty() {
                        return None;
                    }
                    return Some((head, op, word_end));
                }
                at = end;
            }
            _ => at += 1,
        }
    }
    None
}

/// `not? A ( = | != | < | > ) B` or a single truthy word. `$name` expands
/// from the table; both-numeric compares numerically. `None` = malformed.
fn eval_condition(condition: &str, interp: &Interpreter) -> Option<bool> {
    let (negated, body) = match condition.strip_prefix("not ") {
        Some(rest) => (true, rest.trim()),
        None => (false, condition),
    };
    if body.is_empty() {
        return None;
    }
    let mut words = body.split_whitespace();
    let result = match (words.next(), words.next(), words.next(), words.next()) {
        (Some(only), None, None, None) => {
            let value = operand(only, interp);
            !value.is_empty() && value != "0"
        }
        (Some(left), Some(op), Some(right), None) => {
            let left = operand(left, interp);
            let right = operand(right, interp);
            let numeric = parse_u64(left).zip(parse_u64(right));
            match (op, numeric) {
                ("=", _) => left == right,
                ("!=", _) => left != right,
                ("<", Some((a, b))) => a < b,
                ("<", None) => left < right,
                (">", Some((a, b))) => a > b,
                (">", None) => left > right,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(result != negated)
}

fn operand<'a>(word: &'a str, interp: &'a Interpreter) -> &'a str {
    if let Some(name) = word.strip_prefix('$') {
        return interp.get(name).unwrap_or("");
    }
    word
}

fn parse_u64(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
