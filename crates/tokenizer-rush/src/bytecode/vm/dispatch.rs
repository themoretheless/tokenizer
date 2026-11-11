//! Core bytecode instruction dispatch loop.
use super::builtins::is_higher_order_builtin;
use super::{CallFrame, Vm};
use crate::bytecode::iterator::BytecodeIterator;
use crate::bytecode::opcode::{BytecodeClosure, Opcode, PathTarget, Upvalue};
use crate::runtime::builtins::{eval_pure_builtin, lookup_builtin};
use crate::runtime::ops::{format_value_for_display, is_truthy};
use crate::{RecordMap, RuntimeError, UserData, Value};
use std::cell::RefCell;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

impl<'c> Vm<'c> {
    pub(super) fn execute_to_depth(
        &mut self,
        target_depth: usize,
    ) -> Result<Value<'static>, RuntimeError> {
        while self.frames.len() > target_depth {
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

            if (self.fuel & 0x3FF) == 0
                && let Some(cancel) = self.cancel
                && cancel.is_cancelled()
            {
                let span = self.frames[frame_idx]
                    .closure
                    .function
                    .chunk
                    .spans
                    .get(ip)
                    .copied()
                    .unwrap_or(Span::new(0, 0));
                return Err(self.error(span, "Execution cancelled"));
            }

            if ip >= self.frames[frame_idx].closure.function.chunk.code.len() {
                let mut ret_val = self.stack.pop().unwrap_or(Value::Null);
                let exiting = self.frames.pop().unwrap();
                let coerce_int = exiting.coerce_int;
                self.unwind_frame(exiting);
                if coerce_int {
                    self.coerce_int_in_place(&mut ret_val, Span::new(0, 0))?;
                }
                if self.frames.len() == target_depth {
                    return Ok(ret_val);
                }
                self.stack.push(ret_val);
                continue;
            }

            let op = self.frames[frame_idx].closure.function.chunk.code[ip];
            let span = self.frames[frame_idx].closure.function.chunk.spans[ip];
            self.frames[frame_idx].ip = ip + 1;

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
                Opcode::StoreLocal(slot) => {
                    let val = self.stack.pop().unwrap_or(Value::Null);
                    let stack_base = self.frames[frame_idx].stack_base;
                    let idx = stack_base + slot as usize;
                    if idx >= self.stack.len() {
                        self.stack.resize(idx + 1, Value::Null);
                    }
                    self.stack[idx] = val;
                }
                Opcode::GetBuiltin(builtin) => {
                    self.stack.push(Value::Builtin(builtin));
                }
                Opcode::CallBuiltin(builtin, arg_count) => {
                    let arg_start = self.stack.len() - arg_count;
                    if !is_higher_order_builtin(builtin) {
                        let res = eval_pure_builtin(builtin, &self.stack[arg_start..])
                            .map_err(|msg| self.error(span, msg))?;
                        self.stack.truncate(arg_start);
                        self.stack.push(res);
                    } else {
                        let args: Vec<_> = self.stack.drain(arg_start..).collect();
                        let res = self.call_builtin(builtin, &args, span)?;
                        self.stack.push(res);
                    }
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
                Opcode::StoreUpvalue(idx) => {
                    let val = self.stack.pop().unwrap_or(Value::Null);
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
                            Self::mutate_index(&mut self.stack[slot], &idx, val.clone())
                        }
                        Upvalue::Closed(c) => Self::mutate_index(c, &idx, val.clone()),
                    };
                    drop(borrow);
                    if let Some(msg) = err {
                        return Err(self.error(span, msg));
                    }
                    self.stack.push(val);
                }
                Opcode::MemberSetUpvalue(upval_idx, field_idx) => {
                    let val = self.stack.pop().unwrap();
                    let cell = self.frames[frame_idx].closure.upvalues[upval_idx as usize].clone();
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    let mut borrow = cell.borrow_mut();
                    let err = match &mut *borrow {
                        Upvalue::Open(s) => {
                            let slot = *s;
                            Self::mutate_member(
                                &mut self.stack[slot],
                                field_name,
                                val.clone(),
                                Some(ic_cell),
                            )
                        }
                        Upvalue::Closed(c) => {
                            Self::mutate_member(c, field_name, val.clone(), Some(ic_cell))
                        }
                    };
                    drop(borrow);
                    if let Some(msg) = err {
                        return Err(self.error(span, msg));
                    }
                    self.stack.push(val);
                }
                Opcode::GetPath {
                    target,
                    depth,
                    member_mask,
                } => {
                    let depth = depth as usize;
                    let keys_start = self.stack.len() - depth;
                    let val = match target {
                        PathTarget::Local(slot) => {
                            let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                            Self::read_path_vm(
                                &self.stack[target_slot],
                                &self.stack[keys_start..],
                                member_mask,
                                0,
                            )
                        }
                        PathTarget::Upvalue(upval_idx) => {
                            let cell =
                                self.frames[frame_idx].closure.upvalues[upval_idx as usize].clone();
                            let borrow = cell.borrow();
                            match &*borrow {
                                Upvalue::Open(s) => Self::read_path_vm(
                                    &self.stack[*s],
                                    &self.stack[keys_start..],
                                    member_mask,
                                    0,
                                ),
                                Upvalue::Closed(c) => {
                                    Self::read_path_vm(c, &self.stack[keys_start..], member_mask, 0)
                                }
                            }
                        }
                        PathTarget::Global(name_idx) => {
                            let name = Self::const_str(
                                &self.frames[frame_idx].closure.function.chunk,
                                name_idx,
                            );
                            let Some(root) = self.globals.get(name) else {
                                return Err(self.error(span, format!("Unknown name: {name}")));
                            };
                            Self::read_path_vm(root, &self.stack[keys_start..], member_mask, 0)
                        }
                    };
                    match val {
                        Ok(v) => self.stack.push(v),
                        Err(msg) => return Err(self.error(span, msg)),
                    }
                }
                Opcode::SetPath {
                    target,
                    depth,
                    member_mask,
                } => {
                    let val = self.stack.pop().unwrap();
                    let depth = depth as usize;
                    let keys_start = self.stack.len() - depth;
                    let keys: Vec<Value<'static>> = self.stack.drain(keys_start..).collect();
                    let err = match target {
                        PathTarget::Local(slot) => {
                            let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                            Self::mutate_path_vm(
                                &mut self.stack[target_slot],
                                &keys,
                                member_mask,
                                0,
                                val.clone(),
                            )
                        }
                        PathTarget::Upvalue(upval_idx) => {
                            let cell =
                                self.frames[frame_idx].closure.upvalues[upval_idx as usize].clone();
                            let mut borrow = cell.borrow_mut();
                            match &mut *borrow {
                                Upvalue::Open(s) => {
                                    let slot = *s;
                                    Self::mutate_path_vm(
                                        &mut self.stack[slot],
                                        &keys,
                                        member_mask,
                                        0,
                                        val.clone(),
                                    )
                                }
                                Upvalue::Closed(c) => {
                                    Self::mutate_path_vm(c, &keys, member_mask, 0, val.clone())
                                }
                            }
                        }
                        PathTarget::Global(name_idx) => {
                            let name = Self::const_str(
                                &self.frames[frame_idx].closure.function.chunk,
                                name_idx,
                            )
                            .to_string();
                            let Some(root) = self.globals.get_mut(&name) else {
                                return Err(self.error(span, format!("Unknown name: {name}")));
                            };
                            Self::mutate_path_vm(root, &keys, member_mask, 0, val.clone())
                        }
                    };
                    if let Some(msg) = err {
                        return Err(self.error(span, msg));
                    }
                    self.stack.push(val);
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
                    let name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, name_idx);
                    if let Some(val) = self.globals.get(name) {
                        self.stack.push(val.clone());
                    } else if let Some(builtin) = lookup_builtin(name) {
                        self.stack.push(Value::Builtin(builtin));
                    } else {
                        let err_msg = format!("Undefined variable '{name}'");
                        return Err(self.error(span, err_msg));
                    }
                }
                Opcode::SetGlobal(name_idx) => {
                    let name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, name_idx)
                            .to_string();
                    let val = self.stack.last().cloned().unwrap_or(Value::Null);
                    self.globals.insert(name, val);
                }
                Opcode::Add
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::Div
                | Opcode::Mod
                | Opcode::Pow
                | Opcode::BitAnd
                | Opcode::BitXor
                | Opcode::Shl
                | Opcode::Shr
                | Opcode::Neg
                | Opcode::Pos
                | Opcode::BitNot
                | Opcode::CoerceInt
                | Opcode::Not
                | Opcode::Equal
                | Opcode::NotEqual
                | Opcode::Less
                | Opcode::LessEqual
                | Opcode::Greater
                | Opcode::GreaterEqual
                | Opcode::AddImm(_)
                | Opcode::SubImm(_)
                | Opcode::MulImm(_)
                | Opcode::ModImm(_)
                | Opcode::EqualImm(_)
                | Opcode::LessImm(_)
                | Opcode::LessEqualImm(_)
                | Opcode::GreaterImm(_)
                | Opcode::GreaterEqualImm(_)
                | Opcode::LocalAddImm(_, _)
                | Opcode::LocalSubImm(_, _)
                | Opcode::LocalMulImm(_, _)
                | Opcode::LocalModImm(_, _)
                | Opcode::LocalEqualImm(_, _)
                | Opcode::LocalLessImm(_, _)
                | Opcode::LocalLessEqualImm(_, _)
                | Opcode::AddIntImm(_)
                | Opcode::SubIntImm(_)
                | Opcode::MulIntImm(_)
                | Opcode::ModIntImm(_)
                | Opcode::BitAndIntImm(_)
                | Opcode::BitXorIntImm(_)
                | Opcode::ShlIntImm(_)
                | Opcode::ShrIntImm(_)
                | Opcode::EqualIntImm(_)
                | Opcode::LessIntImm(_)
                | Opcode::LessEqualIntImm(_)
                | Opcode::GreaterIntImm(_)
                | Opcode::GreaterEqualIntImm(_)
                | Opcode::LocalAddIntImm(_, _)
                | Opcode::LocalSubIntImm(_, _)
                | Opcode::LocalMulIntImm(_, _)
                | Opcode::LocalModIntImm(_, _)
                | Opcode::LocalBitAndIntImm(_, _)
                | Opcode::LocalEqualIntImm(_, _)
                | Opcode::LocalLessIntImm(_, _)
                | Opcode::LocalLessEqualIntImm(_, _)
                | Opcode::AddAssignLocal(_)
                | Opcode::StoreAddLocal(_)
                | Opcode::AddAssignLocalImm(_, _)
                | Opcode::StoreAddLocalImm(_, _)
                | Opcode::AddAssignLocalIntImm(_, _)
                | Opcode::StoreAddLocalIntImm(_, _) => {
                    self.exec_numeric_op(op, frame_idx, span)?;
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
                    let value = Value::List(items);
                    self.charge_produced(&value, span)?;
                    self.stack.push(value);
                }
                Opcode::BuildTuple(count) => {
                    let start = self.stack.len() - count;
                    let items = self.stack.drain(start..).collect();
                    let value = Value::Tuple(items);
                    self.charge_produced(&value, span)?;
                    self.stack.push(value);
                }
                Opcode::BuildRecord(count) => {
                    let start = self.stack.len() - (count * 2);
                    let mut pairs = Vec::with_capacity(count);
                    let mut iter = self.stack.drain(start..);
                    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                        let Value::String(key) = k else {
                            drop(iter);
                            return Err(self.error(span, "Record key must be a string"));
                        };
                        pairs.push((key, v));
                    }
                    drop(iter);
                    let value = Value::Record(RecordMap::from_iter(pairs));
                    self.charge_produced(&value, span)?;
                    self.stack.push(value);
                }
                Opcode::BuildRecordShape(shape_idx) => {
                    let desc = &self.frames[frame_idx].closure.function.chunk.record_shapes
                        [shape_idx as usize];
                    let len = desc.shape.len();
                    let shape = desc.shape.clone();
                    let start = self.stack.len() - len;
                    let values = if let Some(perm) = &desc.source_to_slot {
                        let mut sorted_vals = vec![Value::Null; len];
                        for (src_i, val) in self.stack.drain(start..).enumerate() {
                            sorted_vals[perm[src_i]] = val;
                        }
                        sorted_vals
                    } else {
                        self.stack.split_off(start)
                    };
                    let value = Value::Record(RecordMap::from_shape_and_values(shape, values));
                    self.charge_produced(&value, span)?;
                    self.stack.push(value);
                }
                Opcode::IndexGet => {
                    let idx = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap();
                    let val = Self::read_index(&obj, &idx).map_err(|msg| self.error(span, msg))?;
                    self.stack.push(val);
                }
                Opcode::LocalIndexGet(slot) => {
                    let idx = self.stack.pop().unwrap();
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    let val = Self::read_index(&self.stack[target_slot], &idx)
                        .map_err(|msg| self.error(span, msg))?;
                    self.stack.push(val);
                }
                Opcode::UpvalueIndexGet(upval_idx) => {
                    let idx = self.stack.pop().unwrap();
                    let upval =
                        self.frames[frame_idx].closure.upvalues[upval_idx as usize].borrow();
                    let val_res = match &*upval {
                        Upvalue::Open(slot_idx) => Self::read_index(&self.stack[*slot_idx], &idx),
                        Upvalue::Closed(container) => Self::read_index(container, &idx),
                    };
                    drop(upval);
                    let val = val_res.map_err(|msg| self.error(span, msg))?;
                    self.stack.push(val);
                }
                Opcode::IndexSet(slot) => {
                    let val = self.stack.pop().unwrap();
                    let idx = self.stack.pop().unwrap();
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(msg) =
                        Self::mutate_index(&mut self.stack[target_slot], &idx, val.clone())
                    {
                        return Err(self.error(span, msg));
                    }
                    self.stack.push(val);
                }
                Opcode::StoreIndexSet(slot) => {
                    let val = self.stack.pop().unwrap();
                    let idx = self.stack.pop().unwrap();
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(msg) = Self::mutate_index(&mut self.stack[target_slot], &idx, val) {
                        return Err(self.error(span, msg));
                    }
                }
                Opcode::MemberGet(field_idx) => {
                    let obj = self.stack.pop().unwrap();
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    let val = Self::read_member(&obj, field_name, ic_cell)
                        .map_err(|msg| self.error(span, msg))?;
                    self.stack.push(val);
                }
                Opcode::LocalMemberGet(slot, field_idx) => {
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    let val = Self::read_member(&self.stack[target_slot], field_name, ic_cell)
                        .map_err(|msg| self.error(span, msg))?;
                    self.stack.push(val);
                }
                Opcode::UpvalueMemberGet(upval_idx, field_idx) => {
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    let upval =
                        self.frames[frame_idx].closure.upvalues[upval_idx as usize].borrow();
                    let val_res = match &*upval {
                        Upvalue::Open(slot_idx) => {
                            Self::read_member(&self.stack[*slot_idx], field_name, ic_cell)
                        }
                        Upvalue::Closed(container) => {
                            Self::read_member(container, field_name, ic_cell)
                        }
                    };
                    drop(upval);
                    let val = val_res.map_err(|msg| self.error(span, msg))?;
                    self.stack.push(val);
                }
                Opcode::MemberSet(slot, field_idx) => {
                    let val = self.stack.pop().unwrap();
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    if let Some(msg) = Self::mutate_member(
                        &mut self.stack[target_slot],
                        field_name,
                        val.clone(),
                        Some(ic_cell),
                    ) {
                        return Err(self.error(span, msg));
                    }
                    self.stack.push(val);
                }
                Opcode::StoreMemberSet(slot, field_idx) => {
                    let val = self.stack.pop().unwrap();
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    if let Some(msg) = Self::mutate_member(
                        &mut self.stack[target_slot],
                        field_name,
                        val,
                        Some(ic_cell),
                    ) {
                        return Err(self.error(span, msg));
                    }
                }
                Opcode::Interpolate(count) => {
                    let start = self.stack.len() - count;
                    let mut out = String::new();
                    for part in &self.stack[start..] {
                        format_value_for_display(part, &mut out, 0);
                    }
                    self.stack.truncate(start);
                    let value = Value::String(out);
                    self.charge_produced(&value, span)?;
                    self.stack.push(value);
                }
                Opcode::Call(arg_count) => {
                    let callee_idx = self.stack.len() - arg_count - 1;
                    let callee = self.stack[callee_idx].clone();
                    match callee {
                        Value::Builtin(b) => {
                            let arg_start = callee_idx + 1;
                            if !is_higher_order_builtin(b) {
                                let res = eval_pure_builtin(b, &self.stack[arg_start..])
                                    .map_err(|msg| self.error(span, msg))?;
                                self.stack.truncate(callee_idx);
                                self.stack.push(res);
                            } else {
                                let args: Vec<_> = self.stack.drain(arg_start..).collect();
                                self.stack.pop();
                                let res = self.call_builtin(b, &args, span)?;
                                self.stack.push(res);
                            }
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
                            if self.frames.len() >= self.max_depth {
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
                                coerce_int: false,
                            });
                        }
                        _ => return Err(self.error(span, "Value is not callable")),
                    }
                }
                Opcode::TailCall(arg_count) => {
                    let callee_idx = self.stack.len() - arg_count - 1;
                    let callee = self.stack[callee_idx].clone();
                    match callee {
                        Value::Builtin(b) => {
                            let arg_start = callee_idx + 1;
                            if !is_higher_order_builtin(b) {
                                let res = eval_pure_builtin(b, &self.stack[arg_start..])
                                    .map_err(|msg| self.error(span, msg))?;
                                self.stack.truncate(callee_idx);
                                self.stack.push(res);
                            } else {
                                let args: Vec<_> = self.stack.drain(arg_start..).collect();
                                self.stack.pop();
                                let res = self.call_builtin(b, &args, span)?;
                                self.stack.push(res);
                            }
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
                            let stack_base = self.frames[frame_idx].stack_base;
                            if !self.open_upvalues.is_empty() {
                                self.close_upvalues(stack_base);
                            }
                            let arg_start = callee_idx + 1;
                            for i in 0..arg_count {
                                self.stack.swap(stack_base + i, arg_start + i);
                            }
                            let needed = stack_base + target_closure.function.chunk.local_count;
                            self.stack.truncate(stack_base + arg_count);
                            if needed > stack_base + arg_count {
                                self.stack.resize(needed, Value::Null);
                            }
                            if self.frames[frame_idx]
                                .closure
                                .function
                                .chunk
                                .code
                                .get(ip + 1)
                                == Some(&Opcode::CoerceInt)
                            {
                                self.frames[frame_idx].coerce_int = true;
                            }
                            self.frames[frame_idx].closure = target_closure;
                            self.frames[frame_idx].ip = 0;
                        }
                        _ => return Err(self.error(span, "Value is not callable")),
                    }
                }
                Opcode::CheckTuple(expected_len) => {
                    let val = self.stack.pop().unwrap();
                    let matches = matches!(val, Value::Tuple(items) if items.len() == expected_len);
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
                    let val = self.stack.pop().unwrap();
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    let hint = ic_cell.get();
                    let contains = match &val {
                        Value::Record(fields) => {
                            if let Some(slot) = fields.contains_key_with_hint(field_name, hint) {
                                if slot != hint {
                                    ic_cell.set(slot);
                                }
                                true
                            } else {
                                false
                            }
                        }
                        Value::UserData(data) if data.variant.is_none() => {
                            if let Some(Value::Record(fields)) = data.values.first() {
                                if let Some(slot) = fields.contains_key_with_hint(field_name, hint)
                                {
                                    if slot != hint {
                                        ic_cell.set(slot);
                                    }
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };
                    self.stack.push(Value::Bool(contains));
                }
                Opcode::RecordGetAssert(field_idx) => {
                    let val = self.stack.pop().unwrap();
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let field_name = Self::const_str(chunk, field_idx);
                    let ic_cell = &chunk.ic_slots[ip];
                    let hint = ic_cell.get();
                    let found = match &val {
                        Value::Record(fields) => {
                            fields.get_with_hint(field_name, hint).map(|(slot, v)| {
                                if slot != hint {
                                    ic_cell.set(slot);
                                }
                                v.clone()
                            })
                        }
                        Value::UserData(data) if data.variant.is_none() => {
                            if let Some(Value::Record(fields)) = data.values.first() {
                                fields.get_with_hint(field_name, hint).map(|(slot, v)| {
                                    if slot != hint {
                                        ic_cell.set(slot);
                                    }
                                    v.clone()
                                })
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
                    let val = self.stack.pop().unwrap();
                    let tag =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, tag_idx);
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
                    let chunk = &self.frames[frame_idx].closure.function.chunk;
                    let type_name = Self::const_str(chunk, type_name_idx).to_string();
                    let variant = variant_idx.map(|idx| Self::const_str(chunk, idx).to_string());
                    let start = self.stack.len() - arity;
                    let values = self.stack.drain(start..).collect();
                    self.stack.push(Value::UserData(Box::new(UserData {
                        type_name,
                        variant,
                        values,
                    })));
                }
                Opcode::IterInit(slot) => {
                    let val = self.stack.pop().unwrap();
                    let iter = BytecodeIterator::new(&val, span)?;
                    let local_idx = self.frames[frame_idx].stack_base + slot as usize;
                    if local_idx >= self.stack.len() {
                        self.stack.resize(local_idx + 1, Value::Null);
                    }
                    self.stack[local_idx] = Value::BytecodeIterator(Rc::new(RefCell::new(iter)));
                }
                Opcode::AssertRegionBudget => {
                    let val = self.stack.pop().unwrap();
                    if val.as_usize_index().is_none() {
                        return Err(
                            self.error(span, "Region budget must be a non-negative integer")
                        );
                    }
                }
                Opcode::IterNext {
                    iter_slot,
                    exit_jump,
                } => {
                    let local_idx = self.frames[frame_idx].stack_base + iter_slot as usize;
                    let iter_val = self.stack[local_idx].clone();
                    match iter_val {
                        Value::BytecodeIterator(cell) => {
                            let next_item = cell.borrow_mut().next(self, span)?;
                            match next_item {
                                Some(item) => {
                                    self.stack.push(item);
                                }
                                None => {
                                    self.frames[frame_idx].ip = exit_jump;
                                }
                            }
                        }
                        _ => return Err(self.error(span, "Expected iterator in loop")),
                    }
                }
                Opcode::TryPropagate => {
                    let val = self.stack.pop().unwrap();
                    match val {
                        Value::Variant("Some" | "Ok", mut items) if items.len() == 1 => {
                            self.stack.push(items.remove(0));
                        }
                        val @ Value::Variant("None" | "Err", _) => {
                            let exiting = self.frames.pop().unwrap();
                            self.unwind_frame(exiting);
                            if self.frames.len() == target_depth {
                                return Ok(val);
                            }
                            self.stack.push(val);
                        }
                        _ => return Err(self.error(span, "? requires Option or Result")),
                    }
                }
                Opcode::Return => {
                    let mut ret_val = self.stack.pop().unwrap_or(Value::Null);
                    let exiting = self.frames.pop().unwrap();
                    let coerce_int = exiting.coerce_int;
                    self.unwind_frame(exiting);
                    if coerce_int {
                        self.coerce_int_in_place(&mut ret_val, span)?;
                    }
                    if self.frames.len() == target_depth {
                        return Ok(ret_val);
                    }
                    self.stack.push(ret_val);
                }
            }
        }

        Ok(self.stack.pop().unwrap_or(Value::Null))
    }
}
