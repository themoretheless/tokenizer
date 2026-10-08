//! Flat-frame Bytecode VM and AST-to-bytecode compiler for Rush.
//!
//! Provides high-performance, flat stack-frame execution of compiled Rush code,
//! bypassing recursive AST walking and Environment map cloning overhead.
//! Supports first-class user functions (`fn`), anonymous lambdas, closures with
//! captured lexical upvalues (mutable and immutable), and higher-order builtins.

use crate::{
    Builtin, Expr, ExprKind, InterpolationPart, RuntimeError, Stmt, StmtKind, Value,
    builtin_catalog,
    runtime::{UserData, exact_remainder, format_value_for_display},
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

/// Upvalue state for variables captured in closures.
#[derive(Clone, Debug, PartialEq)]
pub enum Upvalue {
    /// The variable is still alive on the VM operand/locals stack.
    Open(usize),
    /// The variable's stack frame has exited; the value now lives on the heap.
    Closed(Value<'static>),
}

/// Upvalue binding descriptor used during compilation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpvalueDescriptor {
    pub is_local: bool,
    pub index: u32,
}

/// A compiled bytecode function.
#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeFunction {
    pub name: Option<String>,
    pub arity: usize,
    pub chunk: Chunk,
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
}

/// A runtime closure wrapping a compiled function and its captured upvalue cells.
#[derive(Clone, Debug)]
pub struct BytecodeClosure {
    pub function: Rc<BytecodeFunction>,
    pub upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl PartialEq for BytecodeClosure {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.function, &other.function)
            && self.upvalues.len() == other.upvalues.len()
            && self
                .upvalues
                .iter()
                .zip(&other.upvalues)
                .all(|(a, b)| Rc::ptr_eq(a, b))
    }
}

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
    /// Read local variable at relative stack slot.
    GetLocal(u32),
    /// Write top-of-stack to local variable slot.
    SetLocal(u32),
    /// Read global variable by name constant index.
    GetGlobal(u32),
    /// Write top-of-stack to global variable by name constant index.
    SetGlobal(u32),
    /// Read captured upvalue at closure index.
    GetUpvalue(u32),
    /// Write top-of-stack to captured upvalue at closure index.
    SetUpvalue(u32),
    /// Mutate container in upvalue by index: pops index and new value.
    IndexSetUpvalue(u32),
    /// Mutate member of container in upvalue by field constant index: pops new value.
    MemberSetUpvalue(u32, u32),
    /// Instantiate a closure from function constant index with captured upvalues.
    MakeClosure(u32),
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
    /// Check if top of stack is a tuple with exact arity (pops tuple, pushes bool).
    CheckTuple(usize),
    /// Assert top of stack is a tuple with exact arity (pops tuple, errors if invalid).
    AssertTuple(usize),
    /// Extract element from tuple on top of stack (pops tuple, pushes element).
    TupleGet(u32),
    /// Check if top of stack is a record or struct (pops value, pushes bool).
    CheckRecord,
    /// Assert top of stack is a record or struct (pops value, errors if invalid).
    AssertRecord,
    /// Check if record on top of stack has given field (pops record, pushes bool).
    CheckRecordField(u32),
    /// Extract field from record on top of stack (pops record, pushes field, errors if missing).
    RecordGetAssert(u32),
    /// Check if top of stack is a variant with tag and arity (pops variant, pushes bool).
    CheckVariant(u32, usize),
    /// Extract payload item from variant on top of stack (pops variant, pushes item).
    VariantGet(u32),
    /// Raise runtime error "No matching pattern".
    MatchError,
    /// Construct UserData instance with type_name, optional variant and arguments.
    MakeUserData {
        type_name_idx: u32,
        variant_idx: Option<u32>,
        arity: usize,
    },
    /// Return from execution.
    Return,
}

/// A compiled bytecode compilation unit containing instructions, constants and functions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub code: Vec<Opcode>,
    pub constants: Vec<Value<'static>>,
    pub functions: Vec<Rc<BytecodeFunction>>,
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

    pub fn add_function(&mut self, function: Rc<BytecodeFunction>) -> u32 {
        let idx = self.functions.len() as u32;
        self.functions.push(function);
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
        let top_fn = compiler.pop_state();
        Ok(Self {
            chunk: top_fn.chunk,
        })
    }

    /// Execute the compiled bytecode program with a step budget limit.
    pub fn execute(&self, budget: usize) -> Result<Value<'static>, RuntimeError> {
        let mut vm = Vm::new(budget);
        vm.run(&self.chunk)
    }
}

/// Compile and evaluate source code directly via the Bytecode VM.
pub fn evaluate_bytecode(source: &str, budget: usize) -> Result<Value<'static>, RuntimeError> {
    let program = BytecodeProgram::compile(source)?;
    program.execute(budget)
}

struct LoopContext {
    start_ip: usize,
    break_jumps: Vec<usize>,
}

struct CompilerState {
    name: Option<String>,
    arity: usize,
    chunk: Chunk,
    scopes: Vec<HashMap<String, u32>>,
    local_count: u32,
    upvalues: Vec<UpvalueDescriptor>,
    loops: Vec<LoopContext>,
}

impl CompilerState {
    fn new(name: Option<String>, arity: usize) -> Self {
        Self {
            name,
            arity,
            chunk: Chunk::new(),
            scopes: vec![HashMap::new()],
            local_count: arity as u32,
            upvalues: Vec::new(),
            loops: Vec::new(),
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

    fn declare_local(&mut self, name: &str) -> u32 {
        let slot = self.local_count;
        self.local_count += 1;
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), slot);
        }
        slot
    }

    fn declare_temp_local(&mut self) -> u32 {
        let slot = self.local_count;
        self.local_count += 1;
        slot
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn add_upvalue(&mut self, desc: UpvalueDescriptor) -> u32 {
        for (i, existing) in self.upvalues.iter().enumerate() {
            if *existing == desc {
                return i as u32;
            }
        }
        let idx = self.upvalues.len() as u32;
        self.upvalues.push(desc);
        idx
    }
}

enum VariableResolution {
    Local(u32),
    Upvalue(u32),
    Builtin(Builtin),
    Global(String),
}

struct Compiler<'s> {
    states: Vec<CompilerState>,
    _phantom: std::marker::PhantomData<&'s ()>,
}

impl<'s> Compiler<'s> {
    fn new() -> Self {
        Self {
            states: vec![CompilerState::new(Some("<main>".into()), 0)],
            _phantom: std::marker::PhantomData,
        }
    }

    fn current(&mut self) -> &mut CompilerState {
        self.states.last_mut().expect("Compiler state stack empty")
    }

    fn current_ref(&self) -> &CompilerState {
        self.states.last().expect("Compiler state stack empty")
    }

    fn push_state(&mut self, name: Option<String>, arity: usize) {
        self.states.push(CompilerState::new(name, arity));
    }

    fn pop_state(&mut self) -> BytecodeFunction {
        let mut state = self.states.pop().expect("Compiler state stack underflow");
        state.chunk.local_count = state.local_count as usize;
        BytecodeFunction {
            name: state.name,
            arity: state.arity,
            chunk: state.chunk,
            upvalue_descriptors: state.upvalues,
        }
    }

    fn is_module_level(&self) -> bool {
        self.states.len() == 1
    }

    fn resolve_variable(&mut self, name: &str) -> VariableResolution {
        if let Some(slot) = self.current_ref().resolve_local(name) {
            return VariableResolution::Local(slot);
        }
        let cur_idx = self.states.len() - 1;
        if let Some(upval_idx) = self.resolve_upvalue(cur_idx, name) {
            return VariableResolution::Upvalue(upval_idx);
        }
        if let Some((_, builtin)) = builtin_catalog().iter().find(|(n, _)| *n == name) {
            return VariableResolution::Builtin(*builtin);
        }
        VariableResolution::Global(name.to_string())
    }

    fn resolve_upvalue(&mut self, state_idx: usize, name: &str) -> Option<u32> {
        if state_idx == 0 {
            return None;
        }
        let parent_idx = state_idx - 1;
        if let Some(slot) = self.states[parent_idx].resolve_local(name) {
            let desc = UpvalueDescriptor {
                is_local: true,
                index: slot,
            };
            return Some(self.states[state_idx].add_upvalue(desc));
        }
        if let Some(parent_upval) = self.resolve_upvalue(parent_idx, name) {
            let desc = UpvalueDescriptor {
                is_local: false,
                index: parent_upval,
            };
            return Some(self.states[state_idx].add_upvalue(desc));
        }
        None
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

    fn compile_module(&mut self, items: &[Stmt<'s>]) -> Result<(), RuntimeError> {
        self.compile_block(items, true)?;
        let end_span = items.last().map_or(Span::new(0, 0), |s| s.span);
        self.current().chunk.emit(Opcode::Return, end_span);
        Ok(())
    }

    fn compile_block(&mut self, items: &[Stmt<'s>], keep_result: bool) -> Result<(), RuntimeError> {
        if items.is_empty() {
            if keep_result {
                self.current().chunk.emit(Opcode::Null, Span::new(0, 0));
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
                let slot = self.current().declare_local(name.text);
                if self.is_module_level() {
                    let name_idx = self
                        .current()
                        .chunk
                        .add_constant(Value::String(name.text.to_string()));
                    self.current()
                        .chunk
                        .emit(Opcode::SetGlobal(name_idx), stmt.span);
                }
                self.current().chunk.emit(Opcode::SetLocal(slot), stmt.span);
                self.current().chunk.emit(Opcode::Pop, stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Function {
                name,
                parameters,
                body,
                ..
            } => {
                let slot = self.current().declare_local(name.text);
                self.push_state(Some(name.text.to_string()), parameters.len());
                for (i, p) in parameters.iter().enumerate() {
                    if let Some(p_name) = extract_pattern_name(&p.pattern) {
                        self.current().scopes[0].insert(p_name.to_string(), i as u32);
                    }
                }
                self.compile_block(&body.stmts, false)?;
                self.current().chunk.emit(Opcode::Null, stmt.span);
                self.current().chunk.emit(Opcode::Return, stmt.span);
                let compiled = self.pop_state();
                let fn_idx = self.current().chunk.add_function(Rc::new(compiled));
                self.current()
                    .chunk
                    .emit(Opcode::MakeClosure(fn_idx), stmt.span);
                if self.is_module_level() {
                    let name_idx = self
                        .current()
                        .chunk
                        .add_constant(Value::String(name.text.to_string()));
                    self.current()
                        .chunk
                        .emit(Opcode::SetGlobal(name_idx), stmt.span);
                }
                self.current().chunk.emit(Opcode::SetLocal(slot), stmt.span);
                self.current().chunk.emit(Opcode::Pop, stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.compile_expr(condition)?;
                let false_jump = self.current().chunk.emit(Opcode::JumpIfFalse(0), stmt.span);
                self.compile_block(&then_block.stmts, keep_result)?;
                let end_jump = self.current().chunk.emit(Opcode::Jump(0), stmt.span);
                let then_len = self.current().chunk.code.len();
                self.current().chunk.patch_jump(false_jump, then_len);
                if let Some(else_b) = else_block {
                    self.compile_block(&else_b.stmts, keep_result)?;
                } else if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
                let end_len = self.current().chunk.code.len();
                self.current().chunk.patch_jump(end_jump, end_len);
            }
            StmtKind::While { condition, body } => {
                let start_ip = self.current().chunk.code.len();
                self.compile_expr(condition)?;
                let exit_jump = self.current().chunk.emit(Opcode::JumpIfFalse(0), stmt.span);
                self.current().loops.push(LoopContext {
                    start_ip,
                    break_jumps: Vec::new(),
                });
                self.compile_block(&body.stmts, false)?;
                self.current().chunk.emit(Opcode::Jump(start_ip), stmt.span);
                let loop_ctx = self.current().loops.pop().unwrap();
                let exit_target = self.current().chunk.code.len();
                self.current().chunk.patch_jump(exit_jump, exit_target);
                for brk in loop_ctx.break_jumps {
                    self.current().chunk.patch_jump(brk, exit_target);
                }
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Break => {
                if self.current().loops.is_empty() {
                    return Err(self.error(stmt.span, "break outside of loop"));
                }
                let jmp = self.current().chunk.emit(Opcode::Jump(0), stmt.span);
                self.current()
                    .loops
                    .last_mut()
                    .unwrap()
                    .break_jumps
                    .push(jmp);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Continue => {
                let Some(target) = self.current().loops.last().map(|l| l.start_ip) else {
                    return Err(self.error(stmt.span, "continue outside of loop"));
                };
                self.current().chunk.emit(Opcode::Jump(target), stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Return(value) => {
                if let Some(val) = value {
                    self.compile_expr(val)?;
                } else {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
                self.current().chunk.emit(Opcode::Return, stmt.span);
            }
            StmtKind::Destructure { pattern, value } => {
                self.compile_expr(value)?;
                let val_slot = self.current().declare_temp_local();
                self.current()
                    .chunk
                    .emit(Opcode::SetLocal(val_slot), stmt.span);
                self.current().chunk.emit(Opcode::Pop, stmt.span);
                self.compile_destructure_binding(pattern, val_slot, stmt.span)?;
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Struct { name, .. } => {
                let slot = self.current().declare_local(name.text);
                let type_name_idx = self
                    .current()
                    .chunk
                    .add_constant(Value::String(name.text.to_string()));
                let mut ctor_chunk = Chunk::new();
                let ctor_type_idx = ctor_chunk.add_constant(Value::String(name.text.to_string()));
                ctor_chunk.emit(Opcode::GetLocal(0), stmt.span);
                ctor_chunk.emit(
                    Opcode::MakeUserData {
                        type_name_idx: ctor_type_idx,
                        variant_idx: None,
                        arity: 1,
                    },
                    stmt.span,
                );
                ctor_chunk.emit(Opcode::Return, stmt.span);
                ctor_chunk.local_count = 1;
                let ctor_fn = Rc::new(BytecodeFunction {
                    name: Some(format!("{}::constructor", name.text)),
                    arity: 1,
                    chunk: ctor_chunk,
                    upvalue_descriptors: Vec::new(),
                });
                let fn_idx = self.current().chunk.add_function(ctor_fn);
                self.current()
                    .chunk
                    .emit(Opcode::MakeClosure(fn_idx), stmt.span);
                if self.is_module_level() {
                    self.current()
                        .chunk
                        .emit(Opcode::SetGlobal(type_name_idx), stmt.span);
                }
                self.current().chunk.emit(Opcode::SetLocal(slot), stmt.span);
                self.current().chunk.emit(Opcode::Pop, stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Enum { name, variants } => {
                let slot = self.current().declare_local(name.text);
                let type_name_idx = self
                    .current()
                    .chunk
                    .add_constant(Value::String(name.text.to_string()));

                for (var_name, var_types) in variants {
                    let var_const_idx = self
                        .current()
                        .chunk
                        .add_constant(Value::String(var_name.text.to_string()));
                    self.current()
                        .chunk
                        .emit(Opcode::Constant(var_const_idx), var_name.span);

                    let arity = var_types.len();
                    let mut ctor_chunk = Chunk::new();
                    let ctor_type_idx =
                        ctor_chunk.add_constant(Value::String(name.text.to_string()));
                    let ctor_var_idx =
                        ctor_chunk.add_constant(Value::String(var_name.text.to_string()));
                    for i in 0..arity {
                        ctor_chunk.emit(Opcode::GetLocal(i as u32), var_name.span);
                    }
                    ctor_chunk.emit(
                        Opcode::MakeUserData {
                            type_name_idx: ctor_type_idx,
                            variant_idx: Some(ctor_var_idx),
                            arity,
                        },
                        var_name.span,
                    );
                    ctor_chunk.emit(Opcode::Return, var_name.span);
                    ctor_chunk.local_count = arity;
                    let ctor_fn = Rc::new(BytecodeFunction {
                        name: Some(format!("{}::{}", name.text, var_name.text)),
                        arity,
                        chunk: ctor_chunk,
                        upvalue_descriptors: Vec::new(),
                    });
                    let fn_idx = self.current().chunk.add_function(ctor_fn);
                    self.current()
                        .chunk
                        .emit(Opcode::MakeClosure(fn_idx), var_name.span);
                }

                self.current()
                    .chunk
                    .emit(Opcode::BuildRecord(variants.len()), stmt.span);

                if self.is_module_level() {
                    self.current()
                        .chunk
                        .emit(Opcode::SetGlobal(type_name_idx), stmt.span);
                }
                self.current().chunk.emit(Opcode::SetLocal(slot), stmt.span);
                self.current().chunk.emit(Opcode::Pop, stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Expr(expr) => {
                self.compile_expr(expr)?;
                if !keep_result {
                    self.current().chunk.emit(Opcode::Pop, stmt.span);
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
                let idx = self.current().chunk.add_constant(Value::Number(n));
                self.current().chunk.emit(Opcode::Constant(idx), span);
            }
            ExprKind::String(text) => {
                let mut decoded = String::new();
                for c in crate::string_literal::characters(text) {
                    let ch = c.map_err(|e| self.error(span, e))?;
                    decoded.push(ch);
                }
                let idx = self.current().chunk.add_constant(Value::String(decoded));
                self.current().chunk.emit(Opcode::Constant(idx), span);
            }
            ExprKind::Bool(b) => {
                if *b {
                    self.current().chunk.emit(Opcode::True, span);
                } else {
                    self.current().chunk.emit(Opcode::False, span);
                }
            }
            ExprKind::Null => {
                self.current().chunk.emit(Opcode::Null, span);
            }
            ExprKind::Name(name) => match self.resolve_variable(name.text) {
                VariableResolution::Local(slot) => {
                    self.current().chunk.emit(Opcode::GetLocal(slot), span);
                }
                VariableResolution::Upvalue(idx) => {
                    self.current().chunk.emit(Opcode::GetUpvalue(idx), span);
                }
                VariableResolution::Builtin(builtin) => {
                    let idx = self.current().chunk.add_constant(Value::Builtin(builtin));
                    self.current().chunk.emit(Opcode::Constant(idx), span);
                }
                VariableResolution::Global(var_name) => {
                    let idx = self.current().chunk.add_constant(Value::String(var_name));
                    self.current().chunk.emit(Opcode::GetGlobal(idx), span);
                }
            },
            ExprKind::Unary { operator, value } => {
                self.compile_expr(value)?;
                match *operator {
                    "-" => {
                        self.current().chunk.emit(Opcode::Neg, span);
                    }
                    "!" => {
                        self.current().chunk.emit(Opcode::Not, span);
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
                    self.current().chunk.emit(Opcode::Dup, span);
                    let false_jump = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_expr(right)?;
                    let len = self.current().chunk.code.len();
                    self.current().chunk.patch_jump(false_jump, len);
                    return Ok(());
                }
                if *operator == "||" {
                    self.compile_expr(left)?;
                    self.current().chunk.emit(Opcode::Dup, span);
                    let true_jump = self.current().chunk.emit(Opcode::JumpIfTrue(0), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_expr(right)?;
                    let len = self.current().chunk.code.len();
                    self.current().chunk.patch_jump(true_jump, len);
                    return Ok(());
                }

                self.compile_expr(left)?;
                self.compile_expr(right)?;
                match *operator {
                    "+" => self.current().chunk.emit(Opcode::Add, span),
                    "-" => self.current().chunk.emit(Opcode::Sub, span),
                    "*" => self.current().chunk.emit(Opcode::Mul, span),
                    "/" => self.current().chunk.emit(Opcode::Div, span),
                    "%" => self.current().chunk.emit(Opcode::Mod, span),
                    "==" => self.current().chunk.emit(Opcode::Equal, span),
                    "!=" => self.current().chunk.emit(Opcode::NotEqual, span),
                    "<" => self.current().chunk.emit(Opcode::Less, span),
                    "<=" => self.current().chunk.emit(Opcode::LessEqual, span),
                    ">" => self.current().chunk.emit(Opcode::Greater, span),
                    ">=" => self.current().chunk.emit(Opcode::GreaterEqual, span),
                    _ => return Err(self.error(span, "Unsupported binary operator")),
                };
            }
            ExprKind::Assign {
                target,
                value,
                operator,
            } => match &target.kind {
                ExprKind::Name(name) => {
                    let res = self.resolve_variable(name.text);
                    if *operator == "=" {
                        self.compile_expr(value)?;
                    } else {
                        let base_op = operator.strip_suffix('=').unwrap_or(operator);
                        match &res {
                            VariableResolution::Local(slot) => {
                                self.current()
                                    .chunk
                                    .emit(Opcode::GetLocal(*slot), target.span);
                            }
                            VariableResolution::Upvalue(idx) => {
                                self.current()
                                    .chunk
                                    .emit(Opcode::GetUpvalue(*idx), target.span);
                            }
                            VariableResolution::Global(n) => {
                                let idx =
                                    self.current().chunk.add_constant(Value::String(n.clone()));
                                self.current()
                                    .chunk
                                    .emit(Opcode::GetGlobal(idx), target.span);
                            }
                            VariableResolution::Builtin(_) => {
                                return Err(self.error(target.span, "Cannot reassign builtin"));
                            }
                        }
                        self.compile_expr(value)?;
                        match base_op {
                            "+" => self.current().chunk.emit(Opcode::Add, span),
                            "-" => self.current().chunk.emit(Opcode::Sub, span),
                            "*" => self.current().chunk.emit(Opcode::Mul, span),
                            "/" => self.current().chunk.emit(Opcode::Div, span),
                            "%" => self.current().chunk.emit(Opcode::Mod, span),
                            _ => return Err(self.error(span, "Unsupported assignment operator")),
                        };
                    }
                    match res {
                        VariableResolution::Local(slot) => {
                            self.current()
                                .chunk
                                .emit(Opcode::SetLocal(slot), target.span);
                        }
                        VariableResolution::Upvalue(idx) => {
                            self.current()
                                .chunk
                                .emit(Opcode::SetUpvalue(idx), target.span);
                        }
                        VariableResolution::Global(n) => {
                            let idx = self.current().chunk.add_constant(Value::String(n));
                            self.current()
                                .chunk
                                .emit(Opcode::SetGlobal(idx), target.span);
                        }
                        VariableResolution::Builtin(_) => {
                            return Err(self.error(target.span, "Cannot reassign builtin"));
                        }
                    }
                }
                ExprKind::Index { object, index } => {
                    let ExprKind::Name(obj_name) = &object.kind else {
                        return Err(
                            self.error(object.span, "Index assignment requires a variable target")
                        );
                    };
                    let res = self.resolve_variable(obj_name.text);
                    self.compile_expr(index)?;
                    self.compile_expr(value)?;
                    match res {
                        VariableResolution::Local(slot) => {
                            self.current()
                                .chunk
                                .emit(Opcode::IndexSet(slot), target.span);
                            self.current()
                                .chunk
                                .emit(Opcode::GetLocal(slot), target.span);
                        }
                        VariableResolution::Upvalue(idx) => {
                            self.current()
                                .chunk
                                .emit(Opcode::IndexSetUpvalue(idx), target.span);
                            self.current()
                                .chunk
                                .emit(Opcode::GetUpvalue(idx), target.span);
                        }
                        _ => return Err(self.error(object.span, "Cannot mutate target by index")),
                    }
                }
                ExprKind::Member { object, field } => {
                    let ExprKind::Name(obj_name) = &object.kind else {
                        return Err(
                            self.error(object.span, "Member assignment requires a variable target")
                        );
                    };
                    let res = self.resolve_variable(obj_name.text);
                    self.compile_expr(value)?;
                    let field_idx = self
                        .current()
                        .chunk
                        .add_constant(Value::String(field.text.to_string()));
                    match res {
                        VariableResolution::Local(slot) => {
                            self.current()
                                .chunk
                                .emit(Opcode::MemberSet(slot, field_idx), target.span);
                            self.current()
                                .chunk
                                .emit(Opcode::GetLocal(slot), target.span);
                        }
                        VariableResolution::Upvalue(idx) => {
                            self.current()
                                .chunk
                                .emit(Opcode::MemberSetUpvalue(idx, field_idx), target.span);
                            self.current()
                                .chunk
                                .emit(Opcode::GetUpvalue(idx), target.span);
                        }
                        _ => return Err(self.error(object.span, "Cannot mutate member on value")),
                    }
                }
                _ => return Err(self.error(target.span, "Unsupported assignment expression")),
            },
            ExprKind::Index { object, index } => {
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.current().chunk.emit(Opcode::IndexGet, span);
            }
            ExprKind::Member { object, field } => {
                self.compile_expr(object)?;
                let field_idx = self
                    .current()
                    .chunk
                    .add_constant(Value::String(field.text.to_string()));
                self.current()
                    .chunk
                    .emit(Opcode::MemberGet(field_idx), span);
            }
            ExprKind::List(items) => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.current()
                    .chunk
                    .emit(Opcode::BuildList(items.len()), span);
            }
            ExprKind::Tuple(items) => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.current()
                    .chunk
                    .emit(Opcode::BuildTuple(items.len()), span);
            }
            ExprKind::Map(entries) => {
                for (key, val) in entries {
                    match &key.kind {
                        ExprKind::Name(n) => {
                            let idx = self
                                .current()
                                .chunk
                                .add_constant(Value::String(n.text.to_string()));
                            self.current().chunk.emit(Opcode::Constant(idx), key.span);
                        }
                        _ => self.compile_expr(key)?,
                    }
                    self.compile_expr(val)?;
                }
                self.current()
                    .chunk
                    .emit(Opcode::BuildRecord(entries.len()), span);
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                self.compile_expr(condition)?;
                let false_jump = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                self.compile_expr(then_value)?;
                let end_jump = self.current().chunk.emit(Opcode::Jump(0), span);
                let then_len = self.current().chunk.code.len();
                self.current().chunk.patch_jump(false_jump, then_len);
                self.compile_expr(else_value)?;
                let end_len = self.current().chunk.code.len();
                self.current().chunk.patch_jump(end_jump, end_len);
            }
            ExprKind::Lambda { parameters, body } => {
                self.push_state(None, parameters.len());
                for (i, p) in parameters.iter().enumerate() {
                    if let Some(p_name) = extract_pattern_name(p) {
                        self.current().scopes[0].insert(p_name.to_string(), i as u32);
                    }
                }
                self.compile_expr(body)?;
                self.current().chunk.emit(Opcode::Return, span);
                let compiled = self.pop_state();
                let fn_idx = self.current().chunk.add_function(Rc::new(compiled));
                self.current().chunk.emit(Opcode::MakeClosure(fn_idx), span);
            }
            ExprKind::Call { callee, arguments } => {
                self.compile_expr(callee)?;
                for arg in arguments {
                    self.compile_expr(arg)?;
                }
                self.current()
                    .chunk
                    .emit(Opcode::Call(arguments.len()), span);
            }
            ExprKind::Pipeline { input, stages } => {
                self.compile_expr(input)?;
                for stage in stages {
                    match &stage.kind {
                        ExprKind::Call { callee, arguments } => {
                            let tmp_slot = self.current().declare_local("__pipe_tmp");
                            self.current()
                                .chunk
                                .emit(Opcode::SetLocal(tmp_slot), stage.span);
                            self.current().chunk.emit(Opcode::Pop, stage.span);
                            self.compile_expr(callee)?;
                            self.current()
                                .chunk
                                .emit(Opcode::GetLocal(tmp_slot), stage.span);
                            for arg in arguments {
                                self.compile_expr(arg)?;
                            }
                            self.current()
                                .chunk
                                .emit(Opcode::Call(arguments.len() + 1), stage.span);
                        }
                        _ => {
                            let tmp_slot = self.current().declare_local("__pipe_tmp");
                            self.current()
                                .chunk
                                .emit(Opcode::SetLocal(tmp_slot), stage.span);
                            self.current().chunk.emit(Opcode::Pop, stage.span);
                            self.compile_expr(stage)?;
                            self.current()
                                .chunk
                                .emit(Opcode::GetLocal(tmp_slot), stage.span);
                            self.current().chunk.emit(Opcode::Call(1), stage.span);
                        }
                    }
                }
            }
            ExprKind::Interpolate(parts) => {
                for part in parts {
                    match part {
                        InterpolationPart::Literal(s) => {
                            let idx = self.current().chunk.add_constant(Value::String(s.clone()));
                            self.current().chunk.emit(Opcode::Constant(idx), span);
                        }
                        InterpolationPart::Expr(sub) => {
                            self.compile_expr(sub)?;
                        }
                    }
                }
                self.current()
                    .chunk
                    .emit(Opcode::Interpolate(parts.len()), span);
            }
            ExprKind::Match { value, arms } => {
                self.compile_expr(value)?;
                let val_slot = self.current().declare_temp_local();
                self.current().chunk.emit(Opcode::SetLocal(val_slot), span);
                self.current().chunk.emit(Opcode::Pop, span);

                let mut match_end_jumps = Vec::new();

                for arm in arms {
                    self.current().push_scope();
                    let mut fail_jumps = Vec::new();
                    self.compile_pattern_match(&arm.pattern, val_slot, &mut fail_jumps)?;

                    if let Some(guard) = &arm.guard {
                        self.compile_expr(guard)?;
                        let guard_jmp = self
                            .current()
                            .chunk
                            .emit(Opcode::JumpIfFalse(0), guard.span);
                        fail_jumps.push(guard_jmp);
                    }

                    self.compile_expr(&arm.value)?;
                    let end_jmp = self.current().chunk.emit(Opcode::Jump(0), arm.span);
                    match_end_jumps.push(end_jmp);

                    let next_arm_target = self.current().chunk.code.len();
                    for fj in fail_jumps {
                        self.current().chunk.patch_jump(fj, next_arm_target);
                    }
                    self.current().pop_scope();
                }

                self.current().chunk.emit(Opcode::MatchError, span);
                let end_target = self.current().chunk.code.len();
                for ej in match_end_jumps {
                    self.current().chunk.patch_jump(ej, end_target);
                }
            }
            _ => {
                return Err(self.error(span, "Expression not supported in bytecode compilation"));
            }
        }
        Ok(())
    }

    fn compile_pattern_match(
        &mut self,
        pattern: &Expr<'s>,
        target_slot: u32,
        fail_jumps: &mut Vec<usize>,
    ) -> Result<(), RuntimeError> {
        let span = pattern.span;
        match &pattern.kind {
            ExprKind::Name(name) => {
                if name.text != "_" {
                    let slot = self.current().declare_local(name.text);
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current().chunk.emit(Opcode::SetLocal(slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                }
            }
            ExprKind::Tuple(patterns) => {
                self.current()
                    .chunk
                    .emit(Opcode::GetLocal(target_slot), span);
                self.current()
                    .chunk
                    .emit(Opcode::CheckTuple(patterns.len()), span);
                let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                fail_jumps.push(jmp);
                for (i, subpattern) in patterns.iter().enumerate() {
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current().chunk.emit(Opcode::TupleGet(i as u32), span);
                    let elem_slot = self.current().declare_temp_local();
                    self.current().chunk.emit(Opcode::SetLocal(elem_slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_pattern_match(subpattern, elem_slot, fail_jumps)?;
                }
            }
            ExprKind::Map(entries) => {
                self.current()
                    .chunk
                    .emit(Opcode::GetLocal(target_slot), span);
                self.current().chunk.emit(Opcode::CheckRecord, span);
                let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                fail_jumps.push(jmp);
                for (key, subpattern) in entries {
                    let ExprKind::Name(field_name) = &key.kind else {
                        return Err(self.error(key.span, "Record pattern keys must be names"));
                    };
                    let field_idx = self
                        .current()
                        .chunk
                        .add_constant(Value::String(field_name.text.to_string()));
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current()
                        .chunk
                        .emit(Opcode::CheckRecordField(field_idx), span);
                    let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                    fail_jumps.push(jmp);
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current()
                        .chunk
                        .emit(Opcode::RecordGetAssert(field_idx), span);
                    let field_slot = self.current().declare_temp_local();
                    self.current()
                        .chunk
                        .emit(Opcode::SetLocal(field_slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_pattern_match(subpattern, field_slot, fail_jumps)?;
                }
            }
            ExprKind::Call { callee, arguments } => {
                let Some(tag) = extract_callee_tag(callee) else {
                    return Err(self.error(callee.span, "Invalid variant pattern"));
                };
                let tag_idx = self.current().chunk.add_constant(Value::String(tag));
                self.current()
                    .chunk
                    .emit(Opcode::GetLocal(target_slot), span);
                self.current()
                    .chunk
                    .emit(Opcode::CheckVariant(tag_idx, arguments.len()), span);
                let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                fail_jumps.push(jmp);
                for (i, arg_pattern) in arguments.iter().enumerate() {
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current()
                        .chunk
                        .emit(Opcode::VariantGet(i as u32), span);
                    let payload_slot = self.current().declare_temp_local();
                    self.current()
                        .chunk
                        .emit(Opcode::SetLocal(payload_slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_pattern_match(arg_pattern, payload_slot, fail_jumps)?;
                }
            }
            _ => {
                self.current()
                    .chunk
                    .emit(Opcode::GetLocal(target_slot), span);
                self.compile_expr(pattern)?;
                self.current().chunk.emit(Opcode::Equal, span);
                let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                fail_jumps.push(jmp);
            }
        }
        Ok(())
    }

    fn compile_destructure_binding(
        &mut self,
        pattern: &Expr<'s>,
        target_slot: u32,
        span: Span,
    ) -> Result<(), RuntimeError> {
        match &pattern.kind {
            ExprKind::Name(name) => {
                if name.text != "_" {
                    let slot = self.current().declare_local(name.text);
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    if self.is_module_level() {
                        let name_idx = self
                            .current()
                            .chunk
                            .add_constant(Value::String(name.text.to_string()));
                        self.current().chunk.emit(Opcode::SetGlobal(name_idx), span);
                    }
                    self.current().chunk.emit(Opcode::SetLocal(slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                }
            }
            ExprKind::Tuple(items) => {
                self.current()
                    .chunk
                    .emit(Opcode::GetLocal(target_slot), span);
                self.current()
                    .chunk
                    .emit(Opcode::AssertTuple(items.len()), span);
                for (i, item) in items.iter().enumerate() {
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current().chunk.emit(Opcode::TupleGet(i as u32), span);
                    let elem_slot = self.current().declare_temp_local();
                    self.current().chunk.emit(Opcode::SetLocal(elem_slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_destructure_binding(item, elem_slot, span)?;
                }
            }
            ExprKind::Map(entries) => {
                self.current()
                    .chunk
                    .emit(Opcode::GetLocal(target_slot), span);
                self.current().chunk.emit(Opcode::AssertRecord, span);
                for (key, subpattern) in entries {
                    let ExprKind::Name(field_name) = &key.kind else {
                        return Err(self.error(key.span, "Record pattern keys must be names"));
                    };
                    let field_idx = self
                        .current()
                        .chunk
                        .add_constant(Value::String(field_name.text.to_string()));
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(target_slot), span);
                    self.current()
                        .chunk
                        .emit(Opcode::RecordGetAssert(field_idx), span);
                    let field_slot = self.current().declare_temp_local();
                    self.current()
                        .chunk
                        .emit(Opcode::SetLocal(field_slot), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_destructure_binding(subpattern, field_slot, span)?;
                }
            }
            _ => {
                return Err(self.error(pattern.span, "Unsupported destructuring pattern"));
            }
        }
        Ok(())
    }
}

fn extract_callee_tag(expr: &Expr<'_>) -> Option<String> {
    match &expr.kind {
        ExprKind::Name(n) => Some(n.text.to_string()),
        ExprKind::Member { field, .. } => Some(field.text.to_string()),
        _ => None,
    }
}

fn extract_pattern_name<'s>(expr: &Expr<'s>) -> Option<&'s str> {
    match &expr.kind {
        ExprKind::Name(n) => Some(n.text),
        _ => None,
    }
}

struct CallFrame {
    closure: Rc<BytecodeClosure>,
    ip: usize,
    stack_base: usize,
}

/// The flat stack Bytecode Virtual Machine runtime.
pub struct Vm {
    fuel: usize,
    globals: HashMap<String, Value<'static>>,
    frames: Vec<CallFrame>,
    stack: Vec<Value<'static>>,
    open_upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl Vm {
    pub fn new(budget: usize) -> Self {
        Self {
            fuel: budget,
            globals: HashMap::new(),
            frames: Vec::new(),
            stack: Vec::new(),
            open_upvalues: Vec::new(),
        }
    }

    pub fn fuel(&self) -> usize {
        self.fuel
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
        let main_fn = Rc::new(BytecodeFunction {
            name: Some("<main>".into()),
            arity: 0,
            chunk: chunk.clone(),
            upvalue_descriptors: Vec::new(),
        });
        let main_closure = Rc::new(BytecodeClosure {
            function: main_fn,
            upvalues: Vec::new(),
        });
        self.run_closure(&main_closure, &[])
    }

    pub fn run_closure(
        &mut self,
        closure: &Rc<BytecodeClosure>,
        args: &[Value<'static>],
    ) -> Result<Value<'static>, RuntimeError> {
        if args.len() != closure.function.arity {
            return Err(self.error(
                Span::new(0, 0),
                format!(
                    "Argument count mismatch: expected {}, got {}",
                    closure.function.arity,
                    args.len()
                ),
            ));
        }
        let prev_frames = std::mem::take(&mut self.frames);
        let prev_stack = std::mem::take(&mut self.stack);

        let stack_base = 0;
        let local_count = closure.function.chunk.local_count;
        let mut stack = Vec::with_capacity(local_count.max(args.len()));
        stack.extend_from_slice(args);
        if stack.len() < local_count {
            stack.resize(local_count, Value::Null);
        }
        self.stack = stack;
        self.frames = vec![CallFrame {
            closure: closure.clone(),
            ip: 0,
            stack_base,
        }];

        let result = self.execute_loop();

        self.frames = prev_frames;
        self.stack = prev_stack;

        result
    }

    fn close_upvalues(&mut self, from_slot: usize) {
        let stack = &self.stack;
        self.open_upvalues.retain(|upval| {
            let mut cell = upval.borrow_mut();
            if let Upvalue::Open(slot) = *cell
                && slot >= from_slot
            {
                let val = stack.get(slot).cloned().unwrap_or(Value::Null);
                *cell = Upvalue::Closed(val);
                return false;
            }
            true
        });
    }

    fn execute_loop(&mut self) -> Result<Value<'static>, RuntimeError> {
        while !self.frames.is_empty() {
            let frame_idx = self.frames.len() - 1;
            let ip = self.frames[frame_idx].ip;

            if self.fuel == 0 {
                let span = self.frames[frame_idx]
                    .closure
                    .function
                    .chunk
                    .spans
                    .get(ip)
                    .copied()
                    .unwrap_or(Span::new(0, 0));
                return Err(self.error(span, "Execution limit exceeded"));
            }
            self.fuel -= 1;

            if ip >= self.frames[frame_idx].closure.function.chunk.code.len() {
                let ret_val = self.stack.pop().unwrap_or(Value::Null);
                let exiting = self.frames.pop().unwrap();
                self.close_upvalues(exiting.stack_base);
                if self.frames.is_empty() {
                    return Ok(ret_val);
                }
                if exiting.stack_base > 0 {
                    self.stack.truncate(exiting.stack_base - 1);
                } else {
                    self.stack.clear();
                }
                self.stack.push(ret_val);
                continue;
            }

            let op = self.frames[frame_idx].closure.function.chunk.code[ip].clone();
            let span = self.frames[frame_idx].closure.function.chunk.spans[ip];
            self.frames[frame_idx].ip += 1;

            match op {
                Opcode::Constant(idx) => {
                    let val = self.frames[frame_idx].closure.function.chunk.constants[idx as usize]
                        .clone();
                    self.stack.push(val);
                }
                Opcode::Null => self.stack.push(Value::Null),
                Opcode::True => self.stack.push(Value::Bool(true)),
                Opcode::False => self.stack.push(Value::Bool(false)),
                Opcode::Pop => {
                    self.stack.pop();
                }
                Opcode::Dup => {
                    let top = self.stack.last().cloned().unwrap_or(Value::Null);
                    self.stack.push(top);
                }
                Opcode::GetLocal(slot) => {
                    let stack_base = self.frames[frame_idx].stack_base;
                    let idx = stack_base + slot as usize;
                    let val = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    self.stack.push(val);
                }
                Opcode::SetLocal(slot) => {
                    let stack_base = self.frames[frame_idx].stack_base;
                    let val = self.stack.last().cloned().unwrap_or(Value::Null);
                    let idx = stack_base + slot as usize;
                    if idx >= self.stack.len() {
                        self.stack.resize(idx + 1, Value::Null);
                    }
                    self.stack[idx] = val;
                }
                Opcode::GetUpvalue(idx) => {
                    let cell = self.frames[frame_idx].closure.upvalues[idx as usize].clone();
                    let val = match &*cell.borrow() {
                        Upvalue::Open(slot) => {
                            self.stack.get(*slot).cloned().unwrap_or(Value::Null)
                        }
                        Upvalue::Closed(v) => v.clone(),
                    };
                    self.stack.push(val);
                }
                Opcode::SetUpvalue(idx) => {
                    let val = self.stack.last().cloned().unwrap_or(Value::Null);
                    let cell = self.frames[frame_idx].closure.upvalues[idx as usize].clone();
                    let mut borrow = cell.borrow_mut();
                    match &mut *borrow {
                        Upvalue::Open(slot) => {
                            let slot = *slot;
                            if slot >= self.stack.len() {
                                self.stack.resize(slot + 1, Value::Null);
                            }
                            self.stack[slot] = val;
                        }
                        Upvalue::Closed(v) => *v = val,
                    }
                }
                Opcode::IndexSetUpvalue(upval_idx) => {
                    let val = self.stack.pop().unwrap();
                    let idx = self.stack.pop().unwrap();
                    let cell = self.frames[frame_idx].closure.upvalues[upval_idx as usize].clone();
                    let mut borrow = cell.borrow_mut();
                    let err = match &mut *borrow {
                        Upvalue::Open(s) => {
                            let slot = *s;
                            match (&mut self.stack[slot], &idx) {
                                (Value::List(items), Value::Number(n)) => {
                                    let i = *n as usize;
                                    if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                        items[i] = val;
                                        None
                                    } else {
                                        Some("Index out of bounds")
                                    }
                                }
                                (Value::Record(fields), Value::String(k)) => {
                                    fields.insert(k.clone(), val);
                                    None
                                }
                                _ => Some("Cannot mutate target by index"),
                            }
                        }
                        Upvalue::Closed(c) => match (c, &idx) {
                            (Value::List(items), Value::Number(n)) => {
                                let i = *n as usize;
                                if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                    items[i] = val;
                                    None
                                } else {
                                    Some("Index out of bounds")
                                }
                            }
                            (Value::Record(fields), Value::String(k)) => {
                                fields.insert(k.clone(), val);
                                None
                            }
                            _ => Some("Cannot mutate target by index"),
                        },
                    };
                    if let Some(msg) = err {
                        return Err(self.error(span, msg));
                    }
                }
                Opcode::MemberSetUpvalue(upval_idx, field_idx) => {
                    let field_name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [field_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let val = self.stack.pop().unwrap();
                    let cell = self.frames[frame_idx].closure.upvalues[upval_idx as usize].clone();
                    let mut borrow = cell.borrow_mut();
                    let err = match &mut *borrow {
                        Upvalue::Open(s) => {
                            let slot = *s;
                            match &mut self.stack[slot] {
                                Value::Record(fields) => {
                                    fields.insert(field_name, val);
                                    None
                                }
                                _ => Some("Cannot mutate member on value"),
                            }
                        }
                        Upvalue::Closed(c) => match c {
                            Value::Record(fields) => {
                                fields.insert(field_name, val);
                                None
                            }
                            _ => Some("Cannot mutate member on value"),
                        },
                    };
                    if let Some(msg) = err {
                        return Err(self.error(span, msg));
                    }
                }
                Opcode::MakeClosure(fn_idx) => {
                    let target_fn = self.frames[frame_idx].closure.function.chunk.functions
                        [fn_idx as usize]
                        .clone();
                    let stack_base = self.frames[frame_idx].stack_base;
                    let parent_closure = self.frames[frame_idx].closure.clone();
                    let mut upvals = Vec::with_capacity(target_fn.upvalue_descriptors.len());
                    for desc in &target_fn.upvalue_descriptors {
                        if desc.is_local {
                            let slot = stack_base + desc.index as usize;
                            let upval = if let Some(existing) = self
                                .open_upvalues
                                .iter()
                                .find(|u| matches!(*u.borrow(), Upvalue::Open(s) if s == slot))
                            {
                                existing.clone()
                            } else {
                                let created = Rc::new(RefCell::new(Upvalue::Open(slot)));
                                self.open_upvalues.push(created.clone());
                                created
                            };
                            upvals.push(upval);
                        } else {
                            let parent_upval = parent_closure.upvalues[desc.index as usize].clone();
                            upvals.push(parent_upval);
                        }
                    }
                    let closure = BytecodeClosure {
                        function: target_fn,
                        upvalues: upvals,
                    };
                    self.stack.push(Value::BytecodeFunction(Rc::new(closure)));
                }
                Opcode::GetGlobal(name_idx) => {
                    let name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [name_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    if let Some(val) = self.globals.get(&name) {
                        self.stack.push(val.clone());
                    } else if let Some((_, builtin)) =
                        builtin_catalog().iter().find(|(n, _)| *n == name.as_str())
                    {
                        self.stack.push(Value::Builtin(*builtin));
                    } else {
                        return Err(self.error(span, format!("Undefined variable '{name}'")));
                    }
                }
                Opcode::SetGlobal(name_idx) => {
                    let name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [name_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let val = self.stack.last().cloned().unwrap_or(Value::Null);
                    self.globals.insert(name, val);
                }
                Opcode::Add => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            self.stack.push(Value::Number(a + b))
                        }
                        (Value::String(a), Value::String(b)) => {
                            self.stack.push(Value::String(format!("{a}{b}")));
                        }
                        (Value::Vector(a), Value::Vector(b)) if a.len() == b.len() => {
                            let res = a.iter().zip(&b).map(|(x, y)| x + y).collect();
                            self.stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operands for +")),
                    }
                }
                Opcode::Sub => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            self.stack.push(Value::Number(a - b))
                        }
                        (Value::Vector(a), Value::Vector(b)) if a.len() == b.len() => {
                            let res = a.iter().zip(&b).map(|(x, y)| x - y).collect();
                            self.stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operands for -")),
                    }
                }
                Opcode::Mul => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            self.stack.push(Value::Number(a * b))
                        }
                        (Value::Vector(a), Value::Number(b)) => {
                            let res = a.iter().map(|x| x * b).collect();
                            self.stack.push(Value::Vector(res));
                        }
                        (Value::Number(a), Value::Vector(b)) => {
                            let res = b.iter().map(|x| x * a).collect();
                            self.stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operands for *")),
                    }
                }
                Opcode::Div => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            if b == 0.0 {
                                return Err(self.error(span, "Division by zero"));
                            }
                            self.stack.push(Value::Number(a / b));
                        }
                        _ => return Err(self.error(span, "Invalid operands for /")),
                    }
                }
                Opcode::Mod => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            if b == 0.0 {
                                return Err(self.error(span, "Division by zero"));
                            }
                            self.stack.push(Value::Number(exact_remainder(a, b)));
                        }
                        _ => return Err(self.error(span, "Invalid operands for %")),
                    }
                }
                Opcode::Neg => {
                    let val = self.stack.pop().unwrap();
                    match val {
                        Value::Number(n) => self.stack.push(Value::Number(-n)),
                        Value::Vector(v) => {
                            let res = v.into_iter().map(|x| -x).collect();
                            self.stack.push(Value::Vector(res));
                        }
                        _ => return Err(self.error(span, "Invalid operand for negation")),
                    }
                }
                Opcode::Not => {
                    let val = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(!is_truthy(&val)));
                }
                Opcode::Equal => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(left == right));
                }
                Opcode::NotEqual => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(left != right));
                }
                Opcode::Less => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => self.stack.push(Value::Bool(a < b)),
                        _ => return Err(self.error(span, "Invalid operands for <")),
                    }
                }
                Opcode::LessEqual => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            self.stack.push(Value::Bool(a <= b))
                        }
                        _ => return Err(self.error(span, "Invalid operands for <=")),
                    }
                }
                Opcode::Greater => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => self.stack.push(Value::Bool(a > b)),
                        _ => return Err(self.error(span, "Invalid operands for >")),
                    }
                }
                Opcode::GreaterEqual => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    match (left, right) {
                        (Value::Number(a), Value::Number(b)) => {
                            self.stack.push(Value::Bool(a >= b))
                        }
                        _ => return Err(self.error(span, "Invalid operands for >=")),
                    }
                }
                Opcode::Jump(target) => {
                    self.frames[frame_idx].ip = target;
                }
                Opcode::JumpIfFalse(target) => {
                    let cond = self.stack.pop().unwrap_or(Value::Null);
                    if !is_truthy(&cond) {
                        self.frames[frame_idx].ip = target;
                    }
                }
                Opcode::JumpIfTrue(target) => {
                    let cond = self.stack.pop().unwrap_or(Value::Null);
                    if is_truthy(&cond) {
                        self.frames[frame_idx].ip = target;
                    }
                }
                Opcode::BuildList(count) => {
                    let start = self.stack.len() - count;
                    let items = self.stack.drain(start..).collect();
                    self.stack.push(Value::List(items));
                }
                Opcode::BuildTuple(count) => {
                    let start = self.stack.len() - count;
                    let items = self.stack.drain(start..).collect();
                    self.stack.push(Value::Tuple(items));
                }
                Opcode::BuildRecord(count) => {
                    let start = self.stack.len() - (count * 2);
                    let drained: Vec<_> = self.stack.drain(start..).collect();
                    let mut fields = BTreeMap::new();
                    for chunk in drained.as_chunks::<2>().0 {
                        let key = match &chunk[0] {
                            Value::String(s) => s.clone(),
                            _ => return Err(self.error(span, "Record key must be a string")),
                        };
                        fields.insert(key, chunk[1].clone());
                    }
                    self.stack.push(Value::Record(fields));
                }
                Opcode::IndexGet => {
                    let idx = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap();
                    match (&obj, &idx) {
                        (Value::List(items), Value::Number(n)) => {
                            let i = *n as usize;
                            if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                self.stack.push(items[i].clone());
                            } else {
                                return Err(self.error(span, "Index out of bounds"));
                            }
                        }
                        (Value::Tuple(items), Value::Number(n)) => {
                            let i = *n as usize;
                            if *n >= 0.0 && n.fract() == 0.0 && i < items.len() {
                                self.stack.push(items[i].clone());
                            } else {
                                return Err(self.error(span, "Index out of bounds"));
                            }
                        }
                        (Value::Record(fields), Value::String(k)) => {
                            if let Some(val) = fields.get(k) {
                                self.stack.push(val.clone());
                            } else {
                                return Err(self.error(span, format!("Field '{k}' not found")));
                            }
                        }
                        _ => return Err(self.error(span, "Cannot index value")),
                    }
                }
                Opcode::IndexSet(slot) => {
                    let val = self.stack.pop().unwrap();
                    let idx = self.stack.pop().unwrap();
                    let stack_base = self.frames[frame_idx].stack_base;
                    let target_slot = stack_base + slot as usize;
                    match (&mut self.stack[target_slot], &idx) {
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
                    let field_name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [field_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let obj = self.stack.pop().unwrap();
                    match &obj {
                        Value::Record(fields) => {
                            if let Some(val) = fields.get(&field_name) {
                                self.stack.push(val.clone());
                            } else {
                                return Err(
                                    self.error(span, format!("Field '{field_name}' not found"))
                                );
                            }
                        }
                        Value::Vector(v) => {
                            let val = match field_name.as_str() {
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
                            self.stack.push(Value::Number(val));
                        }
                        _ => return Err(self.error(span, "Cannot access member on value")),
                    }
                }
                Opcode::MemberSet(slot, field_idx) => {
                    let field_name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [field_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let val = self.stack.pop().unwrap();
                    let stack_base = self.frames[frame_idx].stack_base;
                    let target_slot = stack_base + slot as usize;
                    match &mut self.stack[target_slot] {
                        Value::Record(fields) => {
                            fields.insert(field_name, val);
                        }
                        _ => return Err(self.error(span, "Cannot mutate member on value")),
                    }
                }
                Opcode::Interpolate(count) => {
                    let start = self.stack.len() - count;
                    let parts: Vec<_> = self.stack.drain(start..).collect();
                    let mut out = String::new();
                    for part in parts {
                        format_value_for_display(&part, &mut out, 0);
                    }
                    self.stack.push(Value::String(out));
                }
                Opcode::Call(arg_count) => {
                    let callee_idx = self.stack.len() - arg_count - 1;
                    let callee = self.stack[callee_idx].clone();
                    match callee {
                        Value::Builtin(b) => {
                            let args: Vec<_> = self.stack.drain(callee_idx + 1..).collect();
                            self.stack.pop(); // pop callee
                            let res = self.call_builtin(b, &args, span)?;
                            self.stack.push(res);
                        }
                        Value::BytecodeFunction(target_closure) => {
                            if arg_count != target_closure.function.arity {
                                return Err(self.error(
                                    span,
                                    format!(
                                        "Argument count mismatch: expected {}, got {}",
                                        target_closure.function.arity, arg_count
                                    ),
                                ));
                            }
                            if self.frames.len() >= 512 {
                                return Err(self.error(span, "Call stack overflow"));
                            }
                            let stack_base = callee_idx + 1;
                            let needed = stack_base + target_closure.function.chunk.local_count;
                            if self.stack.len() < needed {
                                self.stack.resize(needed, Value::Null);
                            }
                            self.frames.push(CallFrame {
                                closure: target_closure,
                                ip: 0,
                                stack_base,
                            });
                        }
                        _ => return Err(self.error(span, "Value is not callable")),
                    }
                }
                Opcode::CheckTuple(expected_len) => {
                    let val = self.stack.pop().unwrap();
                    let matches = match val {
                        Value::Tuple(items) => items.len() == expected_len,
                        _ => false,
                    };
                    self.stack.push(Value::Bool(matches));
                }
                Opcode::AssertTuple(expected_len) => {
                    let val = self.stack.pop().unwrap();
                    match val {
                        Value::Tuple(items) if items.len() == expected_len => {}
                        _ => return Err(self.error(span, "Value does not match binding pattern")),
                    }
                }
                Opcode::TupleGet(idx) => {
                    let val = self.stack.pop().unwrap();
                    match val {
                        Value::Tuple(items) => {
                            let item = items.get(idx as usize).cloned().unwrap_or(Value::Null);
                            self.stack.push(item);
                        }
                        _ => return Err(self.error(span, "Expected tuple")),
                    }
                }
                Opcode::CheckRecord => {
                    let val = self.stack.pop().unwrap();
                    let matches = match &val {
                        Value::Record(_) => true,
                        Value::UserData(data) if data.variant.is_none() => {
                            matches!(data.values.first(), Some(Value::Record(_)))
                        }
                        _ => false,
                    };
                    self.stack.push(Value::Bool(matches));
                }
                Opcode::AssertRecord => {
                    let val = self.stack.pop().unwrap();
                    match &val {
                        Value::Record(_) => {}
                        Value::UserData(data)
                            if data.variant.is_none()
                                && matches!(data.values.first(), Some(Value::Record(_))) => {}
                        _ => return Err(self.error(span, "Value does not match binding pattern")),
                    }
                }
                Opcode::CheckRecordField(field_idx) => {
                    let field_name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [field_idx as usize]
                    {
                        Value::String(s) => s.as_str(),
                        _ => unreachable!(),
                    };
                    let val = self.stack.pop().unwrap();
                    let contains = match &val {
                        Value::Record(fields) => fields.contains_key(field_name),
                        Value::UserData(data) if data.variant.is_none() => {
                            if let Some(Value::Record(fields)) = data.values.first() {
                                fields.contains_key(field_name)
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    self.stack.push(Value::Bool(contains));
                }
                Opcode::RecordGetAssert(field_idx) => {
                    let field_name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [field_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let val = self.stack.pop().unwrap();
                    let found = match &val {
                        Value::Record(fields) => fields.get(&field_name).cloned(),
                        Value::UserData(data) if data.variant.is_none() => {
                            if let Some(Value::Record(fields)) = data.values.first() {
                                fields.get(&field_name).cloned()
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    match found {
                        Some(v) => self.stack.push(v),
                        None => return Err(self.error(span, "Missing record pattern field")),
                    }
                }
                Opcode::CheckVariant(tag_idx, expected_arity) => {
                    let tag = match &self.frames[frame_idx].closure.function.chunk.constants
                        [tag_idx as usize]
                    {
                        Value::String(s) => s.as_str(),
                        _ => unreachable!(),
                    };
                    let val = self.stack.pop().unwrap();
                    let matches = match &val {
                        Value::Variant(t, items) => *t == tag && items.len() == expected_arity,
                        Value::UserData(data) => {
                            let variant_matches = data.variant.as_deref() == Some(tag)
                                || (data.variant.is_none() && data.type_name.as_str() == tag);
                            variant_matches && data.values.len() == expected_arity
                        }
                        _ => false,
                    };
                    self.stack.push(Value::Bool(matches));
                }
                Opcode::VariantGet(idx) => {
                    let val = self.stack.pop().unwrap();
                    let item = match &val {
                        Value::Variant(_, items) => {
                            items.get(idx as usize).cloned().unwrap_or(Value::Null)
                        }
                        Value::UserData(data) => data
                            .values
                            .get(idx as usize)
                            .cloned()
                            .unwrap_or(Value::Null),
                        _ => return Err(self.error(span, "Expected variant")),
                    };
                    self.stack.push(item);
                }
                Opcode::MatchError => {
                    return Err(self.error(span, "No matching pattern"));
                }
                Opcode::MakeUserData {
                    type_name_idx,
                    variant_idx,
                    arity,
                } => {
                    let type_name = match &self.frames[frame_idx].closure.function.chunk.constants
                        [type_name_idx as usize]
                    {
                        Value::String(s) => s.clone(),
                        _ => unreachable!(),
                    };
                    let variant = variant_idx.map(|idx| {
                        match &self.frames[frame_idx].closure.function.chunk.constants[idx as usize]
                        {
                            Value::String(s) => s.clone(),
                            _ => unreachable!(),
                        }
                    });
                    let start = self.stack.len() - arity;
                    let values = self.stack.drain(start..).collect();
                    self.stack.push(Value::UserData(Box::new(UserData {
                        type_name,
                        variant,
                        values,
                    })));
                }
                Opcode::Return => {
                    let ret_val = self.stack.pop().unwrap_or(Value::Null);
                    let exiting = self.frames.pop().unwrap();
                    self.close_upvalues(exiting.stack_base);
                    if self.frames.is_empty() {
                        return Ok(ret_val);
                    }
                    if exiting.stack_base > 0 {
                        self.stack.truncate(exiting.stack_base - 1);
                    } else {
                        self.stack.clear();
                    }
                    self.stack.push(ret_val);
                }
            }
        }

        Ok(self.stack.pop().unwrap_or(Value::Null))
    }

    pub fn invoke_callable(
        &mut self,
        callable: &Value<'static>,
        args: &[Value<'static>],
        span: Span,
    ) -> Result<Value<'static>, RuntimeError> {
        match callable {
            Value::BytecodeFunction(closure) => self.run_closure(closure, args),
            Value::Builtin(b) => self.call_builtin(*b, args, span),
            _ => Err(self.error(span, "Value is not callable")),
        }
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
            Builtin::Map => {
                if args.len() != 2 {
                    return Err(self.error(span, "map requires 2 arguments"));
                }
                let (Value::List(items), callback) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "map requires a list and a callback"));
                };
                let mut output = Vec::with_capacity(items.len());
                for item in items {
                    let res = self.invoke_callable(callback, std::slice::from_ref(item), span)?;
                    output.push(res);
                }
                Ok(Value::List(output))
            }
            Builtin::Filter => {
                if args.len() != 2 {
                    return Err(self.error(span, "filter requires 2 arguments"));
                }
                let (Value::List(items), callback) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "filter requires a list and a callback"));
                };
                let mut output = Vec::new();
                for item in items {
                    let res = self.invoke_callable(callback, std::slice::from_ref(item), span)?;
                    if is_truthy(&res) {
                        output.push(item.clone());
                    }
                }
                Ok(Value::List(output))
            }
            Builtin::Fold => {
                if args.len() != 3 {
                    return Err(self.error(span, "fold requires 3 arguments"));
                }
                let (Value::List(items), initial, callback) = (&args[0], &args[1], &args[2]) else {
                    return Err(self.error(
                        span,
                        "fold requires a list, an initial value and a callback",
                    ));
                };
                let mut acc = initial.clone();
                for item in items {
                    acc = self.invoke_callable(callback, &[acc, item.clone()], span)?;
                }
                Ok(acc)
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
            Builtin::Sqrt => {
                let Some(Value::Number(n)) = args.first() else {
                    return Err(self.error(span, "sqrt requires a number"));
                };
                if *n < 0.0 {
                    return Err(self.error(span, "sqrt requires a non-negative number"));
                }
                Ok(Value::Number(n.sqrt()))
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
            Builtin::Some => Ok(Value::Variant("Some", args.to_vec())),
            Builtin::None => Ok(Value::Variant("None", Vec::new())),
            Builtin::Ok => Ok(Value::Variant("Ok", args.to_vec())),
            Builtin::Err => Ok(Value::Variant("Err", args.to_vec())),
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
