use super::pattern::extract_pattern_name;
use super::{Compiler, LoopContext, type_annotation_has_int};
use crate::bytecode::opcode::{BytecodeFunction, Chunk, Opcode};
use crate::{ExprKind, RuntimeError, Stmt, StmtKind, Value};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

impl<'s> Compiler<'s> {
    pub(super) fn compile_block(
        &mut self,
        items: &[Stmt<'s>],
        keep_result: bool,
    ) -> Result<(), RuntimeError> {
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
                is_async,
            } = &stmt.kind
            {
                if *is_async {
                    return Err(self.error(
                        stmt.span,
                        "async functions are not supported in bytecode compilation",
                    ));
                }
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

    pub(super) fn compile_stmt(
        &mut self,
        stmt: &Stmt<'s>,
        keep_result: bool,
    ) -> Result<(), RuntimeError> {
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
            StmtKind::IfLet {
                pattern,
                value,
                then_block,
                else_block,
            } => {
                self.compile_expr(value)?;
                let val_slot = self.current().declare_temp_local();
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(val_slot), stmt.span);
                self.current().push_scope();
                let mut fail_jumps = Vec::new();
                self.compile_pattern_match(pattern, val_slot, &mut fail_jumps)?;
                self.compile_block(&then_block.stmts, keep_result)?;
                self.current().pop_scope();
                let end_jump = self.current().chunk.emit(Opcode::Jump(0), stmt.span);
                let else_target = self.current().chunk.code.len();
                for fj in fail_jumps {
                    self.current().chunk.patch_jump(fj, else_target);
                }
                if let Some(else_b) = else_block {
                    self.current().push_scope();
                    self.compile_block(&else_b.stmts, keep_result)?;
                    self.current().pop_scope();
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
            StmtKind::LetElse {
                pattern,
                value,
                else_block,
            } => {
                if !crate::analysis::block_diverges(&else_block.stmts) {
                    return Err(self.error(
                        else_block.span,
                        "let-else fallback block must diverge (e.g. return, break, continue, or ?)",
                    ));
                }
                self.compile_expr(value)?;
                let val_slot = self.current().declare_temp_local();
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(val_slot), stmt.span);
                self.current().push_scope();
                let inner_depth = self.current_ref().scope_depth;
                let mut fail_jumps = Vec::new();
                self.compile_pattern_match(pattern, val_slot, &mut fail_jumps)?;
                let bound_locals: Vec<(&'s str, u32)> = self
                    .current_ref()
                    .locals
                    .iter()
                    .filter(|l| l.depth == inner_depth)
                    .map(|l| (l.name, l.slot))
                    .collect();
                let matched_jump = self.current().chunk.emit(Opcode::Jump(0), stmt.span);
                self.current().pop_scope();
                let else_target = self.current().chunk.code.len();
                for fj in fail_jumps {
                    self.current().chunk.patch_jump(fj, else_target);
                }
                self.current().push_scope();
                self.compile_block(&else_block.stmts, false)?;
                self.current().pop_scope();
                self.current()
                    .chunk
                    .emit(Opcode::MatchError, else_block.span);
                let matched_target = self.current().chunk.code.len();
                self.current()
                    .chunk
                    .patch_jump(matched_jump, matched_target);
                let cur_depth = self.current_ref().scope_depth;
                for (name, slot) in bound_locals {
                    if let Some(existing) = self
                        .current()
                        .locals
                        .iter_mut()
                        .rev()
                        .take_while(|l| l.depth == cur_depth)
                        .find(|l| l.name == name)
                    {
                        existing.slot = slot;
                    } else {
                        self.current().locals.push(super::Local {
                            name,
                            depth: cur_depth,
                            slot,
                        });
                    }
                }
                if keep_result {
                    self.current()
                        .chunk
                        .emit(Opcode::GetLocal(val_slot), stmt.span);
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

    pub(super) fn compile_type_decl(&mut self, stmt: &Stmt<'s>) -> Result<(), RuntimeError> {
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
}
