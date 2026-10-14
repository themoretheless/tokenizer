//! AST to Bytecode compiler with constant folding, compile-time builtin resolution, and superinstruction fusion.
use crate::bytecode::opcode::{BytecodeFunction, Chunk, Opcode, UpvalueDescriptor};
use crate::runtime::builtins::lookup_builtin;
use crate::runtime::ops::{eval_binary_values, eval_unary_value, is_truthy};
use crate::{Builtin, Expr, ExprKind, InterpolationPart, RuntimeError, Stmt, StmtKind, Value};
use std::collections::HashMap;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

/// Evaluates compile-time constants for peephole folding.
pub(crate) fn try_eval_const(expr: &Expr<'_>) -> Option<Value<'static>> {
    match &expr.kind {
        ExprKind::Number(text) => {
            let n = if text.as_bytes().contains(&b'_') {
                text.replace('_', "").parse::<f64>().ok()?
            } else {
                text.parse::<f64>().ok()?
            };
            n.is_finite().then_some(Value::Number(n))
        }
        ExprKind::String(text) => {
            let mut decoded = String::with_capacity(text.len());
            for c in crate::string_literal::characters(text) {
                decoded.push(c.ok()?);
            }
            Some(Value::String(decoded))
        }
        ExprKind::Bool(b) => Some(Value::Bool(*b)),
        ExprKind::Null => Some(Value::Null),
        ExprKind::Unary { operator, value } => {
            let inner = try_eval_const(value)?;
            eval_unary_value(operator, inner, false).ok()
        }
        ExprKind::Binary {
            operator,
            left,
            right,
        } => {
            let l = try_eval_const(left)?;
            if *operator == "&&" {
                return if !is_truthy(&l) {
                    Some(l)
                } else {
                    try_eval_const(right)
                };
            }
            if *operator == "||" {
                return if is_truthy(&l) {
                    Some(l)
                } else {
                    try_eval_const(right)
                };
            }
            let r = try_eval_const(right)?;
            eval_binary_values(operator, l, r).ok()
        }
        ExprKind::If {
            condition,
            then_value,
            else_value,
        } => {
            let cond = try_eval_const(condition)?;
            if is_truthy(&cond) {
                try_eval_const(then_value)
            } else {
                try_eval_const(else_value)
            }
        }
        _ => None,
    }
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

pub(crate) struct Compiler<'s> {
    states: Vec<CompilerState>,
    _phantom: std::marker::PhantomData<&'s ()>,
}

impl<'s> Compiler<'s> {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn pop_state(&mut self) -> BytecodeFunction {
        let mut state = self.states.pop().expect("Compiler state stack underflow");
        state.chunk.local_count = state.local_count as usize;
        BytecodeFunction {
            name: state.name,
            arity: state.arity,
            chunk: state.chunk,
            upvalue_descriptors: state.upvalues,
        }
    }

    fn resolve_variable(&mut self, name: &str) -> VariableResolution {
        if let Some(slot) = self.current_ref().resolve_local(name) {
            return VariableResolution::Local(slot);
        }
        let cur_idx = self.states.len() - 1;
        if let Some(upval_idx) = self.resolve_upvalue(cur_idx, name) {
            return VariableResolution::Upvalue(upval_idx);
        }
        if let Some(builtin) = lookup_builtin(name) {
            return VariableResolution::Builtin(builtin);
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

    pub(crate) fn compile_module(&mut self, items: &[Stmt<'s>]) -> Result<(), RuntimeError> {
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
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(slot), stmt.span);
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
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(slot), stmt.span);
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
            StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                self.compile_expr(iterable)?;
                let iter_slot = self.current().declare_temp_local();
                self.current()
                    .chunk
                    .emit(Opcode::IterInit(iter_slot), stmt.span);
                let start_ip = self.current().chunk.code.len();
                let exit_jump = self.current().chunk.emit(
                    Opcode::IterNext {
                        iter_slot,
                        exit_jump: 0,
                    },
                    stmt.span,
                );
                self.current().loops.push(LoopContext {
                    start_ip,
                    break_jumps: Vec::new(),
                });
                self.current().push_scope();
                let binding_slot = self.current().declare_local(binding.text);
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(binding_slot), binding.span);
                self.compile_block(&body.stmts, false)?;
                self.current().pop_scope();
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
                    .emit(Opcode::StoreLocal(val_slot), stmt.span);
                self.compile_destructure_binding(pattern, val_slot, stmt.span)?;
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Struct { name, .. } => {
                let slot = self.current().declare_local(name.text);
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
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(slot), stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Enum { name, variants } => {
                let slot = self.current().declare_local(name.text);
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
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(slot), stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Region { budget, body, .. } => {
                if let Some(budget_expr) = budget {
                    self.compile_expr(budget_expr)?;
                    self.current()
                        .chunk
                        .emit(Opcode::AssertRegionBudget, budget_expr.span);
                }
                self.current().push_scope();
                self.compile_block(&body.stmts, keep_result)?;
                self.current().pop_scope();
            }
            StmtKind::Show(expr) | StmtKind::Expr(expr) => {
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

    fn emit_constant_value(&mut self, val: Value<'static>, span: Span) {
        match val {
            Value::Null => {
                self.current().chunk.emit(Opcode::Null, span);
            }
            Value::Bool(true) => {
                self.current().chunk.emit(Opcode::True, span);
            }
            Value::Bool(false) => {
                self.current().chunk.emit(Opcode::False, span);
            }
            Value::Builtin(b) => {
                self.current().chunk.emit(Opcode::GetBuiltin(b), span);
            }
            v => {
                let idx = self.current().chunk.add_constant(v);
                self.current().chunk.emit(Opcode::Constant(idx), span);
            }
        }
    }

    fn compile_expr(&mut self, expr: &Expr<'s>) -> Result<(), RuntimeError> {
        let span = expr.span;
        if let Some(const_val) = try_eval_const(expr) {
            self.emit_constant_value(const_val, span);
            return Ok(());
        }
        match &expr.kind {
            ExprKind::Try(value) => {
                self.compile_expr(value)?;
                self.current().chunk.emit(Opcode::TryPropagate, span);
            }
            ExprKind::Number(text) => {
                let n: f64 = if text.as_bytes().contains(&b'_') {
                    text.replace('_', "")
                        .parse()
                        .map_err(|_| self.error(span, "Invalid number"))?
                } else {
                    text.parse()
                        .map_err(|_| self.error(span, "Invalid number"))?
                };
                let idx = self.current().chunk.add_constant(Value::Number(n));
                self.current().chunk.emit(Opcode::Constant(idx), span);
            }
            ExprKind::String(text) => {
                let mut decoded = String::with_capacity(text.len());
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
                    self.current().chunk.emit(Opcode::GetBuiltin(builtin), span);
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
                    "+" => {
                        self.current().chunk.emit(Opcode::Pos, span);
                    }
                    "!" | "not" => {
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
                if *operator == "&&" || *operator == "and" {
                    self.compile_expr(left)?;
                    self.current().chunk.emit(Opcode::Dup, span);
                    let false_jump = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_expr(right)?;
                    let len = self.current().chunk.code.len();
                    self.current().chunk.patch_jump(false_jump, len);
                    return Ok(());
                }
                if *operator == "||" || *operator == "or" {
                    self.compile_expr(left)?;
                    self.current().chunk.emit(Opcode::Dup, span);
                    let true_jump = self.current().chunk.emit(Opcode::JumpIfTrue(0), span);
                    self.current().chunk.emit(Opcode::Pop, span);
                    self.compile_expr(right)?;
                    let len = self.current().chunk.code.len();
                    self.current().chunk.patch_jump(true_jump, len);
                    return Ok(());
                }

                // Superinstruction fast-paths when right operand is a compile-time numeric constant
                if let Some(Value::Number(imm)) = try_eval_const(right) {
                    if let ExprKind::Name(left_name) = &left.kind
                        && let VariableResolution::Local(slot) =
                            self.resolve_variable(left_name.text)
                    {
                        match *operator {
                            "+" => {
                                self.current()
                                    .chunk
                                    .emit(Opcode::LocalAddImm(slot, imm), span);
                                return Ok(());
                            }
                            "-" => {
                                self.current()
                                    .chunk
                                    .emit(Opcode::LocalSubImm(slot, imm), span);
                                return Ok(());
                            }
                            "*" => {
                                self.current()
                                    .chunk
                                    .emit(Opcode::LocalMulImm(slot, imm), span);
                                return Ok(());
                            }
                            "<=" => {
                                self.current()
                                    .chunk
                                    .emit(Opcode::LocalLessEqualImm(slot, imm), span);
                                return Ok(());
                            }
                            _ => {}
                        }
                    }

                    match *operator {
                        "+" => {
                            self.compile_expr(left)?;
                            self.current().chunk.emit(Opcode::AddImm(imm), span);
                            return Ok(());
                        }
                        "-" => {
                            self.compile_expr(left)?;
                            self.current().chunk.emit(Opcode::SubImm(imm), span);
                            return Ok(());
                        }
                        "*" => {
                            self.compile_expr(left)?;
                            self.current().chunk.emit(Opcode::MulImm(imm), span);
                            return Ok(());
                        }
                        "<" => {
                            self.compile_expr(left)?;
                            self.current().chunk.emit(Opcode::LessImm(imm), span);
                            return Ok(());
                        }
                        "<=" => {
                            self.compile_expr(left)?;
                            self.current().chunk.emit(Opcode::LessEqualImm(imm), span);
                            return Ok(());
                        }
                        ">" => {
                            self.compile_expr(left)?;
                            self.current().chunk.emit(Opcode::GreaterImm(imm), span);
                            return Ok(());
                        }
                        ">=" => {
                            self.compile_expr(left)?;
                            self.current()
                                .chunk
                                .emit(Opcode::GreaterEqualImm(imm), span);
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                self.compile_expr(left)?;
                self.compile_expr(right)?;
                match *operator {
                    "+" => self.current().chunk.emit(Opcode::Add, span),
                    "-" => self.current().chunk.emit(Opcode::Sub, span),
                    "*" => self.current().chunk.emit(Opcode::Mul, span),
                    "/" => self.current().chunk.emit(Opcode::Div, span),
                    "%" => self.current().chunk.emit(Opcode::Mod, span),
                    "**" => self.current().chunk.emit(Opcode::Pow, span),
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
                    if *operator == "+="
                        && let VariableResolution::Local(slot) = res
                    {
                        if let Some(Value::Number(imm)) = try_eval_const(value) {
                            self.current()
                                .chunk
                                .emit(Opcode::AddAssignLocalImm(slot, imm), span);
                            return Ok(());
                        }
                        self.compile_expr(value)?;
                        self.current()
                            .chunk
                            .emit(Opcode::AddAssignLocal(slot), span);
                        return Ok(());
                    }

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
                            "**" => self.current().chunk.emit(Opcode::Pow, span),
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
                if let ExprKind::Name(name) = &callee.kind
                    && let VariableResolution::Builtin(builtin) = self.resolve_variable(name.text)
                {
                    for arg in arguments {
                        self.compile_expr(arg)?;
                    }
                    self.current()
                        .chunk
                        .emit(Opcode::CallBuiltin(builtin, arguments.len()), span);
                    return Ok(());
                }
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
                            if let ExprKind::Name(name) = &callee.kind
                                && let VariableResolution::Builtin(builtin) =
                                    self.resolve_variable(name.text)
                            {
                                // Fast path: input is already on top of stack!
                                // Push remaining arguments and emit CallBuiltin(builtin, argc + 1)
                                for arg in arguments {
                                    self.compile_expr(arg)?;
                                }
                                self.current().chunk.emit(
                                    Opcode::CallBuiltin(builtin, arguments.len() + 1),
                                    stage.span,
                                );
                                continue;
                            }
                            let tmp_slot = self.current().declare_temp_local();
                            self.current()
                                .chunk
                                .emit(Opcode::StoreLocal(tmp_slot), stage.span);
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
                        ExprKind::Name(name) => {
                            if let VariableResolution::Builtin(builtin) =
                                self.resolve_variable(name.text)
                            {
                                self.current()
                                    .chunk
                                    .emit(Opcode::CallBuiltin(builtin, 1), stage.span);
                                continue;
                            }
                            let tmp_slot = self.current().declare_temp_local();
                            self.current()
                                .chunk
                                .emit(Opcode::StoreLocal(tmp_slot), stage.span);
                            self.compile_expr(stage)?;
                            self.current()
                                .chunk
                                .emit(Opcode::GetLocal(tmp_slot), stage.span);
                            self.current().chunk.emit(Opcode::Call(1), stage.span);
                        }
                        _ => {
                            let tmp_slot = self.current().declare_temp_local();
                            self.current()
                                .chunk
                                .emit(Opcode::StoreLocal(tmp_slot), stage.span);
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
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(val_slot), span);

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
                    self.current().chunk.emit(Opcode::StoreLocal(slot), span);
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
                    self.current()
                        .chunk
                        .emit(Opcode::StoreLocal(elem_slot), span);
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
                        .emit(Opcode::StoreLocal(field_slot), span);
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
                        .emit(Opcode::StoreLocal(payload_slot), span);
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
                    self.current().chunk.emit(Opcode::StoreLocal(slot), span);
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
                    self.current()
                        .chunk
                        .emit(Opcode::StoreLocal(elem_slot), span);
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
                        .emit(Opcode::StoreLocal(field_slot), span);
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
