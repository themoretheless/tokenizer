use super::{Compiler, try_eval_const};
use crate::bytecode::opcode::Opcode;
use crate::{Expr, ExprKind, RuntimeError, Value};
use themoretheless_tokenizer_core::Span;

pub(super) fn extract_callee_tag(expr: &Expr<'_>) -> Option<String> {
    match &expr.kind {
        ExprKind::Name(n) => Some(n.text.to_string()),
        ExprKind::Member { field, .. } => Some(field.text.to_string()),
        _ => None,
    }
}

pub(super) fn extract_pattern_name<'s>(expr: &Expr<'s>) -> Option<&'s str> {
    match &expr.kind {
        ExprKind::Name(n) => Some(n.text),
        _ => None,
    }
}

impl<'s> Compiler<'s> {
    pub(super) fn compile_pattern_match(
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

    pub(super) fn compile_destructure_binding(
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
