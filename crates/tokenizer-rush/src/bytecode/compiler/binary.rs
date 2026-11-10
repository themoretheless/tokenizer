use super::{Compiler, VariableResolution, try_eval_const};
use crate::bytecode::opcode::{Opcode, PathTarget};
use crate::{Expr, ExprKind, Name, RuntimeError, Value};
use themoretheless_tokenizer_core::Span;

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

impl<'s> Compiler<'s> {
    pub(super) fn compile_binary_expr(
        &mut self,
        operator: &str,
        left: &Expr<'s>,
        right: &Expr<'s>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if operator == "&&" || operator == "and" {
            self.compile_expr(left)?;
            self.current().chunk.emit(Opcode::Dup, span);
            let false_jump = self.current().chunk.emit(Opcode::JumpIfFalse(0), span);
            self.current().chunk.emit(Opcode::Pop, span);
            self.compile_expr(right)?;
            let len = self.current().chunk.code.len();
            self.current().chunk.patch_jump(false_jump, len);
            return Ok(());
        }
        if operator == "||" || operator == "or" {
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
                    && let VariableResolution::Local(slot) = self.resolve_variable(left_name.text)
                {
                    match operator {
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

                match operator {
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
                    && let VariableResolution::Local(slot) = self.resolve_variable(left_name.text)
                {
                    match operator {
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

                match operator {
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
        if matches!(operator, "+" | "*" | "==") {
            match try_eval_const(left) {
                Some(Value::Number(imm)) => {
                    if let ExprKind::Name(right_name) = &right.kind
                        && let VariableResolution::Local(slot) =
                            self.resolve_variable(right_name.text)
                    {
                        let op = match operator {
                            "+" => Opcode::LocalAddImm(slot, imm),
                            "*" => Opcode::LocalMulImm(slot, imm),
                            _ => Opcode::LocalEqualImm(slot, imm),
                        };
                        self.current().chunk.emit(op, span);
                        return Ok(());
                    }
                    self.compile_expr(right)?;
                    let op = match operator {
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
                        let op = match operator {
                            "+" => Opcode::LocalAddIntImm(slot, imm),
                            "*" => Opcode::LocalMulIntImm(slot, imm),
                            _ => Opcode::LocalEqualIntImm(slot, imm),
                        };
                        self.current().chunk.emit(op, span);
                        return Ok(());
                    }
                    self.compile_expr(right)?;
                    let op = match operator {
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
        match operator {
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
        Ok(())
    }

    pub(super) fn compile_assign_expr(
        &mut self,
        target: &Expr<'s>,
        value: &Expr<'s>,
        operator: &str,
        span: Span,
    ) -> Result<(), RuntimeError> {
        match &target.kind {
            ExprKind::Name(name) => {
                let res = self.resolve_variable(name.text);
                if let VariableResolution::Local(slot) = res {
                    if operator == "+=" {
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
                    if operator == "-=" {
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

                if operator == "=" {
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
                            let idx = self.current().chunk.add_constant(Value::String(n.clone()));
                            self.current()
                                .chunk
                                .emit(Opcode::GetGlobal(idx), target.span);
                        }
                        VariableResolution::Builtin(_) => {
                            return Err(self.error(target.span, "Cannot reassign builtin"));
                        }
                    }
                    self.compile_expr(value)?;
                    self.emit_compound_op(base_op, span)?;
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
                    && operator == "="
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
                if operator == "=" {
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
                    self.emit_compound_op(base_op, span)?;
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
        }
        Ok(())
    }

    #[inline]
    fn emit_compound_op(&mut self, base_op: &str, span: Span) -> Result<(), RuntimeError> {
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
        Ok(())
    }
}
