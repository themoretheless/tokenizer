//! Flat stack-based Bytecode Virtual Machine runtime with superinstructions,
//! zero-allocation builtin dispatch, and ExecutionLimits / CancellationToken support.
use crate::bytecode::iterator::{
    BytecodeIterator, BytecodeSequence, BytecodeSequenceSource, BytecodeSequenceStage,
};
use crate::bytecode::opcode::{BytecodeClosure, BytecodeFunction, Chunk, Opcode, Upvalue};
use crate::runtime::builtins::{eval_pure_builtin, lookup_builtin};
use crate::runtime::ops::{
    eval_binary_values, eval_unary_value, format_value_for_display, is_truthy,
};
use crate::{Builtin, CancellationToken, ExecutionLimits, RuntimeError, UserData, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

struct CallFrame {
    closure: Rc<BytecodeClosure>,
    ip: usize,
    stack_base: usize,
}

/// The flat stack Bytecode Virtual Machine runtime.
pub struct Vm<'c> {
    pub(crate) fuel: usize,
    max_depth: usize,
    cancel: Option<&'c CancellationToken>,
    globals: HashMap<String, Value<'static>>,
    frames: Vec<CallFrame>,
    stack: Vec<Value<'static>>,
    open_upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl<'c> Vm<'c> {
    pub fn new(budget: usize) -> Self {
        Self {
            fuel: budget,
            max_depth: 512,
            cancel: None,
            globals: HashMap::new(),
            frames: Vec::with_capacity(16),
            stack: Vec::with_capacity(64),
            open_upvalues: Vec::new(),
        }
    }

    pub fn with_limits(limits: ExecutionLimits, cancel: Option<&'c CancellationToken>) -> Self {
        Self {
            fuel: limits.steps,
            max_depth: limits.max_depth.max(1),
            cancel,
            globals: HashMap::new(),
            frames: Vec::with_capacity(16),
            stack: Vec::with_capacity(64),
            open_upvalues: Vec::new(),
        }
    }

    pub fn fuel(&self) -> usize {
        self.fuel
    }

    #[inline(always)]
    pub(crate) fn error(&self, span: Span, message: impl Into<String>) -> RuntimeError {
        RuntimeError {
            module: None,
            span,
            message: message.into(),
            stack: Vec::new(),
            location: None,
        }
    }

    #[inline(always)]
    fn const_str(chunk: &Chunk, idx: u32) -> &str {
        match &chunk.constants[idx as usize] {
            Value::String(s) => s.as_str(),
            _ => unreachable!("Expected string constant"),
        }
    }

    #[inline]
    fn unwind_frame(&mut self, exiting: CallFrame) {
        if !self.open_upvalues.is_empty() {
            self.close_upvalues(exiting.stack_base);
        }
        if exiting.stack_base > 0 {
            self.stack.truncate(exiting.stack_base - 1);
        } else {
            self.stack.clear();
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

        let target_depth = self.frames.len();
        let callee_slot = self.stack.len();
        self.stack.push(Value::Null);
        let stack_base = self.stack.len();
        self.stack.extend_from_slice(args);
        let local_count = closure.function.chunk.local_count;
        if local_count > args.len() {
            self.stack.resize(stack_base + local_count, Value::Null);
        }
        self.frames.push(CallFrame {
            closure: closure.clone(),
            ip: 0,
            stack_base,
        });

        match self.execute_to_depth(target_depth) {
            Ok(val) => Ok(val),
            Err(e) => {
                self.close_upvalues(stack_base);
                self.frames.truncate(target_depth);
                self.stack.truncate(callee_slot);
                Err(e)
            }
        }
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

    fn execute_to_depth(&mut self, target_depth: usize) -> Result<Value<'static>, RuntimeError> {
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
                let ret_val = self.stack.pop().unwrap_or(Value::Null);
                let exiting = self.frames.pop().unwrap();
                self.unwind_frame(exiting);
                if self.frames.len() == target_depth {
                    return Ok(ret_val);
                }
                self.stack.push(ret_val);
                continue;
            }

            let op = self.frames[frame_idx].closure.function.chunk.code[ip].clone();
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
                    // Fast path for pure builtins without allocating a Vec
                    if !matches!(
                        builtin,
                        Builtin::Map
                            | Builtin::Filter
                            | Builtin::Fold
                            | Builtin::Range
                            | Builtin::RangeIter
                            | Builtin::Iter
                            | Builtin::Collect
                            | Builtin::FlatMap
                            | Builtin::Any
                            | Builtin::All
                            | Builtin::Zip
                            | Builtin::GroupBy
                            | Builtin::FoldBy
                    ) {
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
                            Self::mutate_index(&mut self.stack[slot], &idx, val)
                        }
                        Upvalue::Closed(c) => Self::mutate_index(c, &idx, val),
                    };
                    if let Some(msg) = err {
                        return Err(self.error(span, msg));
                    }
                }
                Opcode::MemberSetUpvalue(upval_idx, field_idx) => {
                    let val = self.stack.pop().unwrap();
                    let field_name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, field_idx)
                            .to_string();
                    let cell = self.frames[frame_idx].closure.upvalues[upval_idx as usize].clone();
                    let mut borrow = cell.borrow_mut();
                    let err = match &mut *borrow {
                        Upvalue::Open(s) => {
                            let slot = *s;
                            Self::mutate_member(&mut self.stack[slot], field_name, val)
                        }
                        Upvalue::Closed(c) => Self::mutate_member(c, field_name, val),
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
                Opcode::Add => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    if let (Value::Number(a), Value::Number(b)) = (&left, &right) {
                        let n = a + b;
                        if n.is_finite() {
                            self.stack.push(Value::Number(n));
                            continue;
                        }
                    }
                    let res =
                        eval_binary_values("+", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Sub => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    if let (Value::Number(a), Value::Number(b)) = (&left, &right) {
                        let n = a - b;
                        if n.is_finite() {
                            self.stack.push(Value::Number(n));
                            continue;
                        }
                    }
                    let res =
                        eval_binary_values("-", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Mul => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    if let (Value::Number(a), Value::Number(b)) = (&left, &right) {
                        let n = a * b;
                        if n.is_finite() {
                            self.stack.push(Value::Number(n));
                            continue;
                        }
                    }
                    let res =
                        eval_binary_values("*", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Div => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("/", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Mod => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("%", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Pow => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("**", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Neg => {
                    let val = self.stack.pop().unwrap();
                    let res = eval_unary_value("-", val, false).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Pos => {
                    let val = self.stack.pop().unwrap();
                    let res = eval_unary_value("+", val, false).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Not => {
                    let val = self.stack.pop().unwrap();
                    self.stack.push(Value::Bool(!is_truthy(&val)));
                }
                Opcode::Equal => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("==", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::NotEqual => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("!=", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
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
                Opcode::AddImm(imm) => match self.stack.last_mut() {
                    Some(Value::Number(a)) => *a += imm,
                    _ => return Err(self.error(span, "Invalid operands for +")),
                },
                Opcode::SubImm(imm) => match self.stack.last_mut() {
                    Some(Value::Number(a)) => *a -= imm,
                    _ => return Err(self.error(span, "Invalid operands for -")),
                },
                Opcode::MulImm(imm) => {
                    let left = self.stack.pop().unwrap();
                    match left {
                        Value::Number(a) => self.stack.push(Value::Number(a * imm)),
                        other => {
                            let res = eval_binary_values("*", other, Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
                            self.stack.push(res);
                        }
                    }
                }
                Opcode::LessImm(imm) => match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a < imm);
                        }
                    }
                    _ => return Err(self.error(span, "Invalid operands for <")),
                },
                Opcode::LessEqualImm(imm) => match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a <= imm);
                        }
                    }
                    _ => return Err(self.error(span, "Invalid operands for <=")),
                },
                Opcode::GreaterImm(imm) => match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a > imm);
                        }
                    }
                    _ => return Err(self.error(span, "Invalid operands for >")),
                },
                Opcode::GreaterEqualImm(imm) => match self.stack.last_mut() {
                    Some(slot @ Value::Number(_)) => {
                        if let Value::Number(a) = *slot {
                            *slot = Value::Bool(a >= imm);
                        }
                    }
                    _ => return Err(self.error(span, "Invalid operands for >=")),
                },
                Opcode::LocalAddImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get(idx) {
                        Some(Value::Number(a)) => self.stack.push(Value::Number(*a + imm)),
                        _ => return Err(self.error(span, "Invalid operands for +")),
                    }
                }
                Opcode::LocalSubImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get(idx) {
                        Some(Value::Number(a)) => self.stack.push(Value::Number(*a - imm)),
                        _ => return Err(self.error(span, "Invalid operands for -")),
                    }
                }
                Opcode::LocalMulImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get(idx) {
                        Some(Value::Number(a)) => self.stack.push(Value::Number(*a * imm)),
                        Some(other) => {
                            let res = eval_binary_values("*", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
                            self.stack.push(res);
                        }
                        None => return Err(self.error(span, "Invalid operands for *")),
                    }
                }
                Opcode::LocalLessEqualImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get(idx) {
                        Some(Value::Number(a)) => self.stack.push(Value::Bool(*a <= imm)),
                        _ => return Err(self.error(span, "Invalid operands for <=")),
                    }
                }
                Opcode::AddAssignLocal(slot) => {
                    let right = self.stack.pop().unwrap();
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    if let (Some(Value::Number(a)), Value::Number(b)) =
                        (self.stack.get_mut(idx), &right)
                    {
                        *a += *b;
                        let updated = *a;
                        self.stack.push(Value::Number(updated));
                    } else {
                        let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                        let res = eval_binary_values("+", left, right)
                            .map_err(|m| self.error(span, m))?;
                        self.stack[idx] = res.clone();
                        self.stack.push(res);
                    }
                }
                Opcode::StoreAddLocal(slot) => {
                    let right = self.stack.pop().unwrap();
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    if let (Some(Value::Number(a)), Value::Number(b)) =
                        (self.stack.get_mut(idx), &right)
                    {
                        *a += *b;
                    } else {
                        let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                        let res = eval_binary_values("+", left, right)
                            .map_err(|m| self.error(span, m))?;
                        self.stack[idx] = res;
                    }
                }
                Opcode::AddAssignLocalImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(Value::Number(a)) = self.stack.get_mut(idx) {
                        *a += imm;
                        let updated = *a;
                        self.stack.push(Value::Number(updated));
                    } else {
                        return Err(self.error(span, "Invalid operands for +="));
                    }
                }
                Opcode::StoreAddLocalImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(Value::Number(a)) = self.stack.get_mut(idx) {
                        *a += imm;
                    } else {
                        return Err(self.error(span, "Invalid operands for +="));
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
                    let mut fields = BTreeMap::new();
                    let mut iter = self.stack.drain(start..);
                    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                        let Value::String(key) = k else {
                            drop(iter);
                            return Err(self.error(span, "Record key must be a string"));
                        };
                        fields.insert(key, v);
                    }
                    drop(iter);
                    self.stack.push(Value::Record(fields));
                }
                Opcode::IndexGet => {
                    let idx = self.stack.pop().unwrap();
                    let obj = self.stack.pop().unwrap();
                    match (&obj, &idx) {
                        (Value::List(items), Value::Number(n))
                        | (Value::Tuple(items), Value::Number(n)) => {
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
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(msg) = Self::mutate_index(&mut self.stack[target_slot], &idx, val) {
                        return Err(self.error(span, msg));
                    }
                }
                Opcode::MemberGet(field_idx) => {
                    let obj = self.stack.pop().unwrap();
                    let field_name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, field_idx);
                    match &obj {
                        Value::Record(fields) => {
                            if let Some(val) = fields.get(field_name) {
                                self.stack.push(val.clone());
                            } else {
                                let err_msg = format!("Field '{field_name}' not found");
                                return Err(self.error(span, err_msg));
                            }
                        }
                        Value::UserData(data) if data.variant.is_none() => {
                            if let Some(Value::Record(fields)) = data.values.first() {
                                if let Some(val) = fields.get(field_name) {
                                    self.stack.push(val.clone());
                                } else {
                                    let err_msg = format!("Field '{field_name}' not found");
                                    return Err(self.error(span, err_msg));
                                }
                            } else {
                                return Err(self.error(span, "Cannot access member on value"));
                            }
                        }
                        Value::Vector(v) => {
                            let val = match field_name {
                                "x" if !v.is_empty() => v[0],
                                "y" if v.len() > 1 => v[1],
                                "z" if v.len() > 2 => v[2],
                                "w" if v.len() > 3 => v[3],
                                _ => {
                                    let err_msg = format!("Vector field '{field_name}' not found");
                                    return Err(self.error(span, err_msg));
                                }
                            };
                            self.stack.push(Value::Number(val));
                        }
                        _ => return Err(self.error(span, "Cannot access member on value")),
                    }
                }
                Opcode::MemberSet(slot, field_idx) => {
                    let val = self.stack.pop().unwrap();
                    let field_name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, field_idx)
                            .to_string();
                    let target_slot = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(msg) =
                        Self::mutate_member(&mut self.stack[target_slot], field_name, val)
                    {
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
                    self.stack.push(Value::String(out));
                }
                Opcode::Call(arg_count) => {
                    let callee_idx = self.stack.len() - arg_count - 1;
                    let callee = self.stack[callee_idx].clone();
                    match callee {
                        Value::Builtin(b) => {
                            let arg_start = callee_idx + 1;
                            if !matches!(
                                b,
                                Builtin::Map
                                    | Builtin::Filter
                                    | Builtin::Fold
                                    | Builtin::Range
                                    | Builtin::RangeIter
                                    | Builtin::Iter
                                    | Builtin::Collect
                                    | Builtin::FlatMap
                                    | Builtin::Any
                                    | Builtin::All
                                    | Builtin::Zip
                                    | Builtin::GroupBy
                                    | Builtin::FoldBy
                            ) {
                                let res = eval_pure_builtin(b, &self.stack[arg_start..])
                                    .map_err(|msg| self.error(span, msg))?;
                                self.stack.truncate(callee_idx);
                                self.stack.push(res);
                            } else {
                                let args: Vec<_> = self.stack.drain(arg_start..).collect();
                                self.stack.pop(); // pop callee
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
                            });
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
                    let field_name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, field_idx);
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
                    let val = self.stack.pop().unwrap();
                    let field_name =
                        Self::const_str(&self.frames[frame_idx].closure.function.chunk, field_idx);
                    let found = match &val {
                        Value::Record(fields) => fields.get(field_name).cloned(),
                        Value::UserData(data) if data.variant.is_none() => {
                            if let Some(Value::Record(fields)) = data.values.first() {
                                fields.get(field_name).cloned()
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
                    match val {
                        Value::Number(number)
                            if number.is_finite()
                                && number >= 0.0
                                && number.fract() == 0.0
                                && number <= usize::MAX as f64 => {}
                        _ => {
                            return Err(
                                self.error(span, "Region budget must be a non-negative integer")
                            );
                        }
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
                    let ret_val = self.stack.pop().unwrap_or(Value::Null);
                    let exiting = self.frames.pop().unwrap();
                    self.unwind_frame(exiting);
                    if self.frames.len() == target_depth {
                        return Ok(ret_val);
                    }
                    self.stack.push(ret_val);
                }
            }
        }

        Ok(self.stack.pop().unwrap_or(Value::Null))
    }

    #[inline]
    fn mutate_index(
        container: &mut Value<'static>,
        idx: &Value<'static>,
        val: Value<'static>,
    ) -> Option<&'static str> {
        match (container, idx) {
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

    #[inline]
    fn mutate_member(
        container: &mut Value<'static>,
        field_name: String,
        val: Value<'static>,
    ) -> Option<&'static str> {
        match container {
            Value::Record(fields) => {
                fields.insert(field_name, val);
                None
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first_mut() {
                    fields.insert(field_name, val);
                    None
                } else {
                    Some("Cannot mutate member on value")
                }
            }
            _ => Some("Cannot mutate member on value"),
        }
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
            Builtin::Map => {
                if args.len() != 2 {
                    return Err(self.error(span, "map requires 2 arguments"));
                }
                let (source, callback) = (&args[0], &args[1]);
                match source {
                    Value::Range { start, end, step } => {
                        let seq = BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: vec![BytecodeSequenceStage {
                                callback: callback.clone(),
                                filter: false,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            filter: false,
                            span,
                        });
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: seq.source.clone(),
                            stages,
                        })))
                    }
                    Value::List(items) => {
                        let mut output = Vec::with_capacity(items.len());
                        for item in items {
                            let res =
                                self.invoke_callable(callback, std::slice::from_ref(item), span)?;
                            output.push(res);
                        }
                        Ok(Value::List(output))
                    }
                    _ => Err(self.error(span, "map requires a list or sequence and a callback")),
                }
            }
            Builtin::FlatMap => {
                if args.len() != 2 {
                    return Err(self.error(span, "flat_map requires a list and a callback"));
                }
                let (Value::List(items), callback) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "flat_map requires a list and a callback"));
                };
                let mut output = Vec::new();
                for item in items {
                    let Value::List(values) =
                        self.invoke_callable(callback, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "flat_map callback must return a list"));
                    };
                    output.extend(values);
                }
                Ok(Value::List(output))
            }
            Builtin::Filter => {
                if args.len() != 2 {
                    return Err(self.error(span, "filter requires 2 arguments"));
                }
                let (source, callback) = (&args[0], &args[1]);
                match source {
                    Value::Range { start, end, step } => {
                        let seq = BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: vec![BytecodeSequenceStage {
                                callback: callback.clone(),
                                filter: true,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            filter: true,
                            span,
                        });
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: seq.source.clone(),
                            stages,
                        })))
                    }
                    Value::List(items) => {
                        let mut output = Vec::new();
                        for item in items {
                            let res =
                                self.invoke_callable(callback, std::slice::from_ref(item), span)?;
                            if is_truthy(&res) {
                                output.push(item.clone());
                            }
                        }
                        Ok(Value::List(output))
                    }
                    _ => Err(self.error(span, "filter requires a list or sequence and a callback")),
                }
            }
            Builtin::Fold => {
                if args.len() != 3 {
                    return Err(self.error(span, "fold requires 3 arguments"));
                }
                let (source, initial, callback) = (&args[0], &args[1], &args[2]);
                match source {
                    Value::List(items) => {
                        let mut acc = initial.clone();
                        for item in items {
                            acc = self.invoke_callable(callback, &[acc, item.clone()], span)?;
                        }
                        Ok(acc)
                    }
                    Value::Range { .. } | Value::BytecodeSequence(_) => {
                        let mut iter = BytecodeIterator::new(source, span)?;
                        let mut acc = initial.clone();
                        while let Some(item) = iter.next(self, span)? {
                            acc = self.invoke_callable(callback, &[acc, item], span)?;
                        }
                        Ok(acc)
                    }
                    _ => Err(self.error(
                        span,
                        "fold requires a list, an initial value and a callback",
                    )),
                }
            }
            Builtin::Any | Builtin::All => {
                if args.len() != 2 {
                    return Err(self.error(span, "any and all require a list and a callback"));
                }
                let (Value::List(items), callback) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "any and all require a list and a callback"));
                };
                for item in items {
                    let Value::Bool(matched) =
                        self.invoke_callable(callback, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "Predicate callback must return bool"));
                    };
                    if builtin == Builtin::Any && matched {
                        return Ok(Value::Bool(true));
                    }
                    if builtin == Builtin::All && !matched {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(builtin == Builtin::All))
            }
            Builtin::Zip => {
                let [Value::List(left), Value::List(right)] = args else {
                    return Err(self.error(span, "zip requires two lists"));
                };
                let count = left.len().min(right.len());
                let items = left
                    .iter()
                    .take(count)
                    .cloned()
                    .zip(right.iter().take(count).cloned())
                    .map(|(l, r)| Value::Tuple(vec![l, r]))
                    .collect();
                Ok(Value::List(items))
            }
            Builtin::GroupBy => {
                let [Value::List(items), key_fn] = args else {
                    return Err(
                        self.error(span, "group_by requires a list and a key selector callback")
                    );
                };
                let mut groups: BTreeMap<String, Vec<Value<'static>>> = BTreeMap::new();
                for item in items {
                    let Value::String(key) =
                        self.invoke_callable(key_fn, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "group_by key selector must return string"));
                    };
                    groups.entry(key).or_default().push(item.clone());
                }
                Ok(Value::Record(
                    groups
                        .into_iter()
                        .map(|(k, v)| (k, Value::List(v)))
                        .collect(),
                ))
            }
            Builtin::FoldBy => {
                let [Value::List(items), key_fn, initial, fold_fn] = args else {
                    return Err(self.error(
                        span,
                        "fold_by requires a list, key selector, initial value and fold callback",
                    ));
                };
                let mut groups: BTreeMap<String, Value<'static>> = BTreeMap::new();
                for item in items {
                    let Value::String(key) =
                        self.invoke_callable(key_fn, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "fold_by key selector must return string"));
                    };
                    let current = groups.remove(&key).unwrap_or_else(|| initial.clone());
                    let next = self.invoke_callable(fold_fn, &[current, item.clone()], span)?;
                    groups.insert(key, next);
                }
                Ok(Value::Record(groups))
            }
            Builtin::Range | Builtin::RangeIter => {
                let (start, end, step) = match args {
                    [Value::Number(start), Value::Number(end)] => (*start, *end, 1.0),
                    [
                        Value::Number(start),
                        Value::Number(end),
                        Value::Number(step),
                    ] => (*start, *end, *step),
                    _ => return Err(self.error(span, "range expects start, end and optional step")),
                };
                if step == 0.0 {
                    return Err(self.error(span, "Range step must be nonzero"));
                }
                if !start.is_finite() || !end.is_finite() || !step.is_finite() {
                    return Err(self.error(span, "Range arguments must be finite"));
                }
                if builtin == Builtin::RangeIter {
                    return Ok(Value::Range { start, end, step });
                }
                let mut values = Vec::new();
                let mut current = start;
                while if step > 0.0 {
                    current < end
                } else {
                    current > end
                } {
                    if self.fuel == 0 {
                        return Err(self.error(span, "Execution limit exceeded"));
                    }
                    self.fuel -= 1;
                    values.push(Value::Number(current));
                    let next = (values.len() as f64).mul_add(step, start);
                    if !next.is_finite() || next == current {
                        return Err(self.error(span, "Range cannot advance finitely"));
                    }
                    current = next;
                }
                Ok(Value::List(values))
            }
            Builtin::Iter => {
                let Some(source) = args.first() else {
                    return Err(self.error(span, "iter requires one list or sequence"));
                };
                match source {
                    Value::List(items) => Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                        source: BytecodeSequenceSource::List(items.clone()),
                        stages: Vec::new(),
                    }))),
                    Value::Range { start, end, step } => {
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: Vec::new(),
                        })))
                    }
                    Value::BytecodeSequence(seq) => Ok(Value::BytecodeSequence(seq.clone())),
                    _ => Err(self.error(span, "iter requires one list or sequence")),
                }
            }
            Builtin::Collect => {
                if args.len() != 2 {
                    return Err(
                        self.error(span, "collect requires a sequence and maximum item count")
                    );
                }
                let source = &args[0];
                let Value::Number(limit) = &args[1] else {
                    return Err(
                        self.error(span, "collect requires a sequence and maximum item count")
                    );
                };
                if !limit.is_finite()
                    || *limit < 0.0
                    || limit.fract() != 0.0
                    || *limit >= usize::MAX as f64
                {
                    return Err(self.error(
                        span,
                        "Collection limit must be a nonnegative representable integer",
                    ));
                }
                let max_count = *limit as usize;
                let mut iter = BytecodeIterator::new(source, span)?;
                let mut output = Vec::new();
                for _ in 0..max_count {
                    match iter.next(self, span)? {
                        Some(item) => output.push(item),
                        None => break,
                    }
                }
                Ok(Value::List(output))
            }
            other => eval_pure_builtin(other, args).map_err(|msg| self.error(span, msg)),
        }
    }
}
