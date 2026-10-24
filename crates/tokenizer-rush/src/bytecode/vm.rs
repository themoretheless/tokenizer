//! Flat stack-based Bytecode Virtual Machine runtime with superinstructions,
//! zero-allocation builtin dispatch, and ExecutionLimits / CancellationToken support.
use crate::bytecode::iterator::{
    BytecodeIterator, BytecodeSequence, BytecodeSequenceSource, BytecodeSequenceStage,
    BytecodeStageKind,
};
use crate::bytecode::opcode::{
    BytecodeClosure, BytecodeFunction, Chunk, Opcode, PathTarget, Upvalue,
};
use crate::runtime::builtins::{eval_pure_builtin, lookup_builtin};
use crate::runtime::ops::{
    eval_binary_values, eval_unary_value, format_value_for_display, is_truthy,
};
use crate::{
    Builtin, CancellationToken, ExecutionLimits, RecordMap, RuntimeError, UserData, Value,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

struct CallFrame {
    closure: Rc<BytecodeClosure>,
    ip: usize,
    stack_base: usize,
    coerce_int: bool,
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
    fn coerce_int_in_place(
        &self,
        slot: &mut Value<'static>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if let Some(i) = slot.as_exact_i64() {
            *slot = Value::Int(i);
            Ok(())
        } else if let Value::List(items) = slot {
            for item in items.iter_mut() {
                if let Some(i) = item.as_exact_i64() {
                    *item = Value::Int(i);
                }
            }
            Ok(())
        } else {
            Err(self.error(span, "Binding value does not match its type annotation"))
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
            coerce_int: false,
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
                Opcode::Add => {
                    let right = self.stack.pop().unwrap();
                    match (self.stack.last_mut(), &right) {
                        (Some(Value::Number(a)), Value::Number(b)) => {
                            let n = *a + *b;
                            if n.is_finite() {
                                *a = n;
                                continue;
                            }
                        }
                        (Some(Value::Int(a)), Value::Int(b)) => {
                            if let Some(n) = a.checked_add(*b) {
                                *a = n;
                                continue;
                            }
                        }
                        (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                            let mut next = *a;
                            for (x, y) in next.iter_mut().zip(b.as_slice()) {
                                *x += *y;
                            }
                            if next.iter().all(|x| x.is_finite()) {
                                *a = next;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("+", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Sub => {
                    let right = self.stack.pop().unwrap();
                    match (self.stack.last_mut(), &right) {
                        (Some(Value::Number(a)), Value::Number(b)) => {
                            let n = *a - *b;
                            if n.is_finite() {
                                *a = n;
                                continue;
                            }
                        }
                        (Some(Value::Int(a)), Value::Int(b)) => {
                            if let Some(n) = a.checked_sub(*b) {
                                *a = n;
                                continue;
                            }
                        }
                        (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                            let mut next = *a;
                            for (x, y) in next.iter_mut().zip(b.as_slice()) {
                                *x -= *y;
                            }
                            if next.iter().all(|x| x.is_finite()) {
                                *a = next;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("-", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Mul => {
                    let right = self.stack.pop().unwrap();
                    match (self.stack.last_mut(), &right) {
                        (Some(Value::Number(a)), Value::Number(b)) => {
                            let n = *a * *b;
                            if n.is_finite() {
                                *a = n;
                                continue;
                            }
                        }
                        (Some(Value::Int(a)), Value::Int(b)) => {
                            if let Some(n) = a.checked_mul(*b) {
                                *a = n;
                                continue;
                            }
                        }
                        (Some(Value::Vector(a)), Value::Number(b)) => {
                            let mut next = *a;
                            for x in &mut next {
                                *x *= *b;
                            }
                            if next.iter().all(|x| x.is_finite()) {
                                *a = next;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("*", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Div => {
                    let right = self.stack.pop().unwrap();
                    if let (Some(Value::Vector(a)), Value::Number(b)) =
                        (self.stack.last_mut(), &right)
                    {
                        let mut next = *a;
                        for x in &mut next {
                            *x /= *b;
                        }
                        if next.iter().all(|x| x.is_finite()) {
                            *a = next;
                            continue;
                        }
                    }
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
                Opcode::BitAnd => {
                    let right = self.stack.pop().unwrap();
                    if let (Some(Value::Int(a)), Value::Int(b)) = (self.stack.last_mut(), &right) {
                        *a &= *b;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("&", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::BitXor => {
                    let right = self.stack.pop().unwrap();
                    if let (Some(Value::Int(a)), Value::Int(b)) = (self.stack.last_mut(), &right) {
                        *a ^= *b;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("^", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Shl => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("<<", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Shr => {
                    let right = self.stack.pop().unwrap();
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values(">>", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Neg => {
                    match self.stack.last_mut() {
                        Some(Value::Number(n)) if (-*n).is_finite() => {
                            *n = -*n;
                            continue;
                        }
                        Some(Value::Int(i)) => {
                            if let Some(neg) = i.checked_neg() {
                                *i = neg;
                                continue;
                            }
                        }
                        Some(Value::Vector(v)) => {
                            let mut next = *v;
                            for x in &mut next {
                                *x = -*x;
                            }
                            if next.iter().all(|x| x.is_finite()) {
                                *v = next;
                                continue;
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
                        continue;
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
                            return Err(self
                                .error(span, "Binding value does not match its type annotation"));
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
                    match (self.stack.last_mut(), &right) {
                        (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a < *b);
                                continue;
                            }
                        }
                        (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                            if let Value::Int(a) = *slot {
                                *slot = Value::Bool(a < *b);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("<", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::LessEqual => {
                    let right = self.stack.pop().unwrap();
                    match (self.stack.last_mut(), &right) {
                        (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a <= *b);
                                continue;
                            }
                        }
                        (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                            if let Value::Int(a) = *slot {
                                *slot = Value::Bool(a <= *b);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values("<=", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::Greater => {
                    let right = self.stack.pop().unwrap();
                    match (self.stack.last_mut(), &right) {
                        (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a > *b);
                                continue;
                            }
                        }
                        (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                            if let Value::Int(a) = *slot {
                                *slot = Value::Bool(a > *b);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values(">", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::GreaterEqual => {
                    let right = self.stack.pop().unwrap();
                    match (self.stack.last_mut(), &right) {
                        (Some(slot @ Value::Number(_)), Value::Number(b)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a >= *b);
                                continue;
                            }
                        }
                        (Some(slot @ Value::Int(_)), Value::Int(b)) => {
                            if let Value::Int(a) = *slot {
                                *slot = Value::Bool(a >= *b);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res =
                        eval_binary_values(">=", left, right).map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::AddImm(imm) => {
                    match self.stack.last_mut() {
                        Some(Value::Number(a)) if (*a + imm).is_finite() => {
                            *a += imm;
                            continue;
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_add(imm as i64) {
                                *a = n;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("+", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::SubImm(imm) => {
                    match self.stack.last_mut() {
                        Some(Value::Number(a)) if (*a - imm).is_finite() => {
                            *a -= imm;
                            continue;
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_sub(imm as i64) {
                                *a = n;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("-", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::MulImm(imm) => {
                    match self.stack.last_mut() {
                        Some(Value::Number(a)) if (*a * imm).is_finite() => {
                            *a *= imm;
                            continue;
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_mul(imm as i64) {
                                *a = n;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("*", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::ModImm(imm) => {
                    match self.stack.last_mut() {
                        Some(Value::Number(a)) => {
                            let r = crate::runtime::ops::exact_remainder(*a, imm);
                            if r.is_finite() {
                                *a = r;
                                continue;
                            }
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_rem(imm as i64) {
                                *a = n;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("%", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::EqualImm(imm) => {
                    match self.stack.last_mut() {
                        Some(slot @ Value::Number(_)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a == imm);
                                continue;
                            }
                        }
                        Some(slot @ Value::Int(_)) => {
                            if let Value::Int(a) = *slot {
                                *slot = Value::Bool(crate::runtime::int_f64_eq(a, imm));
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("==", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::LessImm(imm) => {
                    match self.stack.last_mut() {
                        Some(slot @ Value::Number(_)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a < imm);
                                continue;
                            }
                        }
                        Some(slot @ Value::Int(_)) => {
                            if let Value::Int(a) = *slot {
                                let lt =
                                    crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_lt());
                                *slot = Value::Bool(lt);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("<", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::LessEqualImm(imm) => {
                    match self.stack.last_mut() {
                        Some(slot @ Value::Number(_)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a <= imm);
                                continue;
                            }
                        }
                        Some(slot @ Value::Int(_)) => {
                            if let Value::Int(a) = *slot {
                                let le =
                                    crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_le());
                                *slot = Value::Bool(le);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("<=", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::GreaterImm(imm) => {
                    match self.stack.last_mut() {
                        Some(slot @ Value::Number(_)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a > imm);
                                continue;
                            }
                        }
                        Some(slot @ Value::Int(_)) => {
                            if let Value::Int(a) = *slot {
                                let gt =
                                    crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_gt());
                                *slot = Value::Bool(gt);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values(">", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::GreaterEqualImm(imm) => {
                    match self.stack.last_mut() {
                        Some(slot @ Value::Number(_)) => {
                            if let Value::Number(a) = *slot {
                                *slot = Value::Bool(a >= imm);
                                continue;
                            }
                        }
                        Some(slot @ Value::Int(_)) => {
                            if let Value::Int(a) = *slot {
                                let ge =
                                    crate::runtime::int_f64_cmp(a, imm).is_some_and(|o| o.is_ge());
                                *slot = Value::Bool(ge);
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values(">=", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::LocalAddImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get(idx) {
                        Some(Value::Number(a)) if (*a + imm).is_finite() => {
                            self.stack.push(Value::Number(*a + imm));
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_add(imm as i64) {
                                self.stack.push(Value::Int(n));
                            } else {
                                return Err(self.error(span, "Integer overflow"));
                            }
                        }
                        Some(other) => {
                            let res = eval_binary_values("+", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_sub(imm as i64) {
                                self.stack.push(Value::Int(n));
                            } else {
                                return Err(self.error(span, "Integer overflow"));
                            }
                        }
                        Some(other) => {
                            let res = eval_binary_values("-", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_mul(imm as i64) {
                                self.stack.push(Value::Int(n));
                            } else {
                                return Err(self.error(span, "Integer overflow"));
                            }
                        }
                        Some(other) => {
                            let res = eval_binary_values("*", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(n) = a.checked_rem(imm as i64) {
                                self.stack.push(Value::Int(n));
                            } else {
                                return Err(self.error(span, "Integer overflow"));
                            }
                        }
                        Some(other) => {
                            let res = eval_binary_values("%", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("==", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let lt =
                                crate::runtime::int_f64_cmp(*a, imm).is_some_and(|o| o.is_lt());
                            self.stack.push(Value::Bool(lt));
                        }
                        Some(other) => {
                            let res = eval_binary_values("<", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let le =
                                crate::runtime::int_f64_cmp(*a, imm).is_some_and(|o| o.is_le());
                            self.stack.push(Value::Bool(le));
                        }
                        Some(other) => {
                            let res = eval_binary_values("<=", other.clone(), Value::Number(imm))
                                .map_err(|m| self.error(span, m))?;
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
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("+", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::SubIntImm(imm) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut()
                        && let Some(n) = a.checked_sub(imm)
                    {
                        *a = n;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("-", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::MulIntImm(imm) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut()
                        && let Some(n) = a.checked_mul(imm)
                    {
                        *a = n;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("*", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::ModIntImm(imm) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut()
                        && let Some(n) = a.checked_rem(imm)
                    {
                        *a = n;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("%", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::BitAndIntImm(imm) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut() {
                        *a &= imm;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("&", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::BitXorIntImm(imm) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut() {
                        *a ^= imm;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("^", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::ShlIntImm(shift) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut() {
                        *a <<= shift;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("<<", left, Value::Int(i64::from(shift)))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::ShrIntImm(shift) => {
                    if let Some(Value::Int(a)) = self.stack.last_mut() {
                        *a >>= shift;
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values(">>", left, Value::Int(i64::from(shift)))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::EqualIntImm(imm) => {
                    if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                        && let Value::Int(a) = *slot
                    {
                        *slot = Value::Bool(a == imm);
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("==", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::LessIntImm(imm) => {
                    if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                        && let Value::Int(a) = *slot
                    {
                        *slot = Value::Bool(a < imm);
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("<", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::LessEqualIntImm(imm) => {
                    if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                        && let Value::Int(a) = *slot
                    {
                        *slot = Value::Bool(a <= imm);
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values("<=", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::GreaterIntImm(imm) => {
                    if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                        && let Value::Int(a) = *slot
                    {
                        *slot = Value::Bool(a > imm);
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values(">", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack.push(res);
                }
                Opcode::GreaterEqualIntImm(imm) => {
                    if let Some(slot @ Value::Int(_)) = self.stack.last_mut()
                        && let Value::Int(a) = *slot
                    {
                        *slot = Value::Bool(a >= imm);
                        continue;
                    }
                    let left = self.stack.pop().unwrap();
                    let res = eval_binary_values(">=", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("+", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("-", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("*", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("%", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("&", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("==", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("<", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            let res = eval_binary_values("<=", other.clone(), Value::Int(imm))
                                .map_err(|m| self.error(span, m))?;
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
                            continue;
                        }
                        (Some(Value::Int(a)), Value::Int(b)) => {
                            if let Some(updated) = a.checked_add(*b) {
                                *a = updated;
                                self.stack.push(Value::Int(updated));
                                continue;
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
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    let res =
                        eval_binary_values("+", left, right).map_err(|m| self.error(span, m))?;
                    self.stack[idx] = res.clone();
                    self.stack.push(res);
                }
                Opcode::StoreAddLocal(slot) => {
                    let right = self.stack.pop().unwrap();
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match (self.stack.get_mut(idx), &right) {
                        (Some(Value::Number(a)), Value::Number(b)) if (*a + *b).is_finite() => {
                            *a += *b;
                            continue;
                        }
                        (Some(Value::Int(a)), Value::Int(b)) => {
                            if let Some(updated) = a.checked_add(*b) {
                                *a = updated;
                                continue;
                            }
                        }
                        (Some(Value::Vector(a)), Value::Vector(b)) if a.len() == b.len() => {
                            let mut next = *a;
                            for (x, y) in next.iter_mut().zip(b.as_slice()) {
                                *x += *y;
                            }
                            if next.iter().all(|x| x.is_finite()) {
                                *a = next;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    let res =
                        eval_binary_values("+", left, right).map_err(|m| self.error(span, m))?;
                    self.stack[idx] = res;
                }
                Opcode::AddAssignLocalImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get_mut(idx) {
                        Some(Value::Number(a)) if (*a + imm).is_finite() => {
                            *a += imm;
                            let updated = *a;
                            self.stack.push(Value::Number(updated));
                            continue;
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(updated) = a.checked_add(imm as i64) {
                                *a = updated;
                                self.stack.push(Value::Int(updated));
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    let res = eval_binary_values("+", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack[idx] = res.clone();
                    self.stack.push(res);
                }
                Opcode::StoreAddLocalImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    match self.stack.get_mut(idx) {
                        Some(Value::Number(a)) if (*a + imm).is_finite() => {
                            *a += imm;
                            continue;
                        }
                        Some(Value::Int(a))
                            if imm.is_finite()
                                && imm.fract() == 0.0
                                && imm.abs() <= 9_007_199_254_740_991.0 =>
                        {
                            if let Some(updated) = a.checked_add(imm as i64) {
                                *a = updated;
                                continue;
                            }
                        }
                        _ => {}
                    }
                    let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    let res = eval_binary_values("+", left, Value::Number(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack[idx] = res;
                }
                Opcode::AddAssignLocalIntImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(Value::Int(a)) = self.stack.get_mut(idx)
                        && let Some(updated) = a.checked_add(imm)
                    {
                        *a = updated;
                        self.stack.push(Value::Int(updated));
                        continue;
                    }
                    let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    let res = eval_binary_values("+", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack[idx] = res.clone();
                    self.stack.push(res);
                }
                Opcode::StoreAddLocalIntImm(slot, imm) => {
                    let idx = self.frames[frame_idx].stack_base + slot as usize;
                    if let Some(Value::Int(a)) = self.stack.get_mut(idx)
                        && let Some(updated) = a.checked_add(imm)
                    {
                        *a = updated;
                        continue;
                    }
                    let left = self.stack.get(idx).cloned().unwrap_or(Value::Null);
                    let res = eval_binary_values("+", left, Value::Int(imm))
                        .map_err(|m| self.error(span, m))?;
                    self.stack[idx] = res;
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
                    self.stack.push(Value::Record(RecordMap::from_iter(pairs)));
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
                    self.stack
                        .push(Value::Record(RecordMap::from_shape_and_values(
                            shape, values,
                        )));
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

    #[inline]
    fn read_index(obj: &Value<'static>, idx: &Value<'static>) -> Result<Value<'static>, String> {
        match (obj, idx) {
            (Value::List(items) | Value::Tuple(items), _) => {
                if let Some(i) = idx.as_usize_index()
                    && i < items.len()
                {
                    Ok(items[i].clone())
                } else {
                    Err("Index out of bounds".to_string())
                }
            }
            (Value::Vector(components), _) => {
                if let Some(i) = idx.as_usize_index()
                    && i < components.len()
                {
                    Ok(Value::Number(components[i]))
                } else {
                    Err("Index out of bounds".to_string())
                }
            }
            (Value::Record(fields), Value::String(k)) => fields
                .get(k)
                .cloned()
                .ok_or_else(|| format!("Field '{k}' not found")),
            _ => Err("Cannot index value".to_string()),
        }
    }

    #[inline]
    fn read_member(
        obj: &Value<'static>,
        field_name: &str,
        ic_cell: &std::cell::Cell<u32>,
    ) -> Result<Value<'static>, String> {
        match obj {
            Value::Record(fields) => {
                let hint = ic_cell.get();
                if let Some((slot, val)) = fields.get_with_hint(field_name, hint) {
                    if slot != hint {
                        ic_cell.set(slot);
                    }
                    Ok(val.clone())
                } else {
                    Err(format!("Field '{field_name}' not found"))
                }
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first() {
                    let hint = ic_cell.get();
                    if let Some((slot, val)) = fields.get_with_hint(field_name, hint) {
                        if slot != hint {
                            ic_cell.set(slot);
                        }
                        Ok(val.clone())
                    } else {
                        Err(format!("Field '{field_name}' not found"))
                    }
                } else {
                    Err("Cannot access member on value".to_string())
                }
            }
            Value::Vector(v) => {
                let val = match field_name {
                    "x" if !v.is_empty() => v[0],
                    "y" if v.len() > 1 => v[1],
                    "z" if v.len() > 2 => v[2],
                    "w" if v.len() > 3 => v[3],
                    _ => return Err(format!("Vector field '{field_name}' not found")),
                };
                Ok(Value::Number(val))
            }
            _ => Err("Cannot access member on value".to_string()),
        }
    }

    #[inline]
    fn mutate_index(
        container: &mut Value<'static>,
        idx: &Value<'static>,
        val: Value<'static>,
    ) -> Option<&'static str> {
        match (container, idx) {
            (Value::List(items), _) => {
                if let Some(i) = idx.as_usize_index() {
                    if i < items.len() {
                        items[i] = val;
                        None
                    } else {
                        Some("Index out of bounds")
                    }
                } else if matches!(idx, Value::Int(_) | Value::Number(_)) {
                    Some("Index out of bounds")
                } else {
                    Some("Index must be an integer")
                }
            }
            (Value::Record(fields), Value::String(k)) => {
                if let Some(slot) = fields.get_mut(k.as_str()) {
                    *slot = val;
                } else {
                    fields.insert(k.clone(), val);
                }
                None
            }
            (Value::Record(_), _) => Some("Record index must be a string"),
            (Value::Vector(components), _) => {
                if let Some(i) = idx.as_usize_index() {
                    if i < components.len() {
                        let Some(num) = val.as_f64() else {
                            return Some("Vector component must be a number");
                        };
                        if !num.is_finite() {
                            return Some("Non-finite vector result");
                        }
                        components[i] = num;
                        None
                    } else {
                        Some("Index out of bounds")
                    }
                } else if matches!(idx, Value::Int(_) | Value::Number(_)) {
                    Some("Index out of bounds")
                } else {
                    Some("Index must be an integer")
                }
            }
            _ => Some("Indexing requires a list, record or vector"),
        }
    }

    #[inline]
    fn mutate_member(
        container: &mut Value<'static>,
        field_name: &str,
        val: Value<'static>,
        ic_cell: Option<&std::cell::Cell<u32>>,
    ) -> Option<&'static str> {
        match container {
            Value::Record(fields) => {
                let hint = ic_cell.map_or(u32::MAX, std::cell::Cell::get);
                if let Some((slot_idx, slot)) = fields.get_mut_with_hint(field_name, hint) {
                    if let Some(cell) = ic_cell
                        && slot_idx != hint
                    {
                        cell.set(slot_idx);
                    }
                    *slot = val;
                } else {
                    fields.insert(field_name.to_owned(), val);
                }
                None
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first_mut() {
                    let hint = ic_cell.map_or(u32::MAX, std::cell::Cell::get);
                    let Some((slot_idx, slot)) = fields.get_mut_with_hint(field_name, hint) else {
                        return Some("Unknown struct field");
                    };
                    if let Some(cell) = ic_cell
                        && slot_idx != hint
                    {
                        cell.set(slot_idx);
                    }
                    *slot = val;
                    None
                } else {
                    Some("Member access requires a struct or record")
                }
            }
            Value::Vector(components) => {
                let axis = match field_name {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return Some("Unknown vector component"),
                };
                if axis >= components.len() {
                    return Some("Vector component out of bounds");
                }
                let Some(num) = val.as_f64() else {
                    return Some("Vector component must be a number");
                };
                if !num.is_finite() {
                    return Some("Non-finite vector result");
                }
                components[axis] = num;
                None
            }
            _ => Some("Member access requires a record, struct or vector"),
        }
    }

    fn read_path_vm(
        current: &Value<'static>,
        keys: &[Value<'static>],
        member_mask: u32,
        step_idx: usize,
    ) -> Result<Value<'static>, &'static str> {
        let Some((head, rest)) = keys.split_first() else {
            return Ok(current.clone());
        };
        let is_member = (member_mask & (1u32 << step_idx)) != 0;
        if is_member {
            let Value::String(field_name) = head else {
                return Err("Invalid member name");
            };
            match current {
                Value::Record(fields) => {
                    let sub = fields.get(field_name).ok_or("Unknown field")?;
                    Self::read_path_vm(sub, rest, member_mask, step_idx + 1)
                }
                Value::UserData(data) if data.variant.is_none() => {
                    let Some(Value::Record(fields)) = data.values.first() else {
                        return Err("Member access requires a struct or record");
                    };
                    let sub = fields.get(field_name).ok_or("Unknown struct field")?;
                    Self::read_path_vm(sub, rest, member_mask, step_idx + 1)
                }
                Value::Vector(components) => {
                    let axis = match field_name.as_str() {
                        "x" => 0,
                        "y" => 1,
                        "z" => 2,
                        "w" => 3,
                        _ => return Err("Unknown vector component"),
                    };
                    if axis >= components.len() {
                        return Err("Vector component out of bounds");
                    }
                    if rest.is_empty() {
                        Ok(Value::Number(components[axis]))
                    } else {
                        Err("Vector components do not support nested access")
                    }
                }
                _ => Err("Member access requires a record, struct or vector"),
            }
        } else {
            match (current, head) {
                (Value::List(items) | Value::Tuple(items), _) => {
                    if let Some(i) = head.as_usize_index() {
                        if i < items.len() {
                            Self::read_path_vm(&items[i], rest, member_mask, step_idx + 1)
                        } else {
                            Err("Index out of bounds")
                        }
                    } else if matches!(head, Value::Int(_) | Value::Number(_)) {
                        Err("Index out of bounds")
                    } else {
                        Err("Index must be an integer")
                    }
                }
                (Value::Record(fields), Value::String(k)) => {
                    let sub = fields.get(k).ok_or("Unknown field")?;
                    Self::read_path_vm(sub, rest, member_mask, step_idx + 1)
                }
                (Value::Record(_), _) => Err("Record index must be a string"),
                (Value::Vector(components), _) => {
                    if let Some(i) = head.as_usize_index() {
                        if i < components.len() {
                            if rest.is_empty() {
                                Ok(Value::Number(components[i]))
                            } else {
                                Err("Vector components do not support nested access")
                            }
                        } else {
                            Err("Index out of bounds")
                        }
                    } else if matches!(head, Value::Int(_) | Value::Number(_)) {
                        Err("Index out of bounds")
                    } else {
                        Err("Index must be an integer")
                    }
                }
                _ => Err("Indexing requires a list, record or vector"),
            }
        }
    }

    fn mutate_path_vm(
        current: &mut Value<'static>,
        keys: &[Value<'static>],
        member_mask: u32,
        step_idx: usize,
        val: Value<'static>,
    ) -> Option<&'static str> {
        let Some((head, rest)) = keys.split_first() else {
            *current = val;
            return None;
        };
        let is_member = (member_mask & (1u32 << step_idx)) != 0;
        if rest.is_empty() {
            if is_member {
                let Value::String(field_name) = head else {
                    return Some("Invalid member name");
                };
                return Self::mutate_member(current, field_name.as_str(), val, None);
            } else {
                return Self::mutate_index(current, head, val);
            }
        }
        if is_member {
            let Value::String(field_name) = head else {
                return Some("Invalid member name");
            };
            match current {
                Value::Record(fields) => {
                    let Some(sub) = fields.get_mut(field_name) else {
                        return Some("Unknown field");
                    };
                    Self::mutate_path_vm(sub, rest, member_mask, step_idx + 1, val)
                }
                Value::UserData(data) if data.variant.is_none() => {
                    let Some(Value::Record(fields)) = data.values.first_mut() else {
                        return Some("Member access requires a struct or record");
                    };
                    let Some(sub) = fields.get_mut(field_name) else {
                        return Some("Unknown struct field");
                    };
                    Self::mutate_path_vm(sub, rest, member_mask, step_idx + 1, val)
                }
                Value::Vector(_) => Some("Vector components do not support nested access"),
                _ => Some("Member access requires a record, struct or vector"),
            }
        } else {
            match (current, head) {
                (Value::List(items), _) => {
                    if let Some(i) = head.as_usize_index() {
                        if i < items.len() {
                            Self::mutate_path_vm(
                                &mut items[i],
                                rest,
                                member_mask,
                                step_idx + 1,
                                val,
                            )
                        } else {
                            Some("Index out of bounds")
                        }
                    } else if matches!(head, Value::Int(_) | Value::Number(_)) {
                        Some("Index out of bounds")
                    } else {
                        Some("Index must be an integer")
                    }
                }
                (Value::Record(fields), Value::String(k)) => {
                    let Some(sub) = fields.get_mut(k) else {
                        return Some("Unknown field");
                    };
                    Self::mutate_path_vm(sub, rest, member_mask, step_idx + 1, val)
                }
                (Value::Record(_), _) => Some("Record index must be a string"),
                (Value::Vector(_), Value::Int(_) | Value::Number(_)) => {
                    Some("Vector components do not support nested access")
                }
                (Value::Vector(_), _) => Some("Index must be an integer"),
                _ => Some("Indexing requires a list, record or vector"),
            }
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
                                kind: BytecodeStageKind::Map,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            kind: BytecodeStageKind::Map,
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
                                kind: BytecodeStageKind::FlatMap,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            kind: BytecodeStageKind::FlatMap,
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
                            let Value::List(values) =
                                self.invoke_callable(callback, std::slice::from_ref(item), span)?
                            else {
                                return Err(
                                    self.error(span, "flat_map callback must return a list")
                                );
                            };
                            output.extend(values);
                        }
                        Ok(Value::List(output))
                    }
                    _ => Err(self.error(span, "flat_map requires a list or sequence and a callback")),
                }
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
                                kind: BytecodeStageKind::Filter,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            kind: BytecodeStageKind::Filter,
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
                Ok(Value::Record(RecordMap::from(groups)))
            }
            Builtin::Range | Builtin::RangeIter => {
                let (start, end, step) = match args {
                    [a, b] => {
                        let (Some(start), Some(end)) = (a.as_f64(), b.as_f64()) else {
                            return Err(
                                self.error(span, "range expects start, end and optional step")
                            );
                        };
                        (start, end, 1.0)
                    }
                    [a, b, c] => {
                        let (Some(start), Some(end), Some(step)) =
                            (a.as_f64(), b.as_f64(), c.as_f64())
                        else {
                            return Err(
                                self.error(span, "range expects start, end and optional step")
                            );
                        };
                        (start, end, step)
                    }
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
                let est = ((end - start) / step).ceil();
                let cap = if est.is_finite() && est > 0.0 {
                    (est as usize).min(self.fuel.saturating_add(1)).min(65_536)
                } else {
                    0
                };
                let mut values = Vec::with_capacity(cap);
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
                let Some(max_count) = args[1].as_usize_index() else {
                    if matches!(args[1], Value::Int(_) | Value::Number(_)) {
                        return Err(self.error(
                            span,
                            "Collection limit must be a nonnegative representable integer",
                        ));
                    }
                    return Err(
                        self.error(span, "collect requires a sequence and maximum item count")
                    );
                };
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
