//! Flat-frame Bytecode VM and AST-to-bytecode compiler for Rush.
//!
//! Provides high-performance, flat stack-frame execution of compiled Rush code,
//! bypassing recursive AST walking and Environment map cloning overhead.

use crate::{
    Builtin, Expr, ExprKind, InterpolationPart, RuntimeError, Stmt, StmtKind, Value,
    builtin_catalog,
    runtime::{exact_remainder, format_value_for_display},
};
use std::collections::{BTreeMap, HashMap};
use themoretheless_tokenizer_core::Span;

/// Opcodes executed by the stack-based Bytecode Virtual Machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Opcode {
    /// Load constant from constant pool.
    Constant(u32),
    /// Push `null`.
    Null,
    /// Push `true`.
    True,
    /// Push `false`.
    False,
    /// Discard top value from stack.
    Pop,
    /// Duplicate top value on stack.
    Dup,
    /// Read local variable at stack slot.
    GetLocal(u32),
    /// Write top-of-stack to local variable slot.
    SetLocal(u32),
    /// Read global variable by name constant index.
    GetGlobal(u32),
    /// Write top-of-stack to global variable by name constant index.
    SetGlobal(u32),
    /// Binary addition / string concatenation / vector addition.
    Add,
    /// Binary subtraction.
    Sub,
    /// Binary multiplication.
    Mul,
    /// Binary division.
    Div,
    /// Binary remainder.
    Mod,
    /// Unary negation.
    Neg,
    /// Unary logical NOT.
    Not,
    /// Equality comparison.
    Equal,
    /// Inequality comparison.
    NotEqual,
    /// Less than.
    Less,
    /// Less than or equal.
    LessEqual,
    /// Greater than.
    Greater,
    /// Greater than or equal.
    GreaterEqual,
    /// Unconditional jump to target instruction offset.
    Jump(usize),
    /// Jump if top of stack is false / falsy.
    JumpIfFalse(usize),
    /// Jump if top of stack is true / truthy.
    JumpIfTrue(usize),
    /// Pop N items and push `Value::List`.
    BuildList(usize),
    /// Pop N items and push `Value::Tuple`.
    BuildTuple(usize),
    /// Pop 2*N items and push `Value::Record`.
    BuildRecord(usize),
    /// Pop index, pop container, push indexed element.
    IndexGet,
    /// Mutate container at local slot by index: pops index and new value.
    IndexSet(u32),
    /// Pop container, push member by field name constant index.
    MemberGet(u32),
    /// Mutate member of container at local slot by field constant index: pops new value.
    MemberSet(u32, u32),
    /// Pop N parts and interpolate into a single string.
    Interpolate(usize),
    /// Call callee with N arguments.
    Call(usize),
    /// Return from execution.
    Return,
}

/// A compiled bytecode compilation unit containing instructions and constants.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub code: Vec<Opcode>,
    pub constants: Vec<Value<'static>>,
    pub spans: Vec<Span>,
    pub local_count: usize,
}

impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&mut self, op: Opcode, span: Span) -> usize {
        let idx = self.code.len();
        self.code.push(op);
        self.spans.push(span);
        idx
    }

    pub fn add_constant(&mut self, value: Value<'static>) -> u32 {
        for (i, c) in self.constants.iter().enumerate() {
            if c == &value {
                return i as u32;
            }
        }
        let idx = self.constants.len() as u32;
        self.constants.push(value);
        idx
    }

    pub fn patch_jump(&mut self, instruction_idx: usize, target: usize) {
        match &mut self.code[instruction_idx] {
            Opcode::Jump(t) | Opcode::JumpIfFalse(t) | Opcode::JumpIfTrue(t) => {
                *t = target;
            }
            _ => panic!("Expected jump instruction at index {instruction_idx}"),
        }
    }
}

/// A self-contained executable bytecode program.
#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeProgram {
    pub chunk: Chunk,
}

impl BytecodeProgram {
    /// Compile source Rush code into a `BytecodeProgram`.
    pub fn compile(source: &str) -> Result<Self, RuntimeError> {
        let parsed = crate::parse(source);
        if !parsed.is_valid() {
            let diagnostic = parsed.diagnostics.first();
            let span = diagnostic.map_or(Span::new(0, 0), |d| d.span);
            let message = diagnostic.map_or("Syntax error", |d| d.message);
            return Err(RuntimeError {
                module: None,
                span,
                message: message.to_string(),
                stack: Vec::new(),
                location: None,
            });
        }
        let mut compiler = Compiler::new();
        compiler.compile_module(&parsed.module.items)?;
        Ok(BytecodeProgram {
            chunk: compiler.chunk,
        })
    }

    /// Execute the compiled bytecode with the specified step budget.
    pub fn execute(&self, budget: usize) -> Result<Value<'static>, RuntimeError> {
        let mut vm = Vm::new(budget);
        vm.run(&self.chunk)
    }
}

/// Convenience entry point to compile and run Rush code via the Bytecode VM.
pub fn evaluate_bytecode(source: &str, budget: usize) -> Result<Value<'static>, RuntimeError> {
    let program = BytecodeProgram::compile(source)?;
    program.execute(budget)
}

struct LoopContext {
    start_ip: usize,
    break_jumps: Vec<usize>,
}

struct Compiler<'s> {
    chunk: Chunk,
    scopes: Vec<HashMap<String, u32>>,
    local_count: u32,
    loops: Vec<LoopContext>,
    _phantom: std::marker::PhantomData<&'s ()>,
}

impl<'s> Compiler<'s> {
    fn new() -> Self {
        Self {
            chunk: Chunk::new(),
            scopes: vec![HashMap::new()],
            local_count: 0,
            loops: Vec::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    fn error(&self, span: Span, message: impl Into<String>) -> RuntimeError {
        RuntimeError {
            module: None,
            span,
            message: message.into(),
            stack: Vec::new(),
            location: None,
        }
    }

    fn resolve_local(&self, name: &str) -> Option<u32> {
        for scope in self.scopes.iter().rev() {
            if let Some(&slot) = scope.get(name) {
                return Some(slot);
            }
        }
        None
    }

    fn compile_module(&mut self, items: &[Stmt<'s>]) -> Result<(), RuntimeError> {
        self.compile_block(items, true)?;
        let end_span = items.last().map_or(Span::new(0, 0), |s| s.span);
        self.chunk.emit(Opcode::Return, end_span);
        self.chunk.local_count = self.local_count as usize;
        Ok(())
    }

    fn compile_block(&mut self, items: &[Stmt<'s>], keep_result: bool) -> Result<(), RuntimeError> {
        if items.is_empty() {
            if keep_result {
                self.chunk.emit(Opcode::Null, Span::new(0, 0));
            }
            return Ok(());
        }
        for (i, stmt) in items.iter().enumerate() {
            let is_last = i + 1 == items.len();
            self.compile_stmt(stmt, is_last && keep_result)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt<'s>, keep_result: bool) -> Result<(), RuntimeError> {
        match &stmt.kind {
            StmtKind::Declaration { name, value, .. } => {
                self.compile_expr(value)?;
                let slot = self.local_count;
                self.local_count += 1;
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.text.to_string(), slot);
                }
                self.chunk.emit(Opcode::SetLocal(slot), stmt.span);
                self.chunk.emit(Opcode::Pop, stmt.span);
                if keep_result {
                    self.chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.compile_expr(condition)?;
                let false_jump = self.chunk.emit(Opcode::JumpIfFalse(0), stmt.span);
                self.compile_block(&then_block.stmts, keep_result)?;
                let end_jump = self.chunk.emit(Opcode::Jump(0), stmt.span);
                self.chunk.patch_jump(false_jump, self.chunk.code.len());
                if let Some(else_b) = else_block {
                    self.compile_block(&else_b.stmts, keep_result)?;
                } else if keep_result {
                    self.chunk.emit(Opcode::Null, stmt.span);
                }
                self.chunk.patch_jump(end_jump, self.chunk.code.len());
            }
            StmtKind::While { condition, body } => {
                let start_ip = self.chunk.code.len();
                self.compile_expr(condition)?;
                let exit_jump = self.chunk.emit(Opcode::JumpIfFalse(0), stmt.span);
                self.loops.push(LoopContext {
                    start_ip,
                    break_jumps: Vec::new(),
                });
                self.compile_block(&body.stmts, false)?;
                self.chunk.emit(Opcode::Jump(start_ip), stmt.span);
                let loop_ctx = self.loops.pop().unwrap();
                let exit_target = self.chunk.code.len();
                self.chunk.patch_jump(exit_jump, exit_target);
                for brk in loop_ctx.break_jumps {
                    self.chunk.patch_jump(brk, exit_target);
                }
                if keep_result {
                    self.chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Break => {
                let Some(loop_ctx) = self.loops.last_mut() else {
                    return Err(self.error(stmt.span, "break outside of loop"));
                };
                let jmp = self.chunk.emit(Opcode::Jump(0), stmt.span);
                loop_ctx.break_jumps.push(jmp);
                if keep_result {
                    self.chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Continue => {
                let Some(loop_ctx) = self.loops.last() else {
                    return Err(self.error(stmt.span, "continue outside of loop"));
                };
                self.chunk.emit(Opcode::Jump(loop_ctx.start_ip), stmt.span);
                if keep_result {
                    self.chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Return(value) => {
                if let Some(val) = value {
                    self.compile_expr(val)?;
                } else {
                    self.chunk.emit(Opcode::Null, stmt.span);
                }
                self.chunk.emit(Opcode::Return, stmt.span);
            }
            StmtKind::Expr(expr) => {
                self.compile_expr(expr)?;
                if !keep_result {
                    self.chunk.emit(Opcode::Pop, stmt.span);
                }
            }
            _ => {
                return Err(
                    self.error(stmt.span, "Statement not supported in bytecode compilation")
                );
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr<'s>) -> Result<(), RuntimeError> {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Number(text) => {
                let clean = text.replace('_', "");
                let n: f64 = clean
                    .parse()
                    .map_err(|_| self.error(span, "Invalid number"))?;
                let idx = self.chunk.add_constant(Value::Number(n));
                self.chunk.emit(Opcode::Constant(idx), span);
            }
            ExprKind::String(text) => {
                let mut decoded = String::new();
                for c in crate::string_literal::characters(text) {
                    let ch = c.map_err(|e| self.error(span, e))?;
                    decoded.push(ch);
                }
                let idx = self.chunk.add_constant(Value::String(decoded));
                self.chunk.emit(Opcode::Constant(idx), span);
            }
            ExprKind::Bool(b) => {
                if *b {
                    self.chunk.emit(Opcode::True, span);
                } else {
                    self.chunk.emit(Opcode::False, span);
                }
            }
            ExprKind::Null => {
                self.chunk.emit(Opcode::Null, span);
            }
            ExprKind::Name(name) => {
                if let Some(slot) = self.resolve_local(name.text) {
                    self.chunk.emit(Opcode::GetLocal(slot), span);
                } else if let Some((_, builtin)) =
                    builtin_catalog().iter().find(|(n, _)| *n == name.text)
                {
                    let idx = self.chunk.add_constant(Value::Builtin(*builtin));
                    self.chunk.emit(Opcode::Constant(idx), span);
                } else {
                    let name_idx = self
                        .chunk
                        .add_constant(Value::String(name.text.to_string()));
                    self.chunk.emit(Opcode::GetGlobal(name_idx), span);
                }
            }
            ExprKind::Unary { operator, value } => {
                self.compile_expr(value)?;
                match *operator {
                    "-" => {
                        self.chunk.emit(Opcode::Neg, span);
                    }
                    "!" => {
                        self.chunk.emit(Opcode::Not, span);
                    }
                    _ => return Err(self.error(span, "Unsupported unary operator")),
                }
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                if *operator == "&&" {
                    self.compile_expr(left)?;
                    self.chunk.emit(Opcode::Dup, span);
                    let false_jump = self.chunk.emit(Opcode::JumpIfFalse(0), span);
                    self.chunk.emit(Opcode::Pop, span);
                    self.compile_expr(right)?;
                    self.chunk.patch_jump(false_jump, self.chunk.code.len());
                    return Ok(());
                }
                if *operator == "||" {
                    self.compile_expr(left)?;
                    self.chunk.emit(Opcode::Dup, span);
                    let true_jump = self.chunk.emit(Opcode::JumpIfTrue(0), span);
                    self.chunk.emit(Opcode::Pop, span);
                    self.compile_expr(right)?;
                    self.chunk.patch_jump(true_jump, self.chunk.code.len());
                    return Ok(());
                }

                self.compile_expr(left)?;
                self.compile_expr(right)?;
                match *operator {
                    "+" => self.chunk.emit(Opcode::Add, span),
                    "-" => self.chunk.emit(Opcode::Sub, span),
                    "*" => self.chunk.emit(Opcode::Mul, span),
                    "/" => self.chunk.emit(Opcode::Div, span),
                    "%" => self.chunk.emit(Opcode::Mod, span),
                    "==" => self.chunk.emit(Opcode::Equal, span),
                    "!=" => self.chunk.emit(Opcode::NotEqual, span),
                    "<" => self.chunk.emit(Opcode::Less, span),
                    "<=" => self.chunk.emit(Opcode::LessEqual, span),
                    ">" => self.chunk.emit(Opcode::Greater, span),
                    ">=" => self.chunk.emit(Opcode::GreaterEqual, span),
                    _ => return Err(self.error(span, "Unsupported binary operator")),
                };
            }
            ExprKind::Assign {
                target,
                value,
                operator,
            } => match &target.kind {
                ExprKind::Name(name) => {
                    if *operator == "=" {
                        self.compile_expr(value)?;
                    } else {
                        let base_op = operator.strip_suffix('=').unwrap_or(operator);
                        if let Some(slot) = self.resolve_local(name.text) {
                            self.chunk.emit(Opcode::GetLocal(slot), target.span);
                        } else {
                            let name_idx = self
                                .chunk
                                .add_constant(Value::String(name.text.to_string()));
                            self.chunk.emit(Opcode::GetGlobal(name_idx), target.span);
                        }
                        self.compile_expr(value)?;
                        match base_op {
                            "+" => self.chunk.emit(Opcode::Add, span),
                            "-" => self.chunk.emit(Opcode::Sub, span),
                            "*" => self.chunk.emit(Opcode::Mul, span),
                            "/" => self.chunk.emit(Opcode::Div, span),
                            "%" => self.chunk.emit(Opcode::Mod, span),
                            _ => return Err(self.error(span, "Unsupported assignment operator")),
                        };
                    }
                    if let Some(slot) = self.resolve_local(name.text) {
                        self.chunk.emit(Opcode::SetLocal(slot), target.span);
                    } else {
                        let name_idx = self
                            .chunk
                            .add_constant(Value::String(name.text.to_string()));
                        self.chunk.emit(Opcode::SetGlobal(name_idx), target.span);
                    }
                }
                ExprKind::Index { object, index } => {
                    let ExprKind::Name(obj_name) = &object.kind else {
                        return Err(self.error(
                            object.span,
                            "Index assignment requires a local variable target",
                        ));
                    };
                    let Some(slot) = self.resolve_local(obj_name.text) else {
                        return Err(
                            self.error(object.span, "Variable not found for index assignment")
                        );
                    };
                    self.compile_expr(index)?;
                    self.compile_expr(value)?;
                    self.chunk.emit(Opcode::IndexSet(slot), target.span);
                    self.chunk.emit(Opcode::GetLocal(slot), target.span);
                }
                ExprKind::Member { object, field } => {
                    let ExprKind::Name(obj_name) = &object.kind else {
                        return Err(self.error(
                            object.span,
                            "Member assignment requires a local variable target",
                        ));
                    };
                    let Some(slot) = self.resolve_local(obj_name.text) else {
                        return Err(
                            self.error(object.span, "Variable not found for member assignment")
                        );
                    };
                    self.compile_expr(value)?;
                    let field_idx = self
                        .chunk
                        .add_constant(Value::String(field.text.to_string()));
                    self.chunk
                        .emit(Opcode::MemberSet(slot, field_idx), target.span);
                    self.chunk.emit(Opcode::GetLocal(slot), target.span);
                }
                _ => return Err(self.error(target.span, "Unsupported assignment expression")),
            },
            ExprKind::Index { object, index } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.chunk.emit(Opcode::IndexGet, span);
            }
            ExprKind::Member { object, field } => {
                self.compile_expr(object)?;
                let field_idx = self
                    .chunk
                    .add_constant(Value::String(field.text.to_string()));
                self.chunk.emit(Opcode::MemberGet(field_idx), span);
            }
            ExprKind::List(items) => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.chunk.emit(Opcode::BuildList(items.len()), span);
            }
            ExprKind::Tuple(items) => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.chunk.emit(Opcode::BuildTuple(items.len()), span);
            }
            ExprKind::Map(entries) => {
                for (key, val) in entries {
                    match &key.kind {
                        ExprKind::Name(n) => {
                            let idx = self.chunk.add_constant(Value::String(n.text.to_string()));
                            self.chunk.emit(Opcode::Constant(idx), key.span);
                        }
                        _ => self.compile_expr(key)?,
                    }
                    self.compile_expr(val)?;
                }
                self.chunk.emit(Opcode::BuildRecord(entries.len()), span);
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                self.compile_expr(condition)?;
                let false_jump = self.chunk.emit(Opcode::JumpIfFalse(0), span);
                self.compile_expr(then_value)?;
                let end_jump = self.chunk.emit(Opcode::Jump(0), span);
                self.chunk.patch_jump(false_jump, self.chunk.code.len());
                self.compile_expr(else_value)?;
                self.chunk.patch_jump(end_jump, self.chunk.code.len());
            }
            ExprKind::Call { callee, arguments } => {
                self.compile_expr(callee)?;
                for arg in arguments {
                    self.compile_expr(arg)?;
                }
                self.chunk.emit(Opcode::Call(arguments.len()), span);
            }
            ExprKind::Pipeline { input, stages } => {
                self.compile_expr(input)?;
                for stage in stages {
                    match &stage.kind {
                        ExprKind::Call { callee, arguments } => {
                            let tmp_slot = self.local_count;
                            self.local_count += 1;
                            self.chunk.emit(Opcode::SetLocal(tmp_slot), stage.span);
                            self.chunk.emit(Opcode::Pop, stage.span);
                            self.compile_expr(callee)?;
                            self.chunk.emit(Opcode::GetLocal(tmp_slot), stage.span);
                            for arg in arguments {
                                self.compile_expr(arg)?;
                            }
                            self.chunk
                                .emit(Opcode::Call(arguments.len() + 1), stage.span);
                        }
                        _ => {
                            let tmp_slot = self.local_count;
                            self.local_count += 1;
                            self.chunk.emit(Opcode::SetLocal(tmp_slot), stage.span);
                            self.chunk.emit(Opcode::Pop, stage.span);
                            self.compile_expr(stage)?;
                            self.chunk.emit(Opcode::GetLocal(tmp_slot), stage.span);
                            self.chunk.emit(Opcode::Call(1), stage.span);
                        }
                    }
                }
            }
            ExprKind::Interpolate(parts) => {
                for part in parts {
                    match part {
                        InterpolationPart::Literal(s) => {
                            let idx = self.chunk.add_constant(Value::String(s.clone()));
                            self.chunk.emit(Opcode::Constant(idx), span);
                        }
                        InterpolationPart::Expr(sub) => {
                            self.compile_expr(sub)?;
                        }
                    }
                }
                self.chunk.emit(Opcode::Interpolate(parts.len()), span);
            }
            _ => {
                return Err(self.error(span, "Expression not supported in bytecode compilation"));
            }
        }
        Ok(())
    }
}

/// The flat stack Bytecode Virtual Machine runtime.
pub struct Vm {
    fuel: usize,
    globals: HashMap<String, Value<'static>>,
}

impl Vm {
    pub fn new(budget: usize) -> Self {
        Self {
            fuel: budget,
            globals: HashMap::new(),
        }
    }

    fn error(&self, span: Span, message: impl Into<String>) -> RuntimeError {
        RuntimeError {
            module: None,
            span,
            message: message.into(),
            stack: Vec::new(),
            location: None,
        }
    }

    pub fn run(&mut self, chunk: &Chunk) -> Result<Value<'static>, RuntimeError> {
        let mut ip = 0;
        let mut stack: Vec<Value<'static>> = Vec::with_capacity(chunk.local_count.max(64));
        stack.resize(chunk.local_count, Value::Null);

        while ip < chunk.code.len() {
            if self.fuel == 0 {
                let span = chunk.spans.get(ip).copied().unwrap_or(Span::new(0, 0));
                return Err(self.error(span, "Execution limit exceeded"));
            }
            self.fuel -= 1;

            let op = &chunk.code[ip];
            let span = chunk.spans[ip];
            ip += 1;

            match op {
                Opcode::Constant(idx) => {
                    let val = chunk.constants[*idx as usize].clone();
                    stack.push(val);
                }
                Opcode::Null => stack.push(Value::Null),
                Opcode::True => stack.push(Value::Bool(true)),
                Opcode::False => stack.push(Value::Bool(false)),
                Opcode::Pop => {
                    stack.pop();
                }
                Opcode::Dup => {
                    let top = stack.last().cloned().unwrap_or(Value::Null);
                    stack.push(top);
                }
                Opcode::GetLocal(slot) => {
                    let val = stack.get(*slot as usize).cloned().unwrap_or(Value::Null);
                    stack.push(val);
                }
                Opcode::SetLocal(slot) => {
                    let val = stack.last().cloned().unwrap_or(Value::Null);
                    let idx = *slot as usize;
                    if idx >= stack.len() {
                        stack.resize(idx + 1, Value::Null);
                    }
                    stack[idx] = val;
                }
                Opcode::GetGlobal(name_idx) => {
                    let name = match &chunk.constants[*name_idx as usize] {
                        Value::String(s) => s.as_str(),
                        _ => unreachable!(),
                    };
                    if let Some(val) = self.globals.get(name) {
                        stack.push(val.clone());
                    } else if let Some((_, builtin)) =
                        builtin_catalog().iter().find(|(n, _)| *n == name)
                    {
                        stack.push(Value::Builtin(*builtin));
                    } else {
                        return Err(self.error(span, format!("Undefined variable '{name}'")));
                    }
                }
                Opcode::SetGlobal(name_idx) => {
                    let name = match &chunk.constants[*name_idx as usize] {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let val = stack.last().cloned().unwrap_or(Value::Null);
                    self.globals.insert(name, val);
                }
                Opcode::Add => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Number(a + b)),
                        (Value::String(a), Value::String(b)) => {
                            stack.push(Value::String(format!("{a}{b}")));
                        }
                        (Value::Vector(a), Value::Vector(b)) if a.len() == b.len() => {
                            let res = a.iter().zip(&b).map(|(x, y)| x + y).collect();
                            stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operands for +")),
                    }
                }
                Opcode::Sub => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Number(a - b)),
                        (Value::Vector(a), Value::Vector(b)) if a.len() == b.len() => {
                            let res = a.iter().zip(&b).map(|(x, y)| x - y).collect();
                            stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operands for -")),
                    }
                }
                Opcode::Mul => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Number(a * b)),
                        (Value::Vector(a), Value::Number(b)) => {
                            let res = a.iter().map(|x| x * b).collect();
                            stack.push(Value::Vector(res));
                        }
                        (Value::Number(a), Value::Vector(b)) => {
                            let res = b.iter().map(|x| x * a).collect();
                            stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operands for *")),
                    }
                }
                Opcode::Div => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            if b == 0.0 {
                                return Err(self.error(span, "Division by zero"));
                            }
                            stack.push(Value::Number(a / b));
                        }
                        _ => return Err(self.error(span, "Invalid operands for /")),
                    }
                }
                Opcode::Mod => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            if b == 0.0 {
                                return Err(self.error(span, "Division by zero"));
                            }
                            stack.push(Value::Number(exact_remainder(a, b)));
                        }
                        _ => return Err(self.error(span, "Invalid operands for %")),
                    }
                }
                Opcode::Neg => {
                    let val = stack.pop().unwrap();
                    match val {
                        Value::Number(n) => stack.push(Value::Number(-n)),
                        Value::Vector(v) => {
                            let res = v.into_iter().map(|x| -x).collect();
                            stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operand for negation")),
                    }
                }
                Opcode::Not => {
                    let val = stack.pop().unwrap();
                    let truthy = is_truthy(&val);
                    stack.push(Value::Bool(!truthy));
                }
                Opcode::Equal => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    stack.push(Value::Bool(left == right));
                }
                Opcode::NotEqual => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    stack.push(Value::Bool(left != right));
                }
                Opcode::Less => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Bool(a < b)),
                        (Value::String(a), Value::String(b)) => stack.push(Value::Bool(a < b)),
                        _ => return Err(self.error(span, "Invalid operands for <")),
                    }
                }
                Opcode::LessEqual => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Bool(a <= b)),
                        (Value::String(a), Value::String(b)) => stack.push(Value::Bool(a <= b)),
                        _ => return Err(self.error(span, "Invalid operands for <=")),
                    }
                }
                Opcode::Greater => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Bool(a > b)),
                        (Value::String(a), Value::String(b)) => stack.push(Value::Bool(a > b)),
                        _ => return Err(self.error(span, "Invalid operands for >")),
                    }
                }
                Opcode::GreaterEqual => {
                    let right = stack.pop().unwrap();
                    let left = stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => stack.push(Value::Bool(a >= b)),
                        (Value::String(a), Value::String(b)) => stack.push(Value::Bool(a >= b)),
                        _ => return Err(self.error(span, "Invalid operands for >=")),
                    }
                }
                Opcode::Jump(target) => {
                    ip = *target;
                }
                Opcode::JumpIfFalse(target) => {
                    let val = stack.pop().unwrap();
                    if !is_truthy(&val) {
                        ip = *target;
                    }
                }
                Opcode::JumpIfTrue(target) => {
                    let val = stack.pop().unwrap();
                    if is_truthy(&val) {
                        ip = *target;
                    }
                }
                Opcode::BuildList(count) => {
                    let start = stack.len() - *count;
                    let items = stack.drain(start..).collect();
                    stack.push(Value::List(items));
                }
                Opcode::BuildTuple(count) => {
                    let start = stack.len() - *count;
                    let items = stack.drain(start..).collect();
                    stack.push(Value::Tuple(items));
                }
                Opcode::BuildRecord(count) => {
                    let start = stack.len() - (*count * 2);
                    let drained: Vec<_> = stack.drain(start..).collect();
                    let mut fields = BTreeMap::new();
                    for chunk in drained.as_chunks::<2>().0 {
                        let key = match &chunk[0] {
                            Value::String(s) => s.clone(),
                            _ => return Err(self.error(span, "Record key must be a string")),
                        };
                        fields.insert(key, chunk[1].clone());
                    }
                    stack.push(Value::Record(fields));
                }
                Opcode::IndexGet => {
                    let idx = stack.pop().unwrap();
                    let obj = stack.pop().unwrap();
                    match (&obj, &idx) {
                        (Value::List(items), Value::Number(n)) => {
                            let i = *n as usize;
                            if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                stack.push(items[i].clone());
                            } else {
                                return Err(self.error(span, "Index out of bounds"));
                            }
                        }
                        (Value::Tuple(items), Value::Number(n)) => {
                            let i = *n as usize;
                            if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                stack.push(items[i].clone());
                            } else {
                                return Err(self.error(span, "Index out of bounds"));
                            }
                        }
                        (Value::Record(fields), Value::String(k)) => {
                            if let Some(val) = fields.get(k) {
                                stack.push(val.clone());
                            } else {
                                return Err(self.error(span, format!("Field '{k}' not found")));
                            }
                        }
                        _ => return Err(self.error(span, "Cannot index value")),
                    }
                }
                Opcode::IndexSet(slot) => {
                    let val = stack.pop().unwrap();
                    let idx = stack.pop().unwrap();
                    let target_slot = *slot as usize;
                    match (&mut stack[target_slot], &idx) {
                        (Value::List(items), Value::Number(n)) => {
                            let i = *n as usize;
                            if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                items[i] = val;
                            } else {
                                return Err(self.error(span, "Index out of bounds"));
                            }
                        }
                        (Value::Record(fields), Value::String(k)) => {
                            fields.insert(k.clone(), val);
                        }
                        _ => return Err(self.error(span, "Cannot mutate target by index")),
                    }
                }
                Opcode::MemberGet(field_idx) => {
                    let field_name = match &chunk.constants[*field_idx as usize] {
                        Value::String(s) => s.as_str(),
                        _ => unreachable!(),
                    };
                    let obj = stack.pop().unwrap();
                    match &obj {
                        Value::Record(fields) => {
                            if let Some(val) = fields.get(field_name) {
                                stack.push(val.clone());
                            } else {
                                return Err(
                                    self.error(span, format!("Field '{field_name}' not found"))
                                );
                            }
                        }
                        Value::Vector(v) => {
                            let val = match field_name {
                                "x" if !v.is_empty() => v[0],
                                "y" if v.len() > 1 => v[1],
                                "z" if v.len() > 2 => v[2],
                                "w" if v.len() > 3 => v[3],
                                _ => {
                                    return Err(self.error(
                                        span,
                                        format!("Vector field '{field_name}' not found"),
                                    ));
                                }
                            };
                            stack.push(Value::Number(val));
                        }
                        _ => return Err(self.error(span, "Cannot access member on value")),
                    }
                }
                Opcode::MemberSet(slot, field_idx) => {
                    let field_name = match &chunk.constants[*field_idx as usize] {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let val = stack.pop().unwrap();
                    let target_slot = *slot as usize;
                    match &mut stack[target_slot] {
                        Value::Record(fields) => {
                            fields.insert(field_name, val);
                        }
                        _ => return Err(self.error(span, "Cannot mutate member on value")),
                    }
                }
                Opcode::Interpolate(count) => {
                    let start = stack.len() - *count;
                    let parts: Vec<_> = stack.drain(start..).collect();
                    let mut out = String::new();
                    for part in parts {
                        format_value_for_display(&part, &mut out, 0);
                    }
                    stack.push(Value::String(out));
                }
                Opcode::Call(arg_count) => {
                    let start = stack.len() - *arg_count;
                    let args: Vec<_> = stack.drain(start..).collect();
                    let callee = stack.pop().unwrap();
                    match callee {
                        Value::Builtin(b) => {
                            let result = self.call_builtin(b, &args, span)?;
                            stack.push(result);
                        }
                        _ => return Err(self.error(span, "Value is not callable")),
                    }
                }
                Opcode::Return => {
                    return Ok(stack.pop().unwrap_or(Value::Null));
                }
            }
        }

        Ok(stack.pop().unwrap_or(Value::Null))
    }

    fn call_builtin(
        &mut self,
        builtin: Builtin,
        args: &[Value<'static>],
        span: Span,
    ) -> Result<Value<'static>, RuntimeError> {
        match builtin {
            Builtin::Trim => {
                let Some(Value::String(s)) = args.first() else {
                    return Err(self.error(span, "trim requires a string"));
                };
                Ok(Value::String(s.trim().to_string()))
            }
            Builtin::TrimStart => {
                let Some(Value::String(s)) = args.first() else {
                    return Err(self.error(span, "trim_start requires a string"));
                };
                Ok(Value::String(s.trim_start().to_string()))
            }
            Builtin::TrimEnd => {
                let Some(Value::String(s)) = args.first() else {
                    return Err(self.error(span, "trim_end requires a string"));
                };
                Ok(Value::String(s.trim_end().to_string()))
            }
            Builtin::ToLower => {
                let Some(Value::String(s)) = args.first() else {
                    return Err(self.error(span, "to_lower requires a string"));
                };
                Ok(Value::String(s.to_lowercase()))
            }
            Builtin::ToUpper => {
                let Some(Value::String(s)) = args.first() else {
                    return Err(self.error(span, "to_upper requires a string"));
                };
                Ok(Value::String(s.to_uppercase()))
            }
            Builtin::StartsWith => {
                if args.len() != 2 {
                    return Err(self.error(span, "starts_with requires 2 arguments"));
                }
                let (Value::String(s), Value::String(prefix)) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "starts_with requires string arguments"));
                };
                Ok(Value::Bool(s.starts_with(prefix.as_str())))
            }
            Builtin::EndsWith => {
                if args.len() != 2 {
                    return Err(self.error(span, "ends_with requires 2 arguments"));
                }
                let (Value::String(s), Value::String(suffix)) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "ends_with requires string arguments"));
                };
                Ok(Value::Bool(s.ends_with(suffix.as_str())))
            }
            Builtin::Contains => {
                if args.len() != 2 {
                    return Err(self.error(span, "contains requires 2 arguments"));
                }
                match (&args[0], &args[1]) {
                    (Value::String(s), Value::String(needle)) => {
                        Ok(Value::Bool(s.contains(needle.as_str())))
                    }
                    (Value::List(items), needle) => Ok(Value::Bool(items.contains(needle))),
                    (Value::Record(fields), Value::String(k)) => {
                        Ok(Value::Bool(fields.contains_key(k)))
                    }
                    _ => Err(self.error(span, "Invalid operands for contains")),
                }
            }
            Builtin::Replace => {
                if args.len() != 3 {
                    return Err(self.error(span, "replace requires 3 arguments"));
                }
                let (Value::String(s), Value::String(from), Value::String(to)) =
                    (&args[0], &args[1], &args[2])
                else {
                    return Err(self.error(span, "replace requires string arguments"));
                };
                Ok(Value::String(s.replace(from.as_str(), to.as_str())))
            }
            Builtin::Split => {
                if args.len() != 2 {
                    return Err(self.error(span, "split requires 2 arguments"));
                }
                let (Value::String(s), Value::String(del)) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "split requires string arguments"));
                };
                let raw_parts: Vec<String> = if del.is_empty() {
                    s.chars().map(|c| c.to_string()).collect()
                } else {
                    s.split(del.as_str()).map(|p| p.to_string()).collect()
                };
                Ok(Value::List(
                    raw_parts.into_iter().map(Value::String).collect(),
                ))
            }
            Builtin::Join => {
                if args.len() != 2 {
                    return Err(self.error(span, "join requires 2 arguments"));
                }
                let (Value::List(items), Value::String(sep)) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "join requires a list and a separator"));
                };
                let mut out = String::new();
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(sep);
                    }
                    format_value_for_display(item, &mut out, 0);
                }
                Ok(Value::String(out))
            }
            Builtin::Len => {
                let Some(arg) = args.first() else {
                    return Err(self.error(span, "len requires 1 argument"));
                };
                match arg {
                    Value::String(s) => Ok(Value::Number(s.len() as f64)),
                    Value::List(l) => Ok(Value::Number(l.len() as f64)),
                    Value::Tuple(t) => Ok(Value::Number(t.len() as f64)),
                    Value::Record(r) => Ok(Value::Number(r.len() as f64)),
                    Value::Vector(v) => Ok(Value::Number(v.len() as f64)),
                    _ => Err(self.error(span, "len requires a collection or string")),
                }
            }
            Builtin::Get => {
                if args.len() != 2 {
                    return Err(self.error(span, "get requires 2 arguments"));
                }
                match (&args[0], &args[1]) {
                    (Value::List(items), Value::Number(n)) => {
                        let i = *n as usize;
                        if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                            Ok(items[i].clone())
                        } else {
                            Ok(Value::Null)
                        }
                    }
                    (Value::Record(fields), Value::String(k)) => {
                        Ok(fields.get(k).cloned().unwrap_or(Value::Null))
                    }
                    _ => Err(self.error(span, "get requires (list, number) or (record, string)")),
                }
            }
            Builtin::Assert => {
                let Some(arg) = args.first() else {
                    return Err(self.error(span, "assert requires at least 1 argument"));
                };
                if !is_truthy(arg) {
                    let msg = args
                        .get(1)
                        .and_then(|v| match v {
                            Value::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or("Assertion failed");
                    return Err(self.error(span, msg));
                }
                Ok(Value::Null)
            }
            Builtin::JsonParse => {
                let Some(Value::String(text)) = args.first() else {
                    return Err(self.error(span, "json_parse requires a string"));
                };
                match crate::json_parse(text) {
                    Ok(v) => Ok(Value::Variant("Ok", vec![v])),
                    Err(e) => Ok(Value::Variant("Err", vec![Value::String(e)])),
                }
            }
            Builtin::JsonStringify => {
                let Some(arg) = args.first() else {
                    return Err(self.error(span, "json_stringify requires 1 argument"));
                };
                match crate::json_stringify(arg) {
                    Ok(s) => Ok(Value::String(s)),
                    Err(e) => Err(self.error(span, format!("json_stringify failed: {e}"))),
                }
            }
            Builtin::Sin => {
                let Some(Value::Number(n)) = args.first() else {
                    return Err(self.error(span, "sin requires a number"));
                };
                Ok(Value::Number(n.sin()))
            }
            Builtin::Cos => {
                let Some(Value::Number(n)) = args.first() else {
                    return Err(self.error(span, "cos requires a number"));
                };
                Ok(Value::Number(n.cos()))
            }
            Builtin::Sqrt => {
                let Some(Value::Number(n)) = args.first() else {
                    return Err(self.error(span, "sqrt requires a number"));
                };
                Ok(Value::Number(n.sqrt()))
            }
            Builtin::Degrees => {
                let Some(Value::Number(n)) = args.first() else {
                    return Err(self.error(span, "degrees requires a number"));
                };
                Ok(Value::Number(n.to_degrees()))
            }
            Builtin::Radians | Builtin::Deg => {
                let Some(Value::Number(n)) = args.first() else {
                    return Err(self.error(span, "radians requires a number"));
                };
                Ok(Value::Number(n.to_radians()))
            }
            Builtin::Vec2 => {
                if args.len() != 2 {
                    return Err(self.error(span, "vec2 requires 2 arguments"));
                }
                let (Value::Number(x), Value::Number(y)) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "vec2 requires numbers"));
                };
                Ok(Value::Vector(vec![*x, *y]))
            }
            Builtin::Vec3 => {
                if args.len() != 3 {
                    return Err(self.error(span, "vec3 requires 3 arguments"));
                }
                let (Value::Number(x), Value::Number(y), Value::Number(z)) =
                    (&args[0], &args[1], &args[2])
                else {
                    return Err(self.error(span, "vec3 requires numbers"));
                };
                Ok(Value::Vector(vec![*x, *y, *z]))
            }
            Builtin::Vec4 => {
                if args.len() != 4 {
                    return Err(self.error(span, "vec4 requires 4 arguments"));
                }
                let (Value::Number(x), Value::Number(y), Value::Number(z), Value::Number(w)) =
                    (&args[0], &args[1], &args[2], &args[3])
                else {
                    return Err(self.error(span, "vec4 requires numbers"));
                };
                Ok(Value::Vector(vec![*x, *y, *z, *w]))
            }
            _ => Err(self.error(
                span,
                format!("Builtin '{builtin:?}' not supported in VM yet"),
            )),
        }
    }
}

fn is_truthy(value: &Value<'_>) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::List(l) => !l.is_empty(),
        _ => true,
    }
}
