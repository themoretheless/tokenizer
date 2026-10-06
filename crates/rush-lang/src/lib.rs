#![no_std]
#![deny(unsafe_code)]

//! rush script interpreter core — the executable semantics of the rush language.
//!
//! Pure structure: line classification, indentation blocks, the bounded
//! variable table and `and`/`or` chaining. Execution is the caller's job —
//! this module yields command segments and absorbs their exit status.
//! `no_std`, zero allocation, zero unsafe, all state fixed-size.

use core::str;

pub const MAX_VARS: usize = 16;
pub const MAX_NAME: usize = 16;
pub const MAX_VALUE: usize = 64;
pub const MAX_DEPTH: usize = 8;
/// Longest command segment (one script line, matching `MAX_LINE`).
pub const MAX_SEGMENT: usize = 512;

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
}

/// What the caller should do after a `step`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step<'a> {
    /// Nothing to run (blank, comment, skipped block, assignment, if/else).
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
}

#[derive(Clone, Copy)]
struct Frame {
    indent: u16,
    kind: FrameKind,
    active: bool,
    taken: bool,
    parent_active: bool,
}

const EMPTY_FRAME: Frame = Frame {
    indent: 0,
    kind: FrameKind::If,
    active: false,
    taken: false,
    parent_active: true,
};

pub struct Interpreter {
    vars: [Var; MAX_VARS],
    var_count: usize,
    frames: [Frame; MAX_DEPTH],
    depth: usize,
    executed: u32,
    /// Remaining chain segments of the current line (copied, owned).
    chain: [u8; MAX_SEGMENT],
    chain_len: usize,
    /// Offset of the next unconsumed segment inside `chain`.
    chain_at: usize,
    /// Operator that joins the previous segment to the next one.
    chain_op: Option<ChainOp>,
    /// True when short-circuit dropped the next segment.
    chain_skip: bool,
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

    /// True while a chain tail is still buffered (call `step` with an empty
    /// line to drain it, or use `next_in_chain`).
    pub fn chain_pending(&self) -> bool {
        self.chain_at < self.chain_len
    }

    /// Feed the next physical line of the script. Chain tails from the
    /// previous line are drained first; pass lines in order until
    /// `chain_pending()` is false, then advance to the next script line.
    pub fn step(&mut self, line: &str) -> Result<Step<'_>, RushError> {
        if self.chain_pending() {
            return Ok(self.next_segment());
        }
        let text = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = text.trim_start_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(Step::Skip);
        }
        let indent = indent_width(text);

        // `else:` attaches to the matching open `if` before any popping.
        if trimmed == "else:" {
            return self.step_else(indent);
        }

        // A line at or left of a frame's indent closes that frame (the
        // frame indent is the `if` line itself; its body is deeper).
        while self.depth > 0 && indent <= usize::from(self.frames[self.depth - 1].indent) {
            self.depth -= 1;
        }

        let active = self.is_active();

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
            };
            self.depth += 1;
            return Ok(Step::Skip);
        }

        if !active {
            return Ok(Step::Skip);
        }

        if let Some((name, value)) = parse_assignment(trimmed) {
            self.assign_expand(name, value)?;
            return Ok(Step::Skip);
        }

        // Plain command line: stage it (and any chain tail) and yield the
        // first segment.
        self.begin_chain(trimmed)?;
        Ok(self.next_segment())
    }

    /// End of script: indentation blocks close implicitly at EOF
    /// (Python-style), so this only reports the executed count.
    pub fn finish(&mut self) -> Result<u32, RushError> {
        self.depth = 0;
        Ok(self.executed)
    }

    fn step_else(&mut self, indent: usize) -> Result<Step<'static>, RushError> {
        let Some(top) = self.depth.checked_sub(1) else {
            return Err(RushError::ElseWithoutIf);
        };
        let frame = self.frames[top];
        if frame.kind != FrameKind::If || usize::from(frame.indent) != indent {
            return Err(RushError::ElseWithoutIf);
        }
        self.frames[top] = Frame {
            indent: indent as u16,
            kind: FrameKind::Else,
            active: frame.parent_active && !frame.taken,
            taken: true,
            parent_active: frame.parent_active,
        };
        Ok(Step::Skip)
    }

    fn is_active(&self) -> bool {
        self.depth == 0 || self.frames[self.depth - 1].active
    }

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

    /// Assignment with `$name` expansion from the table into the bounded
    /// value buffer. Unknown names expand to empty, matching `parse`.
    fn assign_expand(&mut self, name: &str, value: &str) -> Result<(), RushError> {
        if !value.contains('$') {
            return self.set_var(name, value);
        }
        let mut buf = [0u8; MAX_VALUE];
        let mut len = 0usize;
        let mut rest = value;
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
                let Some(slot) = buf.get_mut(len..end) else {
                    return Err(RushError::ValueTooLong);
                };
                slot.copy_from_slice(chunk.as_bytes());
                len = end;
            }
            rest = &after[name_len..];
        }
        let end = len + rest.len();
        let Some(slot) = buf.get_mut(len..end) else {
            return Err(RushError::ValueTooLong);
        };
        slot.copy_from_slice(rest.as_bytes());
        let text = str::from_utf8(buf.get(..end).unwrap_or(&[]))
            .map_err(|_| RushError::ValueTooLong)?;
        self.set_var(name, text)
    }

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
            // Compute the head span and the following state without holding
            // a borrow across the mutation.
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
                // Short-circuited: drop this segment, keep draining.
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
    let mut chars = name.bytes();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    if !chars.all(|b| b.is_ascii_alphanumeric() || b == b'_') {
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
