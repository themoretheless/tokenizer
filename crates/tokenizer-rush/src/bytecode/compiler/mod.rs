//! AST to Bytecode compiler with constant folding, compile-time builtin resolution, and superinstruction fusion.

mod binary;
mod expr;
mod pattern;
mod stmt;

use crate::bytecode::opcode::{BytecodeFunction, Chunk, Opcode, UpvalueDescriptor};
use crate::runtime::builtins::lookup_builtin;
use crate::runtime::ops::{eval_binary_values, eval_unary_value, is_truthy};
use crate::{Builtin, Expr, ExprKind, RuntimeError, Stmt, Value};
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

pub(super) fn type_annotation_has_int(ty: &crate::Type<'_>) -> bool {
    matches!(&*ty.qualified_name(), "int" | "i64")
        || ty.arguments.iter().any(type_annotation_has_int)
}

pub(super) struct LoopContext {
    pub(super) start_ip: usize,
    pub(super) break_jumps: Vec<usize>,
}

#[derive(Clone, Copy)]
pub(super) struct Local<'s> {
    pub(super) name: &'s str,
    pub(super) depth: u32,
    pub(super) slot: u32,
}

pub(super) struct CompilerState<'s> {
    pub(super) name: Option<String>,
    pub(super) arity: usize,
    pub(super) chunk: Chunk,
    pub(super) locals: Vec<Local<'s>>,
    pub(super) scope_depth: u32,
    pub(super) local_count: u32,
    pub(super) upvalues: Vec<UpvalueDescriptor>,
    pub(super) loops: Vec<LoopContext>,
    pub(super) coerce_return_int: bool,
}

impl<'s> CompilerState<'s> {
    pub(super) fn new(name: Option<String>, arity: usize) -> Self {
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
    pub(super) fn resolve_local(&self, name: &str) -> Option<u32> {
        self.locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.slot)
    }

    #[inline]
    pub(super) fn declare_local(&mut self, name: &'s str) -> u32 {
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
    pub(super) fn declare_param(&mut self, name: &'s str, slot: u32) {
        self.locals.push(Local {
            name,
            depth: 0,
            slot,
        });
    }

    #[inline]
    pub(super) fn declare_temp_local(&mut self) -> u32 {
        let slot = self.local_count;
        self.local_count += 1;
        slot
    }

    #[inline]
    pub(super) fn push_scope(&mut self) {
        self.scope_depth += 1;
    }

    #[inline]
    pub(super) fn pop_scope(&mut self) {
        let depth = self.scope_depth;
        while self.locals.last().is_some_and(|l| l.depth == depth) {
            self.locals.pop();
        }
        self.scope_depth -= 1;
    }

    pub(super) fn add_upvalue(&mut self, desc: UpvalueDescriptor) -> u32 {
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

pub(super) enum VariableResolution {
    Local(u32),
    Upvalue(u32),
    Builtin(Builtin),
    Global(String),
}

pub(crate) struct Compiler<'s> {
    states: Vec<CompilerState<'s>>,
    pub(super) user_types: std::collections::HashSet<String>,
}

impl<'s> Compiler<'s> {
    pub(crate) fn new() -> Self {
        Self {
            states: vec![CompilerState::new(Some("<main>".into()), 0)],
            user_types: std::collections::HashSet::new(),
        }
    }

    #[inline]
    pub(super) fn current(&mut self) -> &mut CompilerState<'s> {
        self.states.last_mut().expect("Compiler state stack empty")
    }

    #[inline]
    pub(super) fn current_ref(&self) -> &CompilerState<'s> {
        self.states.last().expect("Compiler state stack empty")
    }

    pub(super) fn push_state(&mut self, name: Option<String>, arity: usize) {
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

    pub(super) fn resolve_variable(&mut self, name: &str) -> VariableResolution {
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

    pub(super) fn error(&self, span: Span, message: impl Into<String>) -> RuntimeError {
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

    pub(super) fn emit_constant_value(&mut self, val: Value<'static>, span: Span) {
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
}
