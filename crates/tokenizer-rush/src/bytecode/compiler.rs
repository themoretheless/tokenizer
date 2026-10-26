//! AST to Bytecode compiler with constant folding, compile-time builtin resolution, and superinstruction fusion.
use crate::bytecode::opcode::{
    BytecodeFunction, Chunk, Opcode, PathTarget, RecordShapeDescriptor, UpvalueDescriptor,
};
use crate::runtime::builtins::lookup_builtin;
use crate::runtime::ops::{eval_binary_values, eval_unary_value, is_truthy};
use crate::{
    Builtin, Expr, ExprKind, InterpolationPart, Name, RecordMap, RuntimeError, Stmt, StmtKind,
    Value,
};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

/// Evaluates compile-time constants for peephole folding.
pub(crate) fn try_eval_const(expr: &Expr<'_>) -> Option<Value<'static>> {
    match &expr.kind {
        ExprKind::Number(text) => {
            if crate::runtime::is_int_literal(text) {
                crate::runtime::parse_int_literal(text).map(Value::Int)
            } else {
                crate::runtime::parse_f64_literal(text).map(Value::Number)
            }
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
            if *operator == "-"
                && let ExprKind::Number(text) = &value.kind
                && crate::runtime::is_int_literal(text)
            {
                return crate::runtime::parse_neg_int_literal(text).map(Value::Int);
            }
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

fn type_annotation_has_int(ty: &crate::Type<'_>) -> bool {
    matches!(&*ty.qualified_name(), "int" | "i64")
        || ty.arguments.iter().any(type_annotation_has_int)
}

struct LoopContext {
    start_ip: usize,
    break_jumps: Vec<usize>,
}

#[derive(Clone, Copy)]
struct Local<'s> {
    name: &'s str,
    depth: u32,
    slot: u32,
}

struct CompilerState<'s> {
    name: Option<String>,
    arity: usize,
    chunk: Chunk,
    locals: Vec<Local<'s>>,
    scope_depth: u32,
    local_count: u32,
    upvalues: Vec<UpvalueDescriptor>,
    loops: Vec<LoopContext>,
    coerce_return_int: bool,
}

impl<'s> CompilerState<'s> {
    fn new(name: Option<String>, arity: usize) -> Self {
        Self {
            name,
            arity,
            chunk: Chunk::new(),
            locals: Vec::with_capacity(16),
            scope_depth: 0,
            local_count: arity as u32,
            upvalues: Vec::new(),
            loops: Vec::new(),
            coerce_return_int: false,
        }
    }

    #[inline]
    fn resolve_local(&self, name: &str) -> Option<u32> {
        self.locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.slot)
    }

    #[inline]
    fn declare_local(&mut self, name: &'s str) -> u32 {
        let slot = self.local_count;
        self.local_count += 1;
        let depth = self.scope_depth;
        if let Some(existing) = self
            .locals
            .iter_mut()
            .rev()
            .take_while(|l| l.depth == depth)
            .find(|l| l.name == name)
        {
            existing.slot = slot;
        } else {
            self.locals.push(Local { name, depth, slot });
        }
        slot
    }

    #[inline]
    fn declare_param(&mut self, name: &'s str, slot: u32) {
        self.locals.push(Local {
            name,
            depth: 0,
            slot,
        });
    }

    #[inline]
    fn declare_temp_local(&mut self) -> u32 {
        let slot = self.local_count;
        self.local_count += 1;
        slot
    }

    #[inline]
    fn push_scope(&mut self) {
        self.scope_depth += 1;
    }

    #[inline]
    fn pop_scope(&mut self) {
        let depth = self.scope_depth;
        while self.locals.last().is_some_and(|l| l.depth == depth) {
            self.locals.pop();
        }
        self.scope_depth -= 1;
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
    states: Vec<CompilerState<'s>>,
}

impl<'s> Compiler<'s> {
    pub(crate) fn new() -> Self {
        Self {
            states: vec![CompilerState::new(Some("<main>".into()), 0)],
        }
    }

    fn current(&mut self) -> &mut CompilerState<'s> {
        self.states.last_mut().expect("Compiler state stack empty")
    }

    fn current_ref(&self) -> &CompilerState<'s> {
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
        for stmt in items {
            if matches!(stmt.kind, StmtKind::Struct { .. } | StmtKind::Enum { .. }) {
                self.compile_type_decl(stmt)?;
            }
        }
        let mut hoisted_fns = Vec::new();
        for (i, stmt) in items.iter().enumerate() {
            if let StmtKind::Function {
                name, parameters, ..
            } = &stmt.kind
            {
                let slot = self.current().declare_local(name.text);
                let placeholder = Rc::new(BytecodeFunction {
                    name: Some(name.text.to_string()),
                    arity: parameters.len(),
                    chunk: Chunk::new(),
                    upvalue_descriptors: Vec::new(),
                });
                let fn_idx = self.current().chunk.add_function(placeholder);
                self.current()
                    .chunk
                    .emit(Opcode::MakeClosure(fn_idx), stmt.span);
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(slot), stmt.span);
                hoisted_fns.push((i, fn_idx));
            }
        }
        let mut fn_cursor = 0;
        for (i, stmt) in items.iter().enumerate() {
            let is_last = i + 1 == items.len();
            if let StmtKind::Function {
                name,
                parameters,
                result,
                body,
                ..
            } = &stmt.kind
            {
                let (_, fn_idx) = hoisted_fns[fn_cursor];
                fn_cursor += 1;
                self.push_state(Some(name.text.to_string()), parameters.len());
                self.current().coerce_return_int =
                    result.as_ref().is_some_and(type_annotation_has_int);
                for (p_idx, p) in parameters.iter().enumerate() {
                    if let Some(p_name) = extract_pattern_name(&p.pattern) {
                        self.current().declare_param(p_name, p_idx as u32);
                    }
                    if p.ty.as_ref().is_some_and(type_annotation_has_int) {
                        self.current()
                            .chunk
                            .emit(Opcode::GetLocal(p_idx as u32), p.pattern.span);
                        self.current().chunk.emit(Opcode::CoerceInt, p.pattern.span);
                        self.current()
                            .chunk
                            .emit(Opcode::StoreLocal(p_idx as u32), p.pattern.span);
                    }
                }
                self.compile_block(&body.stmts, false)?;
                self.current().chunk.emit(Opcode::Null, stmt.span);
                self.current().chunk.emit(Opcode::Return, stmt.span);
                let compiled = self.pop_state();
                self.current().chunk.functions[fn_idx as usize] = Rc::new(compiled);
                if is_last && keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            } else {
                self.compile_stmt(stmt, is_last && keep_result)?;
            }
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt<'s>, keep_result: bool) -> Result<(), RuntimeError> {
        match &stmt.kind {
            StmtKind::Declaration {
                name, value, ty, ..
            } => {
                self.compile_expr(value)?;
                if ty.as_ref().is_some_and(type_annotation_has_int) {
                    self.current().chunk.emit(Opcode::CoerceInt, stmt.span);
                }
                let slot = self.current().declare_local(name.text);
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(slot), stmt.span);
                if keep_result {
                    self.current().chunk.emit(Opcode::Null, stmt.span);
                }
            }
            StmtKind::Function { .. } => {
                unreachable!("functions are compiled via compile_block hoisting");
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
                    if matches!(val.kind, ExprKind::Call { .. })
                        && let Some(Opcode::Call(argc)) = self.current().chunk.code.last().copied()
                    {
                        *self.current().chunk.code.last_mut().expect("present") =
                            Opcode::TailCall(argc);
                    }
                    if self.current().coerce_return_int {
                        self.current().chunk.emit(Opcode::CoerceInt, stmt.span);
                    }
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
            StmtKind::Struct { .. } | StmtKind::Enum { .. } => {
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

    fn compile_type_decl(&mut self, stmt: &Stmt<'s>) -> Result<(), RuntimeError> {
        match &stmt.kind {
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
            }
            _ => unreachable!("type declaration"),
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
                let val = if crate::runtime::is_int_literal(text) {
                    let i = crate::runtime::parse_int_literal(text)
                        .ok_or_else(|| self.error(span, "Invalid integer literal"))?;
                    Value::Int(i)
                } else {
                    let n = crate::runtime::parse_f64_literal(text)
                        .ok_or_else(|| self.error(span, "Invalid number literal"))?;
                    Value::Number(n)
                };
                let idx = self.current().chunk.add_constant(val);
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
                if *operator == "-"
                    && let ExprKind::Number(text) = &value.kind
                    && crate::runtime::is_int_literal(text)
                {
                    let i = crate::runtime::parse_neg_int_literal(text)
                        .ok_or_else(|| self.error(span, "Invalid integer literal"))?;
                    let idx = self.current().chunk.add_constant(Value::Int(i));
                    self.current().chunk.emit(Opcode::Constant(idx), span);
                    return Ok(());
                }
                self.compile_expr(value)?;
                match *operator {
                    "-" => {
                        self.current().chunk.emit(Opcode::Neg, span);
                    }
                    "+" => {
                        self.current().chunk.emit(Opcode::Pos, span);
                    }
                    "~" => {
                        self.current().chunk.emit(Opcode::BitNot, span);
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
                match try_eval_const(right) {
                    Some(Value::Number(imm)) => {
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
                                "%" if imm != 0.0 => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalModImm(slot, imm), span);
                                    return Ok(());
                                }
                                "&" => {
                                    if let Some(int_imm) = Value::Number(imm).as_exact_i64() {
                                        self.current()
                                            .chunk
                                            .emit(Opcode::LocalBitAndIntImm(slot, int_imm), span);
                                        return Ok(());
                                    }
                                }
                                "==" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalEqualImm(slot, imm), span);
                                    return Ok(());
                                }
                                "<" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalLessImm(slot, imm), span);
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
                            "%" if imm != 0.0 => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::ModImm(imm), span);
                                return Ok(());
                            }
                            "&" => {
                                if let Some(int_imm) = Value::Number(imm).as_exact_i64() {
                                    self.compile_expr(left)?;
                                    self.current()
                                        .chunk
                                        .emit(Opcode::BitAndIntImm(int_imm), span);
                                    return Ok(());
                                }
                            }
                            "^" => {
                                if let Some(int_imm) = Value::Number(imm).as_exact_i64() {
                                    self.compile_expr(left)?;
                                    self.current()
                                        .chunk
                                        .emit(Opcode::BitXorIntImm(int_imm), span);
                                    return Ok(());
                                }
                            }
                            "<<" => {
                                if let Some(int_imm) = Value::Number(imm).as_exact_i64()
                                    && let Ok(shift) = u32::try_from(int_imm)
                                    && shift < 64
                                {
                                    self.compile_expr(left)?;
                                    self.current().chunk.emit(Opcode::ShlIntImm(shift), span);
                                    return Ok(());
                                }
                            }
                            ">>" => {
                                if let Some(int_imm) = Value::Number(imm).as_exact_i64()
                                    && let Ok(shift) = u32::try_from(int_imm)
                                    && shift < 64
                                {
                                    self.compile_expr(left)?;
                                    self.current().chunk.emit(Opcode::ShrIntImm(shift), span);
                                    return Ok(());
                                }
                            }
                            "==" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::EqualImm(imm), span);
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
                    Some(Value::Int(imm)) => {
                        if let ExprKind::Name(left_name) = &left.kind
                            && let VariableResolution::Local(slot) =
                                self.resolve_variable(left_name.text)
                        {
                            match *operator {
                                "+" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalAddIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "-" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalSubIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "*" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalMulIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "%" if imm != 0 && imm != -1 => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalModIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "&" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalBitAndIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "==" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalEqualIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "<" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalLessIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                "<=" => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::LocalLessEqualIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }

                        match *operator {
                            "+" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::AddIntImm(imm), span);
                                return Ok(());
                            }
                            "-" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::SubIntImm(imm), span);
                                return Ok(());
                            }
                            "*" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::MulIntImm(imm), span);
                                return Ok(());
                            }
                            "%" if imm != 0 && imm != -1 => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::ModIntImm(imm), span);
                                return Ok(());
                            }
                            "&" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::BitAndIntImm(imm), span);
                                return Ok(());
                            }
                            "^" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::BitXorIntImm(imm), span);
                                return Ok(());
                            }
                            "<<" if u32::try_from(imm).is_ok_and(|s| s < 64) => {
                                self.compile_expr(left)?;
                                self.current()
                                    .chunk
                                    .emit(Opcode::ShlIntImm(imm as u32), span);
                                return Ok(());
                            }
                            ">>" if u32::try_from(imm).is_ok_and(|s| s < 64) => {
                                self.compile_expr(left)?;
                                self.current()
                                    .chunk
                                    .emit(Opcode::ShrIntImm(imm as u32), span);
                                return Ok(());
                            }
                            "==" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::EqualIntImm(imm), span);
                                return Ok(());
                            }
                            "<" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::LessIntImm(imm), span);
                                return Ok(());
                            }
                            "<=" => {
                                self.compile_expr(left)?;
                                self.current()
                                    .chunk
                                    .emit(Opcode::LessEqualIntImm(imm), span);
                                return Ok(());
                            }
                            ">" => {
                                self.compile_expr(left)?;
                                self.current().chunk.emit(Opcode::GreaterIntImm(imm), span);
                                return Ok(());
                            }
                            ">=" => {
                                self.compile_expr(left)?;
                                self.current()
                                    .chunk
                                    .emit(Opcode::GreaterEqualIntImm(imm), span);
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }

                // Symmetric superinstruction fast-paths when left operand is a compile-time numeric constant
                if matches!(*operator, "+" | "*" | "==") {
                    match try_eval_const(left) {
                        Some(Value::Number(imm)) => {
                            if let ExprKind::Name(right_name) = &right.kind
                                && let VariableResolution::Local(slot) =
                                    self.resolve_variable(right_name.text)
                            {
                                let op = match *operator {
                                    "+" => Opcode::LocalAddImm(slot, imm),
                                    "*" => Opcode::LocalMulImm(slot, imm),
                                    _ => Opcode::LocalEqualImm(slot, imm),
                                };
                                self.current().chunk.emit(op, span);
                                return Ok(());
                            }
                            self.compile_expr(right)?;
                            let op = match *operator {
                                "+" => Opcode::AddImm(imm),
                                "*" => Opcode::MulImm(imm),
                                _ => Opcode::EqualImm(imm),
                            };
                            self.current().chunk.emit(op, span);
                            return Ok(());
                        }
                        Some(Value::Int(imm)) => {
                            if let ExprKind::Name(right_name) = &right.kind
                                && let VariableResolution::Local(slot) =
                                    self.resolve_variable(right_name.text)
                            {
                                let op = match *operator {
                                    "+" => Opcode::LocalAddIntImm(slot, imm),
                                    "*" => Opcode::LocalMulIntImm(slot, imm),
                                    _ => Opcode::LocalEqualIntImm(slot, imm),
                                };
                                self.current().chunk.emit(op, span);
                                return Ok(());
                            }
                            self.compile_expr(right)?;
                            let op = match *operator {
                                "+" => Opcode::AddIntImm(imm),
                                "*" => Opcode::MulIntImm(imm),
                                _ => Opcode::EqualIntImm(imm),
                            };
                            self.current().chunk.emit(op, span);
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
                    "&" => self.current().chunk.emit(Opcode::BitAnd, span),
                    "^" => self.current().chunk.emit(Opcode::BitXor, span),
                    "<<" => self.current().chunk.emit(Opcode::Shl, span),
                    ">>" => self.current().chunk.emit(Opcode::Shr, span),
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
                    if let VariableResolution::Local(slot) = res {
                        if *operator == "+=" {
                            match try_eval_const(value) {
                                Some(Value::Number(imm)) => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::AddAssignLocalImm(slot, imm), span);
                                    return Ok(());
                                }
                                Some(Value::Int(imm)) => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::AddAssignLocalIntImm(slot, imm), span);
                                    return Ok(());
                                }
                                _ => {}
                            }
                            self.compile_expr(value)?;
                            self.current()
                                .chunk
                                .emit(Opcode::AddAssignLocal(slot), span);
                            return Ok(());
                        }
                        if *operator == "-=" {
                            match try_eval_const(value) {
                                Some(Value::Number(imm)) => {
                                    self.current()
                                        .chunk
                                        .emit(Opcode::AddAssignLocalImm(slot, -imm), span);
                                    return Ok(());
                                }
                                Some(Value::Int(imm)) => {
                                    if let Some(neg) = imm.checked_neg() {
                                        self.current()
                                            .chunk
                                            .emit(Opcode::AddAssignLocalIntImm(slot, neg), span);
                                        return Ok(());
                                    }
                                }
                                _ => {}
                            }
                        }
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
                            "&" => self.current().chunk.emit(Opcode::BitAnd, span),
                            "^" => self.current().chunk.emit(Opcode::BitXor, span),
                            "<<" => self.current().chunk.emit(Opcode::Shl, span),
                            ">>" => self.current().chunk.emit(Opcode::Shr, span),
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
                ExprKind::Index { .. } | ExprKind::Member { .. } => {
                    let Some((root_name, steps)) = extract_lvalue_path(target) else {
                        return Err(self.error(
                            target.span,
                            "Assignment requires a variable, member or index",
                        ));
                    };
                    if steps.len() > 32 {
                        return Err(self.error(target.span, "Assignment path is too deep"));
                    }
                    let res = self.resolve_variable(root_name.text);
                    let path_target = match res {
                        VariableResolution::Local(slot) => PathTarget::Local(slot),
                        VariableResolution::Upvalue(idx) => PathTarget::Upvalue(idx),
                        VariableResolution::Global(n) => {
                            let idx = self.current().chunk.add_constant(Value::String(n));
                            PathTarget::Global(idx)
                        }
                        VariableResolution::Builtin(_) => {
                            return Err(self.error(root_name.span, "Cannot mutate builtin"));
                        }
                    };
                    if steps.len() == 1
                        && *operator == "="
                        && matches!(path_target, PathTarget::Local(_) | PathTarget::Upvalue(_))
                    {
                        match (&steps[0], path_target) {
                            (LvalueStep::Index(index), PathTarget::Local(slot)) => {
                                self.compile_expr(index)?;
                                self.compile_expr(value)?;
                                self.current()
                                    .chunk
                                    .emit(Opcode::IndexSet(slot), target.span);
                            }
                            (LvalueStep::Index(index), PathTarget::Upvalue(idx)) => {
                                self.compile_expr(index)?;
                                self.compile_expr(value)?;
                                self.current()
                                    .chunk
                                    .emit(Opcode::IndexSetUpvalue(idx), target.span);
                            }
                            (LvalueStep::Member(field), PathTarget::Local(slot)) => {
                                self.compile_expr(value)?;
                                let field_idx = self
                                    .current()
                                    .chunk
                                    .add_constant(Value::String(field.text.to_string()));
                                self.current()
                                    .chunk
                                    .emit(Opcode::MemberSet(slot, field_idx), target.span);
                            }
                            (LvalueStep::Member(field), PathTarget::Upvalue(idx)) => {
                                self.compile_expr(value)?;
                                let field_idx = self
                                    .current()
                                    .chunk
                                    .add_constant(Value::String(field.text.to_string()));
                                self.current()
                                    .chunk
                                    .emit(Opcode::MemberSetUpvalue(idx, field_idx), target.span);
                            }
                            _ => unreachable!(),
                        }
                        return Ok(());
                    }
                    let mut member_mask = 0u32;
                    for (i, step) in steps.iter().enumerate() {
                        match step {
                            LvalueStep::Index(index) => {
                                self.compile_expr(index)?;
                            }
                            LvalueStep::Member(field) => {
                                member_mask |= 1u32 << i;
                                let idx = self
                                    .current()
                                    .chunk
                                    .add_constant(Value::String(field.text.to_string()));
                                self.current().chunk.emit(Opcode::Constant(idx), field.span);
                            }
                        }
                    }
                    let depth = steps.len() as u8;
                    if *operator == "=" {
                        self.compile_expr(value)?;
                    } else {
                        let base_op = operator.strip_suffix('=').unwrap_or(operator);
                        self.current().chunk.emit(
                            Opcode::GetPath {
                                target: path_target,
                                depth,
                                member_mask,
                            },
                            target.span,
                        );
                        self.compile_expr(value)?;
                        match base_op {
                            "+" => self.current().chunk.emit(Opcode::Add, span),
                            "-" => self.current().chunk.emit(Opcode::Sub, span),
                            "*" => self.current().chunk.emit(Opcode::Mul, span),
                            "/" => self.current().chunk.emit(Opcode::Div, span),
                            "%" => self.current().chunk.emit(Opcode::Mod, span),
                            "**" => self.current().chunk.emit(Opcode::Pow, span),
                            "&" => self.current().chunk.emit(Opcode::BitAnd, span),
                            "^" => self.current().chunk.emit(Opcode::BitXor, span),
                            "<<" => self.current().chunk.emit(Opcode::Shl, span),
                            ">>" => self.current().chunk.emit(Opcode::Shr, span),
                            _ => return Err(self.error(span, "Unsupported assignment operator")),
                        };
                    }
                    self.current().chunk.emit(
                        Opcode::SetPath {
                            target: path_target,
                            depth,
                            member_mask,
                        },
                        target.span,
                    );
                }
                _ => return Err(self.error(target.span, "Unsupported assignment expression")),
            },
            ExprKind::Index { object, index } => {
                if let ExprKind::Name(name) = &object.kind
                    && !expr_may_mutate(index)
                {
                    match self.resolve_variable(name.text) {
                        VariableResolution::Local(slot) => {
                            self.compile_expr(index)?;
                            self.current().chunk.emit(Opcode::LocalIndexGet(slot), span);
                            return Ok(());
                        }
                        VariableResolution::Upvalue(upval_idx) => {
                            self.compile_expr(index)?;
                            self.current()
                                .chunk
                                .emit(Opcode::UpvalueIndexGet(upval_idx), span);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                self.compile_expr(object)?;
                self.compile_expr(index)?;
                self.current().chunk.emit(Opcode::IndexGet, span);
            }
            ExprKind::Member { object, field } => {
                let field_idx = self
                    .current()
                    .chunk
                    .add_constant(Value::String(field.text.to_string()));
                if let ExprKind::Name(name) = &object.kind {
                    match self.resolve_variable(name.text) {
                        VariableResolution::Local(slot) => {
                            self.current()
                                .chunk
                                .emit(Opcode::LocalMemberGet(slot, field_idx), span);
                            return Ok(());
                        }
                        VariableResolution::Upvalue(upval_idx) => {
                            self.current()
                                .chunk
                                .emit(Opcode::UpvalueMemberGet(upval_idx, field_idx), span);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                self.compile_expr(object)?;
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
                let mut static_keys = Vec::with_capacity(entries.len());
                let mut all_static_unique = true;
                for (key, _) in entries {
                    let k_opt = match &key.kind {
                        ExprKind::Name(n) => Some(n.text.to_string()),
                        _ => match try_eval_const(key) {
                            Some(Value::String(s)) => Some(s),
                            _ => None,
                        },
                    };
                    if let Some(k) = k_opt {
                        if static_keys.contains(&k) {
                            all_static_unique = false;
                            break;
                        }
                        static_keys.push(k);
                    } else {
                        all_static_unique = false;
                        break;
                    }
                }

                if all_static_unique {
                    if let Some(const_vals) = entries
                        .iter()
                        .map(|(_, val)| try_eval_const(val))
                        .collect::<Option<Vec<_>>>()
                    {
                        let pairs: Vec<(String, Value<'static>)> =
                            static_keys.into_iter().zip(const_vals).collect();
                        let record = Value::Record(RecordMap::from_unique_entries(pairs));
                        self.emit_constant_value(record, span);
                        return Ok(());
                    }

                    for (_, val) in entries {
                        self.compile_expr(val)?;
                    }
                    let is_sorted = static_keys.windows(2).all(|w| w[0] < w[1]);
                    let desc = if is_sorted {
                        RecordShapeDescriptor {
                            shape: Rc::from(static_keys),
                            source_to_slot: None,
                        }
                    } else {
                        let mut indexed: Vec<(usize, String)> =
                            static_keys.into_iter().enumerate().collect();
                        indexed.sort_by(|(_, a), (_, b)| a.cmp(b));
                        let mut sorted_keys = Vec::with_capacity(indexed.len());
                        let mut source_to_slot = vec![0usize; indexed.len()];
                        for (slot_idx, (src_idx, key)) in indexed.into_iter().enumerate() {
                            sorted_keys.push(key);
                            source_to_slot[src_idx] = slot_idx;
                        }
                        RecordShapeDescriptor {
                            shape: Rc::from(sorted_keys),
                            source_to_slot: Some(source_to_slot.into_boxed_slice()),
                        }
                    };
                    let shape_idx = self.current().chunk.add_record_shape(desc);
                    self.current()
                        .chunk
                        .emit(Opcode::BuildRecordShape(shape_idx), span);
                } else {
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
                        self.current().declare_param(p_name, i as u32);
                    }
                }
                self.compile_expr(body)?;
                if matches!(body.kind, ExprKind::Call { .. })
                    && let Some(Opcode::Call(argc)) = self.current().chunk.code.last().copied()
                {
                    *self.current().chunk.code.last_mut().expect("present") =
                        Opcode::TailCall(argc);
                }
                self.current().chunk.emit(Opcode::Return, span);
                let compiled = self.pop_state();
                let fn_idx = self.current().chunk.add_function(Rc::new(compiled));
                self.current().chunk.emit(Opcode::MakeClosure(fn_idx), span);
            }
            ExprKind::Call { callee, arguments } => {
                if let ExprKind::Name(name) = &callee.kind
                    && let VariableResolution::Builtin(builtin) = self.resolve_variable(name.text)
                {
                    if matches!(builtin, Builtin::Vec2 | Builtin::Vec3 | Builtin::Vec4) {
                        let expected_dim = match builtin {
                            Builtin::Vec2 => 2,
                            Builtin::Vec3 => 3,
                            _ => 4,
                        };
                        if arguments.len() == expected_dim {
                            let mut comps = crate::InlineVec::new();
                            let mut all_const = true;
                            for arg in arguments {
                                if let Some(val) = try_eval_const(arg)
                                    && let Some(n) = val.as_f64()
                                    && n.is_finite()
                                {
                                    comps.push(n);
                                } else {
                                    all_const = false;
                                    break;
                                }
                            }
                            if all_const {
                                self.emit_constant_value(Value::Vector(comps), span);
                                return Ok(());
                            }
                        }
                    }
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
                let mut shared_tmp_slot: Option<u32> = None;
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
                            let tmp_slot = *shared_tmp_slot
                                .get_or_insert_with(|| self.current().declare_temp_local());
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
                            let tmp_slot = *shared_tmp_slot
                                .get_or_insert_with(|| self.current().declare_temp_local());
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
                            let tmp_slot = *shared_tmp_slot
                                .get_or_insert_with(|| self.current().declare_temp_local());
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
            ExprKind::Await(_) => {
                return Err(self.error(span, "await is not supported in bytecode compilation"));
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
                    if matches!(&subpattern.kind, ExprKind::Name(n) if n.text == "_") {
                        continue;
                    }
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
                    if matches!(&subpattern.kind, ExprKind::Name(n) if n.text == "_") {
                        continue;
                    }
                    self.current()
                        .chunk
                        .emit(Opcode::LocalMemberGet(target_slot, field_idx), span);
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
                    if matches!(&arg_pattern.kind, ExprKind::Name(n) if n.text == "_") {
                        continue;
                    }
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
                match try_eval_const(pattern) {
                    Some(Value::Number(imm)) => {
                        self.current()
                            .chunk
                            .emit(Opcode::LocalEqualImm(target_slot, imm), span);
                        let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                        fail_jumps.push(jmp);
                        return Ok(());
                    }
                    Some(Value::Int(imm)) => {
                        self.current()
                            .chunk
                            .emit(Opcode::LocalEqualIntImm(target_slot, imm), span);
                        let jmp = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
                        fail_jumps.push(jmp);
                        return Ok(());
                    }
                    _ => {}
                }
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
                    if matches!(&item.kind, ExprKind::Name(n) if n.text == "_") {
                        continue;
                    }
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
                    if matches!(&subpattern.kind, ExprKind::Name(n) if n.text == "_") {
                        self.current().chunk.emit(Opcode::Pop, span);
                        continue;
                    }
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

fn expr_may_mutate(expr: &Expr<'_>) -> bool {
    match &expr.kind {
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Null
        | ExprKind::Name(_) => false,
        ExprKind::Unary { value, .. } | ExprKind::Try(value) => expr_may_mutate(value),
        ExprKind::Binary { left, right, .. }
        | ExprKind::Index {
            object: left,
            index: right,
        } => expr_may_mutate(left) || expr_may_mutate(right),
        ExprKind::Member { object, .. } => expr_may_mutate(object),
        ExprKind::List(items) | ExprKind::Tuple(items) => items.iter().any(expr_may_mutate),
        ExprKind::Map(entries) => entries
            .iter()
            .any(|(k, v)| expr_may_mutate(k) || expr_may_mutate(v)),
        ExprKind::If {
            condition,
            then_value,
            else_value,
        } => {
            expr_may_mutate(condition) || expr_may_mutate(then_value) || expr_may_mutate(else_value)
        }
        _ => true,
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

enum LvalueStep<'a, 's> {
    Index(&'a Expr<'s>),
    Member(&'a Name<'s>),
}

fn extract_lvalue_path<'a, 's>(
    expr: &'a Expr<'s>,
) -> Option<(&'a Name<'s>, Vec<LvalueStep<'a, 's>>)> {
    match &expr.kind {
        ExprKind::Name(name) => Some((name, Vec::new())),
        ExprKind::Index { object, index } => {
            let (root, mut steps) = extract_lvalue_path(object)?;
            steps.push(LvalueStep::Index(index));
            Some((root, steps))
        }
        ExprKind::Member { object, field } => {
            let (root, mut steps) = extract_lvalue_path(object)?;
            steps.push(LvalueStep::Member(field));
            Some((root, steps))
        }
        _ => None,
    }
}
