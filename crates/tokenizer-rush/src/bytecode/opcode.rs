//! Bytecode instruction set (`Opcode`), compilation chunk (`Chunk`), and function/closure descriptors.
use crate::{Builtin, Value};
use std::cell::RefCell;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

/// Upvalue state for variables captured in closures.
#[derive(Clone, Debug, PartialEq)]
pub enum Upvalue {
    /// The variable is still alive on the VM operand/locals stack.
    Open(usize),
    /// The variable's stack frame has exited; the value now lives on the heap.
    Closed(Value<'static>),
}

/// Upvalue binding descriptor used during compilation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpvalueDescriptor {
    pub is_local: bool,
    pub index: u32,
}

/// A compiled bytecode function.
#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeFunction {
    pub name: Option<String>,
    pub arity: usize,
    pub chunk: Chunk,
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
}

/// A runtime closure wrapping a compiled function and its captured upvalue cells.
#[derive(Clone, Debug)]
pub struct BytecodeClosure {
    pub function: Rc<BytecodeFunction>,
    pub upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl PartialEq for BytecodeClosure {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.function, &other.function)
            && self.upvalues.len() == other.upvalues.len()
            && self
                .upvalues
                .iter()
                .zip(&other.upvalues)
                .all(|(a, b)| Rc::ptr_eq(a, b))
    }
}

/// Opcodes executed by the stack-based Bytecode Virtual Machine.
#[derive(Clone, Debug, PartialEq)]
pub enum Opcode {
    /// Load constant from constant pool.
    Constant(u32),
    /// Push `null`.
    Null,
    /// Push `true`.
    True,
    /// Push `false`.
    False,
    /// Discard top value from stack.
    Pop,
    /// Duplicate top value on stack.
    Dup,
    /// Read local variable at relative stack slot.
    GetLocal(u32),
    /// Write top-of-stack to local variable slot (leaving value on stack).
    SetLocal(u32),
    /// Read global variable by name constant index.
    GetGlobal(u32),
    /// Write top-of-stack to global variable by name constant index.
    SetGlobal(u32),
    /// Push resolved builtin function directly without constant-pool or global lookup.
    GetBuiltin(Builtin),
    /// Call a statically resolved builtin with N arguments on the stack.
    CallBuiltin(Builtin, usize),
    /// Read captured upvalue at closure index.
    GetUpvalue(u32),
    /// Write top-of-stack to captured upvalue at closure index.
    SetUpvalue(u32),
    /// Mutate container in upvalue by index: pops index and new value.
    IndexSetUpvalue(u32),
    /// Mutate member of container in upvalue by field constant index: pops new value.
    MemberSetUpvalue(u32, u32),
    /// Instantiate a closure from function constant index with captured upvalues.
    MakeClosure(u32),
    /// Binary addition / string concatenation / vector addition / list concatenation.
    Add,
    /// Binary subtraction.
    Sub,
    /// Binary multiplication.
    Mul,
    /// Binary division.
    Div,
    /// Binary remainder.
    Mod,
    /// Binary exponentiation (`**`).
    Pow,
    /// Unary negation.
    Neg,
    /// Unary plus.
    Pos,
    /// Unary logical NOT.
    Not,
    /// Equality comparison.
    Equal,
    /// Inequality comparison.
    NotEqual,
    /// Less than.
    Less,
    /// Less than or equal.
    LessEqual,
    /// Greater than.
    Greater,
    /// Greater than or equal.
    GreaterEqual,
    /// Add immediate f64 to top-of-stack number in-place.
    AddImm(f64),
    /// Subtract immediate f64 from top-of-stack number in-place.
    SubImm(f64),
    /// Multiply top-of-stack number by immediate f64 in-place.
    MulImm(f64),
    /// Compare top-of-stack `< imm` in-place.
    LessImm(f64),
    /// Compare top-of-stack `<= imm` in-place.
    LessEqualImm(f64),
    /// Compare top-of-stack `> imm` in-place.
    GreaterImm(f64),
    /// Compare top-of-stack `>= imm` in-place.
    GreaterEqualImm(f64),
    /// Push `local[slot] + imm`.
    LocalAddImm(u32, f64),
    /// Push `local[slot] - imm`.
    LocalSubImm(u32, f64),
    /// Push `local[slot] * imm`.
    LocalMulImm(u32, f64),
    /// Push `local[slot] <= imm`.
    LocalLessEqualImm(u32, f64),
    /// Add top-of-stack to `local[slot]` and leave result on stack.
    AddAssignLocal(u32),
    /// Pop top-of-stack and add directly to `local[slot]` (fused `AddAssignLocal` + `Pop`).
    StoreAddLocal(u32),
    /// Add immediate f64 to `local[slot]` and push result.
    AddAssignLocalImm(u32, f64),
    /// Add immediate f64 directly to `local[slot]` in-place (fused `AddAssignLocalImm` + `Pop`).
    StoreAddLocalImm(u32, f64),
    /// Unconditional jump to target instruction offset.
    Jump(usize),
    /// Jump if top of stack is false / falsy.
    JumpIfFalse(usize),
    /// Jump if top of stack is true / truthy.
    JumpIfTrue(usize),
    /// Pop N items and push `Value::List`.
    BuildList(usize),
    /// Pop N items and push `Value::Tuple`.
    BuildTuple(usize),
    /// Pop 2*N items and push `Value::Record`.
    BuildRecord(usize),
    /// Pop index, pop container, push indexed element.
    IndexGet,
    /// Mutate container at local slot by index: pops index and new value.
    IndexSet(u32),
    /// Pop container, push member by field name constant index.
    MemberGet(u32),
    /// Mutate member of container at local slot by field constant index: pops new value.
    MemberSet(u32, u32),
    /// Pop N parts and interpolate into a single string.
    Interpolate(usize),
    /// Call callee with N arguments.
    Call(usize),
    /// Check if top of stack is a tuple with exact arity (pops tuple, pushes bool).
    CheckTuple(usize),
    /// Assert top of stack is a tuple with exact arity (pops tuple, errors if invalid).
    AssertTuple(usize),
    /// Extract element from tuple on top of stack (pops tuple, pushes element).
    TupleGet(u32),
    /// Check if top of stack is a record or struct (pops value, pushes bool).
    CheckRecord,
    /// Assert top of stack is a record or struct (pops value, errors if invalid).
    AssertRecord,
    /// Check if record on top of stack has given field (pops record, pushes bool).
    CheckRecordField(u32),
    /// Extract field from record on top of stack (pops record, pushes field, errors if missing).
    RecordGetAssert(u32),
    /// Check if top of stack is a variant with tag and arity (pops variant, pushes bool).
    CheckVariant(u32, usize),
    /// Extract payload item from variant on top of stack (pops variant, pushes item).
    VariantGet(u32),
    /// Raise runtime error "No matching pattern".
    MatchError,
    /// Construct UserData instance with type_name, optional variant and arguments.
    MakeUserData {
        type_name_idx: u32,
        variant_idx: Option<u32>,
        arity: usize,
    },
    /// Initialize iterator for iterable on top of stack and store at local slot.
    IterInit(u32),
    /// Advance iterator at local slot: pushes next item, or jumps to exit target.
    IterNext { iter_slot: u32, exit_jump: usize },
    /// Try-propagation `?`: unwrap `Some`/`Ok` or early-return `None`/`Err`.
    TryPropagate,
    /// Pop top-of-stack and store directly into local variable slot (fused SetLocal + Pop).
    StoreLocal(u32),
    /// Pop top-of-stack and store directly into captured upvalue at closure index (fused SetUpvalue + Pop).
    StoreUpvalue(u32),
    /// Pop and assert region budget is a non-negative integer.
    AssertRegionBudget,
    /// Return from execution.
    Return,
}

/// A compiled bytecode compilation unit containing instructions, constants and functions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub code: Vec<Opcode>,
    pub constants: Vec<Value<'static>>,
    pub functions: Vec<Rc<BytecodeFunction>>,
    pub spans: Vec<Span>,
    pub local_count: usize,
}

impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&mut self, op: Opcode, span: Span) -> usize {
        if op == Opcode::Pop {
            match self.code.last_mut() {
                Some(Opcode::SetLocal(slot)) => {
                    let s = *slot;
                    *self.code.last_mut().expect("present") = Opcode::StoreLocal(s);
                    return self.code.len() - 1;
                }
                Some(Opcode::SetUpvalue(idx)) => {
                    let i = *idx;
                    *self.code.last_mut().expect("present") = Opcode::StoreUpvalue(i);
                    return self.code.len() - 1;
                }
                Some(Opcode::AddAssignLocal(slot)) => {
                    let s = *slot;
                    *self.code.last_mut().expect("present") = Opcode::StoreAddLocal(s);
                    return self.code.len() - 1;
                }
                Some(Opcode::AddAssignLocalImm(slot, imm)) => {
                    let (s, v) = (*slot, *imm);
                    *self.code.last_mut().expect("present") = Opcode::StoreAddLocalImm(s, v);
                    return self.code.len() - 1;
                }
                _ => {}
            }
        }
        let idx = self.code.len();
        self.code.push(op);
        self.spans.push(span);
        idx
    }

    pub fn add_constant(&mut self, value: Value<'static>) -> u32 {
        for (i, c) in self.constants.iter().enumerate() {
            if c == &value {
                return i as u32;
            }
        }
        let idx = self.constants.len() as u32;
        self.constants.push(value);
        idx
    }

    pub fn add_function(&mut self, function: Rc<BytecodeFunction>) -> u32 {
        let idx = self.functions.len() as u32;
        self.functions.push(function);
        idx
    }

    pub fn patch_jump(&mut self, instruction_idx: usize, mut target: usize) {
        // Jump threading: skip intermediate unconditional jumps
        while target < self.code.len() {
            if let Opcode::Jump(next_target) = self.code[target] {
                if next_target == target {
                    break;
                }
                target = next_target;
            } else {
                break;
            }
        }
        match &mut self.code[instruction_idx] {
            Opcode::Jump(t) | Opcode::JumpIfFalse(t) | Opcode::JumpIfTrue(t) => {
                *t = target;
            }
            Opcode::IterNext { exit_jump, .. } => {
                *exit_jump = target;
            }
            _ => panic!("Expected jump instruction at index {instruction_idx}"),
        }
    }
}
