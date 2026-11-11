//! Arithmetic, bitwise, comparison, and superinstruction opcode handlers.
use super::Vm;
use crate::bytecode::opcode::Opcode;
use crate::runtime::ops::{eval_unary_value, is_truthy};
use crate::{RuntimeError, Value};
use themoretheless_tokenizer_core::Span;

const MAX_SAFE_INT_F64: f64 = 9_007_199_254_740_991.0;

#[inline(always)]
fn is_safe_int_f64(imm: f64) -> bool {
    imm.is_finite() && imm.fract() == 0.0 && imm.abs() <= MAX_SAFE_INT_F64
}

impl<'c> Vm<'c> {
    #[inline(always)]
    pub(super) fn exec_numeric_op(
        &mut self,
        op: Opcode,
        frame_idx: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        match op {
            Opcode::Add => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(Value::Number(a)), Value::Number(b)) => {
                        let n = *a + *b;
                        if n.is_finite() {
                            *a = n;
                            return Ok(());
                        }
                    }
                    (Some(Value::Int(a)), Value::Int(b)) => {
                        if let Some(n) = a.checked_add(*b) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                        let mut next = *a;
                        for (x, y) in next.iter_mut().zip(b.as_slice()) {
                            *x += *y;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *a = next;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("+", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Sub => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(Value::Number(a)), Value::Number(b)) => {
                        let n = *a - *b;
                        if n.is_finite() {
                            *a = n;
                            return Ok(());
                        }
                    }
                    (Some(Value::Int(a)), Value::Int(b)) => {
                        if let Some(n) = a.checked_sub(*b) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                        let mut next = *a;
                        for (x, y) in next.iter_mut().zip(b.as_slice()) {
                            *x -= *y;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *a = next;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("-", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Mul => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(Value::Number(a)), Value::Number(b)) => {
                        let n = *a * *b;
                        if n.is_finite() {
                            *a = n;
                            return Ok(());
                        }
                    }
                    (Some(Value::Int(a)), Value::Int(b)) => {
                        if let Some(n) = a.checked_mul(*b) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    (Some(Value::Vector(a)), Value::Number(b)) => {
                        let mut next = *a;
                        for x in &mut next {
                            *x *= *b;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *a = next;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("*", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Div => {
                let right = self.stack.pop().unwrap();
                if let (Some(Value::Vector(a)), Value::Number(b)) = (self.stack.last_mut(), &right)
                {
                    let mut next = *a;
                    for x in &mut next {
                        *x /= *b;
                    }
                    if next.iter().all(|x| x.is_finite()) {
                        *a = next;
                        return Ok(());
                    }
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("/", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Mod => {
                let right = self.stack.pop().unwrap();
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("%", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Pow => {
                let right = self.stack.pop().unwrap();
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("**", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::BitAnd => {
                let right = self.stack.pop().unwrap();
                if let (Some(Value::Int(a)), Value::Int(b)) = (self.stack.last_mut(), &right) {
                    *a &= *b;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("&", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::BitXor => {
                let right = self.stack.pop().unwrap();
                if let (Some(Value::Int(a)), Value::Int(b)) = (self.stack.last_mut(), &right) {
                    *a ^= *b;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("^", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Shl => {
                let right = self.stack.pop().unwrap();
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<<", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Shr => {
                let right = self.stack.pop().unwrap();
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">>", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Neg => {
                match self.stack.last_mut() {
                    Some(Value::Number(n)) if (-*n).is_finite() => {
                        *n = -*n;
                        return Ok(());
                    }
                    Some(Value::Int(i)) => {
                        if let Some(neg) = i.checked_neg() {
                            *i = neg;
                            return Ok(());
                        }
                    }
                    Some(Value::Vector(v)) => {
                        let mut next = *v;
                        for x in &mut next {
                            *x = -*x;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *v = next;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let val = self.stack.pop().unwrap();
                let res = eval_unary_value("-", val, false).map_err(|m| self.error(span, m))?;
                self.stack.push(res);
            }
            Opcode::Pos => {
                let val = self.stack.pop().unwrap();
                let res = eval_unary_value("+", val, false).map_err(|m| self.error(span, m))?;
                self.stack.push(res);
            }
            Opcode::BitNot => {
                if let Some(Value::Int(i)) = self.stack.last_mut() {
                    *i = !*i;
                    return Ok(());
                }
                let val = self.stack.pop().unwrap();
                let res = eval_unary_value("~", val, false).map_err(|m| self.error(span, m))?;
                self.stack.push(res);
            }
            Opcode::CoerceInt => {
                if let Some(slot) = self.stack.last_mut() {
                    if let Some(i) = slot.as_exact_i64() {
                        *slot = Value::Int(i);
                    } else if let Value::List(items) = slot {
                        for item in items.iter_mut() {
                            if let Some(i) = item.as_exact_i64() {
                                *item = Value::Int(i);
                            }
                        }
                    } else {
                        return Err(
                            self.error(span, "Binding value does not match its type annotation")
                        );
                    }
                }
            }
            Opcode::Not => {
                if let Some(slot) = self.stack.last_mut() {
                    *slot = Value::Bool(!is_truthy(slot));
                }
            }
            Opcode::Equal => {
                let right = self.stack.pop().unwrap();
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("==", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::NotEqual => {
                let right = self.stack.pop().unwrap();
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("!=", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Less => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a < *b);
                            return Ok(());
                        }
                    }
                    (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                        if let Value::Int(a) = *slot {
                            *slot = Value::Bool(a < *b);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::LessEqual => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a <= *b);
                            return Ok(());
                        }
                    }
                    (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                        if let Value::Int(a) = *slot {
                            *slot = Value::Bool(a <= *b);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<=", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::Greater => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a > *b);
                            return Ok(());
                        }
                    }
                    (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                        if let Value::Int(a) = *slot {
                            *slot = Value::Bool(a > *b);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::GreaterEqual => {
                let right = self.stack.pop().unwrap();
                match (self.stack.last_mut(), &right) {
                    (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a >= *b);
                            return Ok(());
                        }
                    }
                    (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                        if let Value::Int(a) = *slot {
                            *slot = Value::Bool(a >= *b);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">=", left, right, span)?;
                self.stack.push(res);
            }
            Opcode::AddImm(imm) => {
                match self.stack.last_mut() {
                    Some(Value::Number(a)) if (*a + imm).is_finite() => {
                        *a += imm;
                        return Ok(());
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_add(imm as i64) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("+", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::SubImm(imm) => {
                match self.stack.last_mut() {
                    Some(Value::Number(a)) if (*a - imm).is_finite() => {
                        *a -= imm;
                        return Ok(());
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_sub(imm as i64) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("-", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::MulImm(imm) => {
                match self.stack.last_mut() {
                    Some(Value::Number(a)) if (*a * imm).is_finite() => {
                        *a *= imm;
                        return Ok(());
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_mul(imm as i64) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("*", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::ModImm(imm) => {
                match self.stack.last_mut() {
                    Some(Value::Number(a)) => {
                        let r = crate::runtime::ops::exact_remainder(*a, imm);
                        if r.is_finite() {
                            *a = r;
                            return Ok(());
                        }
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_rem(imm as i64) {
                            *a = n;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("%", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::EqualImm(imm) => {
                match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a == imm);
                            return Ok(());
                        }
                    }
                    Some(slot @ Value::Int(_)) => {
                        if let Value::Int(a) = *slot {
                            *slot = Value::Bool(crate::runtime::int_f64_eq(a, imm));
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("==", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::LessImm(imm) => {
                match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a < imm);
                            return Ok(());
                        }
                    }
                    Some(slot @ Value::Int(_)) => {
                        if let Value::Int(a) = *slot {
                            let lt = crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_lt());
                            *slot = Value::Bool(lt);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::LessEqualImm(imm) => {
                match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a <= imm);
                            return Ok(());
                        }
                    }
                    Some(slot @ Value::Int(_)) => {
                        if let Value::Int(a) = *slot {
                            let le = crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_le());
                            *slot = Value::Bool(le);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<=", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::GreaterImm(imm) => {
                match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a > imm);
                            return Ok(());
                        }
                    }
                    Some(slot @ Value::Int(_)) => {
                        if let Value::Int(a) = *slot {
                            let gt = crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_gt());
                            *slot = Value::Bool(gt);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::GreaterEqualImm(imm) => {
                match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a >= imm);
                            return Ok(());
                        }
                    }
                    Some(slot @ Value::Int(_)) => {
                        if let Value::Int(a) = *slot {
                            let ge = crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_ge());
                            *slot = Value::Bool(ge);
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">=", left, Value::Number(imm), span)?;
                self.stack.push(res);
            }
            Opcode::LocalAddImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) if (*a + imm).is_finite() => {
                        self.stack.push(Value::Number(*a + imm));
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_add(imm as i64) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("+", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for +")),
                }
            }
            Opcode::LocalSubImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) if (*a - imm).is_finite() => {
                        self.stack.push(Value::Number(*a - imm));
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_sub(imm as i64) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("-", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for -")),
                }
            }
            Opcode::LocalMulImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) if (*a * imm).is_finite() => {
                        self.stack.push(Value::Number(*a * imm));
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_mul(imm as i64) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("*", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for *")),
                }
            }
            Opcode::LocalModImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) => {
                        let rem = crate::runtime::ops::exact_remainder(*a, imm);
                        if rem.is_finite() {
                            self.stack.push(Value::Number(rem));
                        } else {
                            return Err(self.error(span, "Non-finite arithmetic result"));
                        }
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(n) = a.checked_rem(imm as i64) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("%", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for %")),
                }
            }
            Opcode::LocalEqualImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) => self.stack.push(Value::Bool(*a == imm)),
                    Some(Value::Int(a)) => {
                        self.stack
                            .push(Value::Bool(crate::runtime::int_f64_eq(*a, imm)));
                    }
                    Some(other) => {
                        let res =
                            self.eval_binary("==", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for ==")),
                }
            }
            Opcode::LocalLessImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) => self.stack.push(Value::Bool(*a < imm)),
                    Some(Value::Int(a)) => {
                        let lt = crate::runtime::int_f64_cmp(*a, imm).is_some_and(|o| o.is_lt());
                        self.stack.push(Value::Bool(lt));
                    }
                    Some(other) => {
                        let res = self.eval_binary("<", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for <")),
                }
            }
            Opcode::LocalLessEqualImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Number(a)) => self.stack.push(Value::Bool(*a <= imm)),
                    Some(Value::Int(a)) => {
                        let le = crate::runtime::int_f64_cmp(*a, imm).is_some_and(|o| o.is_le());
                        self.stack.push(Value::Bool(le));
                    }
                    Some(other) => {
                        let res =
                            self.eval_binary("<=", other.clone(), Value::Number(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for <=")),
                }
            }
            Opcode::AddIntImm(imm) => {
                if let Some(Value::Int(a)) = self.stack.last_mut()
                    && let Some(n) = a.checked_add(imm)
                {
                    *a = n;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("+", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::SubIntImm(imm) => {
                if let Some(Value::Int(a)) = self.stack.last_mut()
                    && let Some(n) = a.checked_sub(imm)
                {
                    *a = n;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("-", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::MulIntImm(imm) => {
                if let Some(Value::Int(a)) = self.stack.last_mut()
                    && let Some(n) = a.checked_mul(imm)
                {
                    *a = n;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("*", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::ModIntImm(imm) => {
                if let Some(Value::Int(a)) = self.stack.last_mut()
                    && let Some(n) = a.checked_rem(imm)
                {
                    *a = n;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("%", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::BitAndIntImm(imm) => {
                if let Some(Value::Int(a)) = self.stack.last_mut() {
                    *a &= imm;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("&", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::BitXorIntImm(imm) => {
                if let Some(Value::Int(a)) = self.stack.last_mut() {
                    *a ^= imm;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("^", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::ShlIntImm(shift) => {
                if let Some(Value::Int(a)) = self.stack.last_mut() {
                    *a <<= shift;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<<", left, Value::Int(i64::from(shift)), span)?;
                self.stack.push(res);
            }
            Opcode::ShrIntImm(shift) => {
                if let Some(Value::Int(a)) = self.stack.last_mut() {
                    *a >>= shift;
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">>", left, Value::Int(i64::from(shift)), span)?;
                self.stack.push(res);
            }
            Opcode::EqualIntImm(imm) => {
                if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                    && let Value::Int(a) = *slot
                {
                    *slot = Value::Bool(a == imm);
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("==", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::LessIntImm(imm) => {
                if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                    && let Value::Int(a) = *slot
                {
                    *slot = Value::Bool(a < imm);
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::LessEqualIntImm(imm) => {
                if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                    && let Value::Int(a) = *slot
                {
                    *slot = Value::Bool(a <= imm);
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary("<=", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::GreaterIntImm(imm) => {
                if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                    && let Value::Int(a) = *slot
                {
                    *slot = Value::Bool(a > imm);
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::GreaterEqualIntImm(imm) => {
                if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                    && let Value::Int(a) = *slot
                {
                    *slot = Value::Bool(a >= imm);
                    return Ok(());
                }
                let left = self.stack.pop().unwrap();
                let res = self.eval_binary(">=", left, Value::Int(imm), span)?;
                self.stack.push(res);
            }
            Opcode::LocalAddIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => {
                        if let Some(n) = a.checked_add(imm) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("+", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for +")),
                }
            }
            Opcode::LocalSubIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => {
                        if let Some(n) = a.checked_sub(imm) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("-", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for -")),
                }
            }
            Opcode::LocalMulIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => {
                        if let Some(n) = a.checked_mul(imm) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("*", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for *")),
                }
            }
            Opcode::LocalModIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => {
                        if let Some(n) = a.checked_rem(imm) {
                            self.stack.push(Value::Int(n));
                        } else {
                            return Err(self.error(span, "Integer overflow"));
                        }
                    }
                    Some(other) => {
                        let res = self.eval_binary("%", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for %")),
                }
            }
            Opcode::LocalBitAndIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => self.stack.push(Value::Int(*a & imm)),
                    Some(other) => {
                        let res = self.eval_binary("&", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for &")),
                }
            }
            Opcode::LocalEqualIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => self.stack.push(Value::Bool(*a == imm)),
                    Some(other) => {
                        let res = self.eval_binary("==", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for ==")),
                }
            }
            Opcode::LocalLessIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => self.stack.push(Value::Bool(*a < imm)),
                    Some(other) => {
                        let res = self.eval_binary("<", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for <")),
                }
            }
            Opcode::LocalLessEqualIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get(idx) {
                    Some(Value::Int(a)) => self.stack.push(Value::Bool(*a <= imm)),
                    Some(other) => {
                        let res = self.eval_binary("<=", other.clone(), Value::Int(imm), span)?;
                        self.stack.push(res);
                    }
                    None => return Err(self.error(span, "Invalid operands for <=")),
                }
            }
            Opcode::AddAssignLocal(slot) => {
                let right = self.stack.pop().unwrap();
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match (self.stack.get_mut(idx), &right) {
                    (Some(Value::Number(a)), Value::Number(b)) if (*a + *b).is_finite() => {
                        *a += *b;
                        let updated = *a;
                        self.stack.push(Value::Number(updated));
                        return Ok(());
                    }
                    (Some(Value::Int(a)), Value::Int(b)) => {
                        if let Some(updated) = a.checked_add(*b) {
                            *a = updated;
                            self.stack.push(Value::Int(updated));
                            return Ok(());
                        }
                    }
                    (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                        let mut next = *a;
                        for (x, y) in next.iter_mut().zip(b.as_slice()) {
                            *x += *y;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *a = next;
                            self.stack.push(Value::Vector(next));
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                let res = self.eval_binary("+", left, right, span)?;
                self.stack[idx] = res.clone();
                self.stack.push(res);
            }
            Opcode::StoreAddLocal(slot) => {
                let right = self.stack.pop().unwrap();
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match (self.stack.get_mut(idx), &right) {
                    (Some(Value::Number(a)), Value::Number(b)) if (*a + *b).is_finite() => {
                        *a += *b;
                        return Ok(());
                    }
                    (Some(Value::Int(a)), Value::Int(b)) => {
                        if let Some(updated) = a.checked_add(*b) {
                            *a = updated;
                            return Ok(());
                        }
                    }
                    (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                        let mut next = *a;
                        for (x, y) in next.iter_mut().zip(b.as_slice()) {
                            *x += *y;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *a = next;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                let res = self.eval_binary("+", left, right, span)?;
                self.stack[idx] = res;
            }
            Opcode::AddAssignLocalImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get_mut(idx) {
                    Some(Value::Number(a)) if (*a + imm).is_finite() => {
                        *a += imm;
                        let updated = *a;
                        self.stack.push(Value::Number(updated));
                        return Ok(());
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(updated) = a.checked_add(imm as i64) {
                            *a = updated;
                            self.stack.push(Value::Int(updated));
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                let res = self.eval_binary("+", left, Value::Number(imm), span)?;
                self.stack[idx] = res.clone();
                self.stack.push(res);
            }
            Opcode::StoreAddLocalImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                match self.stack.get_mut(idx) {
                    Some(Value::Number(a)) if (*a + imm).is_finite() => {
                        *a += imm;
                        return Ok(());
                    }
                    Some(Value::Int(a)) if is_safe_int_f64(imm) => {
                        if let Some(updated) = a.checked_add(imm as i64) {
                            *a = updated;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                let res = self.eval_binary("+", left, Value::Number(imm), span)?;
                self.stack[idx] = res;
            }
            Opcode::AddAssignLocalIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                if let Some(Value::Int(a)) = self.stack.get_mut(idx)
                    && let Some(updated) = a.checked_add(imm)
                {
                    *a = updated;
                    self.stack.push(Value::Int(updated));
                    return Ok(());
                }
                let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                let res = self.eval_binary("+", left, Value::Int(imm), span)?;
                self.stack[idx] = res.clone();
                self.stack.push(res);
            }
            Opcode::StoreAddLocalIntImm(slot, imm) => {
                let idx = self.frames[frame_idx].stack_base + slot as usize;
                if let Some(Value::Int(a)) = self.stack.get_mut(idx)
                    && let Some(updated) = a.checked_add(imm)
                {
                    *a = updated;
                    return Ok(());
                }
                let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                let res = self.eval_binary("+", left, Value::Int(imm), span)?;
                self.stack[idx] = res;
            }
            _ => unreachable!("non-numeric opcode routed to exec_numeric_op"),
        }
        Ok(())
    }
}
