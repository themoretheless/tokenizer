//! Flat stack-based Bytecode Virtual Machine runtime with superinstructions,
//! zero-allocation builtin dispatch, and ExecutionLimits / CancellationToken support.

mod access;
mod builtins;
mod dispatch;
mod ops;

use crate::bytecode::opcode::{BytecodeClosure, BytecodeFunction, Chunk, Upvalue};
use crate::runtime::memory::{Budget, CoverError};
use crate::runtime::ops::eval_binary_values;
use crate::{CancellationToken, ExecutionLimits, RuntimeError, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

pub(super) struct CallFrame {
    pub(super) closure: Rc<BytecodeClosure>,
    pub(super) ip: usize,
    pub(super) stack_base: usize,
    pub(super) coerce_int: bool,
}

/// The flat stack Bytecode Virtual Machine runtime.
pub struct Vm<'c> {
    pub(crate) fuel: usize,
    pub(super) max_depth: usize,
    pub(super) cancel: Option<&'c CancellationToken>,
    pub(super) memory: Budget,
    pub(super) globals: HashMap<String, Value<'static>>,
    pub(super) frames: Vec<CallFrame>,
    pub(super) stack: Vec<Value<'static>>,
    pub(super) open_upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl<'c> Vm<'c> {
    pub fn new(budget: usize) -> Self {
        Self::with_shared_budget(budget, Budget::with_allocated_limit(usize::MAX, usize::MAX))
    }

    pub fn with_limits(limits: ExecutionLimits, cancel: Option<&'c CancellationToken>) -> Self {
        Self {
            fuel: limits.steps,
            max_depth: limits.max_depth.max(1),
            cancel,
            memory: Budget::with_allocated_limit(limits.max_heap_bytes, limits.max_allocated_bytes),
            globals: HashMap::new(),
            frames: Vec::with_capacity(16),
            stack: Vec::with_capacity(64),
            open_upvalues: Vec::new(),
        }
    }

    /// A nested VM sharing the caller's memory budget, so limits hold across
    /// tree-walk/bytecode boundaries. Fuel is synchronized back by the caller.
    pub(crate) fn with_shared_budget(fuel: usize, memory: Budget) -> Self {
        Self {
            fuel,
            max_depth: 512,
            cancel: None,
            memory,
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

    /// Maps a Cover rejection to the corresponding limit error.
    pub(super) fn cover_error(&self, span: Span, error: CoverError) -> RuntimeError {
        self.error(
            span,
            match error {
                CoverError::Heap => "Heap limit exceeded",
                CoverError::Allocated => "Allocation limit exceeded",
            },
        )
    }

    /// Charges the cumulative work budget for a produced value's fresh
    /// storage, mirroring string_growth/collection_growth in the tree-walk
    /// runtime. Scalars and shared structures allocate nothing new.
    pub(super) fn charge_produced(
        &self,
        value: &Value<'static>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let bytes = match value {
            Value::String(s) => s.len(),
            Value::List(items) | Value::Tuple(items) => {
                items.len().saturating_mul(std::mem::size_of::<Value>())
            }
            Value::Record(fields) => fields.len().saturating_mul(std::mem::size_of::<Value>()),
            _ => 0,
        };
        if bytes > 0 && self.memory.charge_allocated(bytes).is_err() {
            return Err(self.error(span, "Allocation limit exceeded"));
        }
        Ok(())
    }

    /// Binary operation with the produced storage charged to the work budget.
    pub(super) fn eval_binary(
        &self,
        op: &str,
        left: Value<'static>,
        right: Value<'static>,
        span: Span,
    ) -> Result<Value<'static>, RuntimeError> {
        let res = eval_binary_values(op, left, right).map_err(|m| self.error(span, m))?;
        self.charge_produced(&res, span)?;
        Ok(res)
    }

    #[inline(always)]
    pub(super) fn const_str(chunk: &Chunk, idx: u32) -> &str {
        match &chunk.constants[idx as usize] {
            Value::String(s) => s.as_str(),
            _ => unreachable!("Expected string constant"),
        }
    }

    #[inline]
    pub(super) fn coerce_int_in_place(
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
    pub(super) fn unwind_frame(&mut self, exiting: CallFrame) {
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

    pub(super) fn close_upvalues(&mut self, from_slot: usize) {
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
}
