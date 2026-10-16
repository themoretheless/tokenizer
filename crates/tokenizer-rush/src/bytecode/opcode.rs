//! Bytecode instruction set (`Opcode`), compilation chunk (`Chunk`), and function/closure descriptors.
use crate::{Builtin, Value};
use std::cell::{Cell, RefCell};
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

/// Root variable target for multi-step or compound member/index path operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathTarget {
    Local(u32),
    Upvalue(u32),
    Global(u32),
}

/// Opcodes executed by the stack-based Bytecode Virtual Machine.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Mutate container in upvalue by index: pops index and new value, pushes assigned value.
    IndexSetUpvalue(u32),
    /// Mutate member of container in upvalue by field constant index: pops new value, pushes assigned value.
    MemberSetUpvalue(u32, u32),
    /// Read value at access path of length `depth` from `target` without consuming the `depth` keys on stack.
    GetPath {
        target: PathTarget,
        depth: u8,
        member_mask: u32,
    },
    /// Mutate value at access path of length `depth` in `target`: pops new value and `depth` keys, pushes assigned value.
    SetPath {
        target: PathTarget,
        depth: u8,
        member_mask: u32,
    },
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
    /// Unary bitwise NOT (`~`).
    BitNot,
    /// Binary bitwise AND (`&`).
    BitAnd,
    /// Binary bitwise XOR (`^`).
    BitXor,
    /// Binary bitwise left shift (`<<`).
    Shl,
    /// Binary bitwise right shift (`>>`).
    Shr,
    /// Coerce top-of-stack to `Value::Int` if it is an exact integer `Value::Number`.
    CoerceInt,
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
    /// Modulo top-of-stack number by immediate f64 in-place.
    ModImm(f64),
    /// Compare top-of-stack `== imm` in-place.
    EqualImm(f64),
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
    /// Push `local[slot] % imm`.
    LocalModImm(u32, f64),
    /// Push `local[slot] == imm`.
    LocalEqualImm(u32, f64),
    /// Push `local[slot] < imm`.
    LocalLessImm(u32, f64),
    /// Push `local[slot] <= imm`.
    LocalLessEqualImm(u32, f64),
    /// Add immediate i64 to top-of-stack integer in-place.
    AddIntImm(i64),
    /// Subtract immediate i64 from top-of-stack integer in-place.
    SubIntImm(i64),
    /// Multiply top-of-stack integer by immediate i64 in-place.
    MulIntImm(i64),
    /// Modulo top-of-stack integer by immediate i64 in-place.
    ModIntImm(i64),
    /// Bitwise AND top-of-stack integer with immediate i64 in-place.
    BitAndIntImm(i64),
    /// Bitwise XOR top-of-stack integer with immediate i64 in-place.
    BitXorIntImm(i64),
    /// Bitwise left shift top-of-stack integer by immediate shift count in-place.
    ShlIntImm(u32),
    /// Bitwise right shift top-of-stack integer by immediate shift count in-place.
    ShrIntImm(u32),
    /// Compare top-of-stack `== imm (i64)` in-place.
    EqualIntImm(i64),
    /// Compare top-of-stack `< imm (i64)` in-place.
    LessIntImm(i64),
    /// Compare top-of-stack `<= imm (i64)` in-place.
    LessEqualIntImm(i64),
    /// Compare top-of-stack `> imm (i64)` in-place.
    GreaterIntImm(i64),
    /// Compare top-of-stack `>= imm (i64)` in-place.
    GreaterEqualIntImm(i64),
    /// Push `local[slot] + imm (i64)`.
    LocalAddIntImm(u32, i64),
    /// Push `local[slot] - imm (i64)`.
    LocalSubIntImm(u32, i64),
    /// Push `local[slot] * imm (i64)`.
    LocalMulIntImm(u32, i64),
    /// Push `local[slot] % imm (i64)`.
    LocalModIntImm(u32, i64),
    /// Push `local[slot] & imm (i64)`.
    LocalBitAndIntImm(u32, i64),
    /// Push `local[slot] == imm (i64)`.
    LocalEqualIntImm(u32, i64),
    /// Push `local[slot] < imm (i64)`.
    LocalLessIntImm(u32, i64),
    /// Push `local[slot] <= imm (i64)`.
    LocalLessEqualIntImm(u32, i64),
    /// Add top-of-stack to `local[slot]` and leave result on stack.
    AddAssignLocal(u32),
    /// Pop top-of-stack and add directly to `local[slot]` (fused `AddAssignLocal` + `Pop`).
    StoreAddLocal(u32),
    /// Add immediate f64 to `local[slot]` and push result.
    AddAssignLocalImm(u32, f64),
    /// Add immediate f64 directly to `local[slot]` in-place (fused `AddAssignLocalImm` + `Pop`).
    StoreAddLocalImm(u32, f64),
    /// Add immediate i64 to `local[slot]` and push result.
    AddAssignLocalIntImm(u32, i64),
    /// Add immediate i64 directly to `local[slot]` in-place (fused `AddAssignLocalIntImm` + `Pop`).
    StoreAddLocalIntImm(u32, i64),
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
    /// Pop N value items and push `Value::Record` using precomputed shape descriptor `u32`.
    BuildRecordShape(u32),
    /// Pop index, pop container, push indexed element.
    IndexGet,
    /// Pop index, read element from container at local slot in-place without cloning container.
    LocalIndexGet(u32),
    /// Pop index, read element from container at upvalue index in-place without cloning container.
    UpvalueIndexGet(u32),
    /// Mutate container at local slot by index: pops index and new value, pushes assigned value.
    IndexSet(u32),
    /// Mutate container at local slot by index without pushing result (fused `IndexSet` + `Pop`).
    StoreIndexSet(u32),
    /// Pop container, push member by field name constant index.
    MemberGet(u32),
    /// Read member of container at local slot in-place without cloning container.
    LocalMemberGet(u32, u32),
    /// Read member of container at upvalue index in-place without cloning container.
    UpvalueMemberGet(u32, u32),
    /// Mutate member of container at local slot by field constant index: pops new value, pushes assigned value.
    MemberSet(u32, u32),
    /// Mutate member of container at local slot without pushing result (fused `MemberSet` + `Pop`).
    StoreMemberSet(u32, u32),
    /// Pop N parts and interpolate into a single string.
    Interpolate(usize),
    /// Call callee with N arguments.
    Call(usize),
    /// Tail-call callee with N arguments, reusing the current call frame in-place.
    TailCall(usize),
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

/// Precompiled record shape and optional source-to-sorted-slot mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordShapeDescriptor {
    /// Sorted unique field names shared by all records created at this site.
    pub shape: crate::RecordShape,
    /// Maps each source expression position `0..N` to its sorted slot index `0..N`,
    /// or `None` when source order is already sorted.
    pub source_to_slot: Option<Box<[usize]>>,
}

/// A compiled bytecode compilation unit containing instructions, constants and functions.
#[derive(Clone, Debug, Default)]
pub struct Chunk {
    pub code: Vec<Opcode>,
    pub constants: Vec<Value<'static>>,
    pub functions: Vec<Rc<BytecodeFunction>>,
    pub record_shapes: Vec<RecordShapeDescriptor>,
    pub spans: Vec<Span>,
    /// Per-instruction inline-cache slot hints for $O(1)$ record/struct field lookup.
    pub ic_slots: Vec<Cell<u32>>,
    pub local_count: usize,
}

impl PartialEq for Chunk {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
            && self.constants == other.constants
            && self.functions == other.functions
            && self.record_shapes == other.record_shapes
            && self.spans == other.spans
            && self.local_count == other.local_count
    }
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
                Some(Opcode::AddAssignLocalIntImm(slot, imm)) => {
                    let (s, v) = (*slot, *imm);
                    *self.code.last_mut().expect("present") = Opcode::StoreAddLocalIntImm(s, v);
                    return self.code.len() - 1;
                }
                Some(Opcode::IndexSet(slot)) => {
                    let s = *slot;
                    *self.code.last_mut().expect("present") = Opcode::StoreIndexSet(s);
                    return self.code.len() - 1;
                }
                Some(Opcode::MemberSet(slot, field_idx)) => {
                    let (s, f) = (*slot, *field_idx);
                    *self.code.last_mut().expect("present") = Opcode::StoreMemberSet(s, f);
                    return self.code.len() - 1;
                }
                _ => {}
            }
        }
        let idx = self.code.len();
        self.code.push(op);
        self.spans.push(span);
        self.ic_slots.push(Cell::new(0));
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

    pub fn add_record_shape(&mut self, desc: RecordShapeDescriptor) -> u32 {
        for (i, existing) in self.record_shapes.iter().enumerate() {
            if existing == &desc {
                return i as u32;
            }
        }
        let idx = self.record_shapes.len() as u32;
        self.record_shapes.push(desc);
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
