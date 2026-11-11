use super::pattern::extract_pattern_name;
use super::{Compiler, VariableResolution, try_eval_const};
use crate::bytecode::opcode::{Opcode, RecordShapeDescriptor};
use crate::{Builtin, Expr, ExprKind, InterpolationPart, RecordMap, RuntimeError, Value};
use std::rc::Rc;

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
            condition: scrutinee,
            then_value,
            else_value,
        }
        | ExprKind::IfLet {
            value: scrutinee,
            then_value,
            else_value,
            ..
        } => {
            expr_may_mutate(scrutinee) || expr_may_mutate(then_value) || expr_may_mutate(else_value)
        }
        _ => true,
    }
}

impl<'s> Compiler<'s> {
    pub(super) fn compile_expr(&mut self, expr: &Expr<'s>) -> Result<(), RuntimeError> {
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
                self.compile_binary_expr(operator, left, right, span)?;
            }
            ExprKind::Assign {
                target,
                value,
                operator,
            } => {
                self.compile_assign_expr(target, value, operator, span)?;
            }
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
            ExprKind::IfLet {
                pattern,
                value,
                then_value,
                else_value,
            } => {
                self.compile_expr(value)?;
                let val_slot = self.current().declare_temp_local();
                self.current()
                    .chunk
                    .emit(Opcode::StoreLocal(val_slot), span);
                self.current().push_scope();
                let mut fail_jumps = Vec::new();
                self.compile_pattern_match(pattern, val_slot, &mut fail_jumps)?;
                self.compile_expr(then_value)?;
                self.current().pop_scope();
                let end_jump = self.current().chunk.emit(Opcode::Jump(0), span);
                let else_target = self.current().chunk.code.len();
                for fj in fail_jumps {
                    self.current().chunk.patch_jump(fj, else_target);
                }
                self.compile_expr(else_value)?;
                let end_len = self.current().chunk.code.len();
                self.current().chunk.patch_jump(end_jump, end_len);
            }
            ExprKind::Lambda {
                parameters,
                body,
                is_async,
            } => {
                if *is_async {
                    return Err(self.error(
                        span,
                        "async closures are not supported in bytecode compilation",
                    ));
                }
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
            ExprKind::TryBlock(_) => {
                return Err(
                    self.error(span, "try blocks are not supported in bytecode compilation")
                );
            }
            _ => {
                return Err(self.error(span, "Expression not supported in bytecode compilation"));
            }
        }
        Ok(())
    }
}
