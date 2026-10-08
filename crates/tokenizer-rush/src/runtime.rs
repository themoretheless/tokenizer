//! Bounded expression evaluator. Closures capture immutable snapshots.
use crate::{Block, Expr, ExprKind, Name, Stmt, StmtKind};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::rc::{Rc, Weak};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use themoretheless_tokenizer_core::Span;
mod cell_gc;
mod coroutine;
mod model;
mod scheduler;
pub use coroutine::{CoroutineId, CoroutineState};
pub use model::{ModelInstance, ModelNode, ModelProgram};
pub use scheduler::{
    CoroutineScheduler, OwnedCoroutineScheduler, ScheduledState, ScheduledStep, WakeRequest,
};
mod instance;
pub use instance::{OwnedScriptInstance, ScriptState, StateValue};
pub mod json;
pub use json::{parse as json_parse, stringify as json_stringify};
mod memory;

#[derive(Clone, Debug, PartialEq)]
enum Binding<'s> {
    Value(Value<'s>),
    Cell(Rc<CellId>),
}
/// Bindings and captured environments keep a slot alive. The runtime owns its
/// value separately so assigning a closure that captures itself cannot create
/// an Rc cycle owning the runtime. Dead slots are cleared at scope exit and
/// before allocation.
#[derive(Debug)]
struct CellId {
    index: usize,
    released: Weak<RefCell<memory::Slots<usize>>>,
}
impl PartialEq for CellId {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && Weak::ptr_eq(&self.released, &other.released)
    }
}
impl Drop for CellId {
    fn drop(&mut self) {
        if let Some(released) = self.released.upgrade() {
            released
                .borrow_mut()
                .push(self.index)
                .expect("release queue reserved with cell");
        }
    }
}
#[derive(Debug)]
struct Environment<'s> {
    bindings: memory::Slots<(&'s str, Binding<'s>)>,
    budget: memory::Budget,
    parent: Option<memory::Shared<Environment<'s>>>,
}
impl PartialEq for Environment<'_> {
    fn eq(&self, other: &Self) -> bool {
        *self.bindings == *other.bindings && self.parent == other.parent
    }
}
impl<'s> Environment<'s> {
    fn new(budget: &memory::Budget) -> Self {
        Self {
            bindings: memory::Slots::new(budget, 0).expect("empty bindings require no allocation"),
            budget: budget.clone(),
            parent: None,
        }
    }
    fn child(parent: memory::Shared<Self>) -> Self {
        let mut child = Self::new(&parent.budget);
        child.parent = Some(parent);
        child
    }
    fn try_clone(&self) -> std::result::Result<Self, memory::AllocationError> {
        let mut copy = Self::new(&self.budget);
        copy.bindings.reserve(self.bindings.len())?;
        for (name, binding) in self.bindings.iter() {
            copy.bindings
                .push((*name, binding.clone()))
                .expect("copy capacity reserved");
        }
        copy.parent = self.parent.clone();
        Ok(copy)
    }
    /// Drop every binding so cell references release exactly as they would
    /// when a freshly cloned scope goes out of scope.
    fn release_bindings(&mut self) {
        while self.bindings.pop().is_some() {}
    }
    /// Rebind this scope as a child of `parent`, reusing the allocated
    /// binding storage. Used to recycle per-call environments.
    fn reset_child(&mut self, parent: memory::Shared<Self>) {
        self.release_bindings();
        self.parent = Some(parent);
    }
    fn get(&self, name: &str) -> Option<&Binding<'s>> {
        let mut current = self;
        loop {
            if let Ok(index) = current
                .bindings
                .binary_search_by_key(&name, |(key, _)| *key)
            {
                return Some(&current.bindings[index].1);
            }
            current = current.parent.as_deref()?;
        }
    }
    fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    fn insert_binding(
        &mut self,
        name: &'s str,
        binding: Binding<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        match self.bindings.binary_search_by_key(&name, |(key, _)| *key) {
            Ok(index) => self.bindings[index].1 = binding,
            Err(index) => {
                self.bindings.push((name, binding))?;
                self.bindings[index..].rotate_right(1);
            }
        }
        Ok(())
    }
    fn insert(
        &mut self,
        name: &'s str,
        value: Value<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        self.insert_binding(name, Binding::Value(value))
    }
    fn extend(&mut self, mut other: Self) -> std::result::Result<(), memory::AllocationError> {
        debug_assert!(other.parent.is_none());
        let additional = other
            .bindings
            .iter()
            .filter(|(name, _)| {
                self.bindings
                    .binary_search_by_key(name, |(key, _)| *key)
                    .is_err()
            })
            .count();
        self.bindings.reserve(additional)?;
        while let Some((name, binding)) = other.bindings.pop() {
            self.insert_binding(name, binding)
                .expect("extension capacity reserved");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value<'s> {
    /// Nominal user data; struct payload is one record, enum payload is positional.
    UserData(Box<UserData<'s>>),
    HostObject(crate::HostObject),
    /// Angle stored in radians, distinct from an ordinary number.
    Angle(f64),
    Variant(&'static str, Vec<Value<'s>>),
    Mesh(Rc<crate::Mesh>),
    Quaternion(Box<crate::Quaternion>),
    String(String),
    Record(BTreeMap<String, Value<'s>>),
    Matrix(Box<crate::Matrix4>),
    Polygon(crate::Polygon),
    Number(f64),
    Vector(Vec<f64>),
    Bool(bool),
    Null,
    /// Repeatable numeric sequence; elements are computed only when consumed.
    Range {
        start: f64,
        end: f64,
        step: f64,
    },
    Sequence(Rc<Sequence<'s>>),
    List(Vec<Value<'s>>),
    Tuple(Vec<Value<'s>>),
    Function(Rc<Closure<'s>>),
    BytecodeFunction(Rc<crate::bytecode::BytecodeClosure>),
    BytecodeSequence(Rc<crate::bytecode::BytecodeSequence>),
    BytecodeIterator(Rc<RefCell<crate::bytecode::BytecodeIterator>>),
    Builtin(Builtin),
    Host(Rc<HostFunction>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserData<'s> {
    pub type_name: String,
    pub variant: Option<String>,
    pub values: Vec<Value<'s>>,
}

/// Factory for a host-owned source. Each consumption opens a fresh iterator.
/// Keep resource handles in the iterator and release them in its Drop implementation.
pub trait HostSequence: std::fmt::Debug {
    fn open(
        &self,
        cancellation: &CancellationToken,
    ) -> std::result::Result<Box<dyn HostSequenceIterator>, String>;
}

/// One opened source. Values own their data; no reference to a temporary host buffer escapes.
/// Blocking implementations must cooperate with the cancellation token themselves.
pub trait HostSequenceIterator {
    fn next(
        &mut self,
        cancellation: &CancellationToken,
    ) -> std::result::Result<Option<Value<'static>>, String>;
}

#[derive(Clone, Debug)]
struct HostSource {
    factory: Rc<dyn HostSequence>,
    item_type: ValueType,
}
impl PartialEq for HostSource {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.factory, &other.factory) && self.item_type == other.item_type
    }
}
impl<'s> Value<'s> {
    /// Create a lazy source without opening a resource. Items are checked against item_type.
    pub fn host_sequence(factory: Rc<dyn HostSequence>, item_type: ValueType) -> Self {
        Self::Sequence(Rc::new(Sequence {
            source: SequenceSource::Host(HostSource { factory, item_type }),
            stages: SequenceStages::default(),
        }))
    }

    /// Try to convert to an owned static value if all inner parts can be static.
    pub fn to_static(&self) -> Option<Value<'static>> {
        match self {
            Value::Null => Some(Value::Null),
            Value::Bool(b) => Some(Value::Bool(*b)),
            Value::Number(n) => Some(Value::Number(*n)),
            Value::Angle(a) => Some(Value::Angle(*a)),
            Value::String(s) => Some(Value::String(s.clone())),
            Value::Vector(v) => Some(Value::Vector(v.clone())),
            Value::Matrix(m) => Some(Value::Matrix(m.clone())),
            Value::Quaternion(q) => Some(Value::Quaternion(q.clone())),
            Value::Mesh(m) => Some(Value::Mesh(m.clone())),
            Value::Polygon(p) => Some(Value::Polygon(p.clone())),
            Value::Range { start, end, step } => Some(Value::Range {
                start: *start,
                end: *end,
                step: *step,
            }),
            Value::List(items) => {
                let static_items: Option<Vec<_>> = items.iter().map(|it| it.to_static()).collect();
                static_items.map(Value::List)
            }
            Value::Tuple(items) => {
                let static_items: Option<Vec<_>> = items.iter().map(|it| it.to_static()).collect();
                static_items.map(Value::Tuple)
            }
            Value::Record(fields) => {
                let mut static_fields = BTreeMap::new();
                for (k, v) in fields {
                    static_fields.insert(k.clone(), v.to_static()?);
                }
                Some(Value::Record(static_fields))
            }
            Value::Variant(tag, items) => {
                let static_items: Option<Vec<_>> = items.iter().map(|it| it.to_static()).collect();
                static_items.map(|its| Value::Variant(tag, its))
            }
            Value::Builtin(b) => Some(Value::Builtin(*b)),
            Value::Host(h) => Some(Value::Host(h.clone())),
            Value::HostObject(o) => Some(Value::HostObject(o.clone())),
            Value::BytecodeFunction(f) => Some(Value::BytecodeFunction(f.clone())),
            Value::BytecodeSequence(s) => Some(Value::BytecodeSequence(s.clone())),
            Value::BytecodeIterator(i) => Some(Value::BytecodeIterator(i.clone())),
            _ => None,
        }
    }
}

pub(crate) fn format_value_for_display(value: &Value<'_>, out: &mut String, depth: usize) {
    if depth > 16 {
        out.push_str("...");
        return;
    }
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) {
                let _ = write!(out, "{:.0}", n);
            } else {
                let _ = write!(out, "{}", n);
            }
        }
        Value::Angle(a) => {
            let _ = write!(out, "{} rad", a);
        }
        Value::String(s) => {
            if depth == 0 {
                out.push_str(s);
            } else {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        _ => out.push(c),
                    }
                }
                out.push('"');
            }
        }
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_for_display(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::Tuple(items) => {
            out.push('(');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_for_display(item, out, depth + 1);
            }
            if items.len() == 1 {
                out.push(',');
            }
            out.push(')');
        }
        Value::Record(fields) => {
            out.push_str("{ ");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(k);
                out.push_str(": ");
                format_value_for_display(v, out, depth + 1);
            }
            out.push_str(" }");
        }
        Value::Vector(comps) => {
            match comps.len() {
                2 => out.push_str("vec2("),
                3 => out.push_str("vec3("),
                4 => out.push_str("vec4("),
                _ => out.push_str("vec("),
            }
            for (i, c) in comps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "{}", c);
            }
            out.push(')');
        }
        Value::Variant(tag, payload) => {
            out.push_str(tag);
            if !payload.is_empty() {
                out.push('(');
                for (i, item) in payload.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_value_for_display(item, out, depth + 1);
                }
                out.push(')');
            }
        }
        Value::UserData(data) => {
            out.push_str(&data.type_name);
            if let Some(var) = &data.variant {
                out.push('.');
                out.push_str(var);
            }
            out.push('(');
            for (i, item) in data.values.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_for_display(item, out, depth + 1);
            }
            out.push(')');
        }
        Value::Function(f) => {
            if let Some(name) = f.name() {
                let _ = write!(out, "<function {}>", name);
            } else {
                out.push_str("<function>");
            }
        }
        Value::BytecodeFunction(f) => {
            if let Some(name) = f.function.name.as_deref() {
                let _ = write!(out, "<function {}>", name);
            } else {
                out.push_str("<function>");
            }
        }
        Value::BytecodeSequence(_) => {
            out.push_str("<sequence>");
        }
        Value::BytecodeIterator(_) => {
            out.push_str("<iterator>");
        }
        Value::Builtin(b) => {
            let _ = write!(out, "<builtin {:?}>", b);
        }
        Value::Host(h) => {
            let _ = write!(out, "<host function {}>", h.name);
        }
        Value::HostObject(o) => {
            let _ = write!(out, "<host object {:?}>", o);
        }
        Value::Range { start, end, step } => {
            let _ = write!(out, "range({}, {}, {})", start, end, step);
        }
        Value::Mesh(_) => out.push_str("<Mesh>"),
        Value::Polygon(_) => out.push_str("<Polygon>"),
        Value::Matrix(_) => out.push_str("<Matrix4>"),
        Value::Quaternion(_) => out.push_str("<Quaternion>"),
        Value::Sequence(_) => out.push_str("<sequence>"),
    }
}

/// Repeatable sequence with deferred transformations. Construct through Rush builtins.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence<'s> {
    source: SequenceSource<'s>,
    stages: SequenceStages<'s>,
}

impl<'s> Sequence<'s> {
    pub(crate) fn to_bytecode_sequence(&self) -> Option<crate::bytecode::BytecodeSequence> {
        let source = match &self.source {
            SequenceSource::Range { start, end, step } => {
                crate::bytecode::BytecodeSequenceSource::Range {
                    start: *start,
                    end: *end,
                    step: *step,
                }
            }
            SequenceSource::List(buf) => {
                let items: Option<Vec<Value<'static>>> =
                    buf.iter().map(|v| v.to_static()).collect();
                crate::bytecode::BytecodeSequenceSource::List(items?)
            }
            SequenceSource::Host(_) => return None,
        };
        let mut stages = Vec::with_capacity(self.stages.len());
        for stage in self.stages.iter() {
            stages.push(crate::bytecode::BytecodeSequenceStage {
                callback: stage.callback.to_static()?,
                filter: stage.filter,
                span: stage.span,
            });
        }
        Some(crate::bytecode::BytecodeSequence { source, stages })
    }
}

#[derive(Clone, Debug, PartialEq)]
enum SequenceSource<'s> {
    Range { start: f64, end: f64, step: f64 },
    List(memory::Buffer<Value<'s>>),
    Host(HostSource),
}

#[derive(Clone, Debug, PartialEq)]
struct SequenceStage<'s> {
    callback: Value<'s>,
    filter: bool,
    span: Span,
    module: Option<&'s str>,
}

#[derive(Clone, Debug, Default)]
struct SequenceStages<'s>(Option<memory::Shared<memory::Slots<SequenceStage<'s>>>>);
impl PartialEq for SequenceStages<'_> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl<'s> std::ops::Deref for SequenceStages<'s> {
    type Target = [SequenceStage<'s>];
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().map(|slots| &**slots).unwrap_or(&[])
    }
}
impl<'s> SequenceStages<'s> {
    fn capacity(&self) -> usize {
        self.0.as_ref().map_or(0, |slots| slots.capacity())
    }
    fn append(
        &mut self,
        budget: &memory::Budget,
        stage: SequenceStage<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        let size = self
            .len()
            .checked_add(1)
            .ok_or(memory::AllocationError::Capacity)?;
        let mut slots = memory::Slots::new(budget, size)?;
        for existing in self.iter() {
            slots
                .push(existing.clone())
                .expect("stage capacity reserved");
        }
        slots.push(stage).expect("stage capacity reserved");
        let shared = memory::Shared::new(budget, slots)?;
        self.0 = Some(shared);
        Ok(())
    }
}

struct SequenceCursor<'s> {
    sequence: Rc<Sequence<'s>>,
    index: usize,
    previous: Option<f64>,
    host: Option<Box<dyn HostSequenceIterator>>,
    finished: bool,
}

/// Pure collection operations available in the initial environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Degrees,
    Radians,
    Some,
    None,
    Ok,
    Err,
    Promote,
    Random,
    Noise,
    GridMesh,
    Mesh,
    Transform,
    Slerp,
    AxisAngle,
    RotationMatrix,
    Zip,
    Identity,
    Translation,
    Scaling,
    RotationX,
    RotationY,
    RotationZ,
    TransformPoint,
    TransformDirection,
    Polygon,
    Translate,
    Rotate,
    Range,
    RangeIter,
    Iter,
    Collect,
    Cross,
    Lerp,
    Clamp,
    Smoothstep,
    Vec2,
    Vec3,
    Vec4,
    Dot,
    Length,
    Normalize,
    Sin,
    Cos,
    Sqrt,
    Deg,
    Map,
    FlatMap,
    Assert,
    Len,
    Get,
    Any,
    All,
    Filter,
    Fold,
    GroupBy,
    FoldBy,
    Arena,
    ArenaRun,
    ArenaStats,
    JsonParse,
    JsonStringify,
    Trim,
    TrimStart,
    TrimEnd,
    Split,
    Join,
    StartsWith,
    EndsWith,
    Contains,
    Replace,
    ToLower,
    ToUpper,
}

impl Builtin {
    /// Callback argument index and number of values supplied per invocation.
    pub fn callback_arities(self) -> &'static [(usize, usize)] {
        match self {
            Self::Map | Self::Filter | Self::FlatMap | Self::Any | Self::All | Self::GroupBy => {
                &[(1, 1)]
            }
            Self::Fold => &[(2, 2)],
            Self::FoldBy => &[(1, 1), (3, 2)],
            Self::GridMesh => &[(2, 2)],
            Self::ArenaRun => &[(1, 0)],
            _ => &[],
        }
    }
    /// Inclusive accepted argument-count range, shared by runtime and analysis.
    pub fn arity(self) -> std::ops::RangeInclusive<usize> {
        let (min, max) = match self {
            Self::None | Self::Identity => (0, 0),
            Self::Degrees
            | Self::Iter
            | Self::Len
            | Self::Promote
            | Self::Radians
            | Self::Some
            | Self::Ok
            | Self::Err
            | Self::RotationMatrix
            | Self::Translation
            | Self::Scaling
            | Self::RotationX
            | Self::RotationY
            | Self::RotationZ
            | Self::Polygon
            | Self::Length
            | Self::Normalize
            | Self::Sin
            | Self::Cos
            | Self::Sqrt
            | Self::JsonParse
            | Self::JsonStringify
            | Self::Deg
            | Self::Trim
            | Self::TrimStart
            | Self::TrimEnd
            | Self::ToLower
            | Self::ToUpper => (1, 1),
            Self::Range | Self::RangeIter => (2, 3),
            Self::Assert | Self::Arena => (1, 2),
            Self::ArenaStats => (1, 1),
            Self::ArenaRun => (2, 2),
            Self::GridMesh
            | Self::Slerp
            | Self::Lerp
            | Self::Clamp
            | Self::Smoothstep
            | Self::Vec3
            | Self::Replace
            | Self::Fold => (3, 3),
            Self::Vec4 | Self::FoldBy => (4, 4),
            Self::Random
            | Self::Collect
            | Self::Any
            | Self::All
            | Self::Get
            | Self::Mesh
            | Self::Noise
            | Self::Transform
            | Self::AxisAngle
            | Self::Zip
            | Self::TransformPoint
            | Self::TransformDirection
            | Self::Translate
            | Self::Rotate
            | Self::Cross
            | Self::Vec2
            | Self::Dot
            | Self::Map
            | Self::GroupBy
            | Self::FlatMap
            | Self::Split
            | Self::Join
            | Self::StartsWith
            | Self::EndsWith
            | Self::Contains
            | Self::Filter => (2, 2),
        };
        min..=max
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Closure<'s> {
    module: Option<&'s str>,
    parameters: Vec<Expr<'s>>,
    parameter_types: Vec<Option<ValueType>>,
    result_type: Option<ValueType>,
    /// Shared per program point; lambda evaluation no longer deep-clones AST.
    body: Rc<FunctionBody<'s>>,
    name: Option<&'s str>,
    environment: memory::Shared<Environment<'s>>,
    references: Rc<Vec<CaptureReference<'s>>>,
}
impl<'s> Closure<'s> {
    pub fn name(&self) -> Option<&'s str> {
        self.name
    }
}

#[derive(Clone, Debug, PartialEq)]
enum FunctionBody<'s> {
    Constructor {
        type_name: String,
        variant: Option<String>,
        fields: Vec<(String, ValueType)>,
    },
    Expression(Expr<'s>),
    Block(Block<'s>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeError {
    /// None identifies the entry program; Some identifies a registered module.
    pub module: Option<String>,
    pub span: Span,
    pub message: String,
    /// Innermost frame first; positions are one-based UTF-8 character columns.
    pub stack: Vec<CallFrame>,
    pub location: Option<SourceLocation>,
}

/// A callable and the source position at which it was invoked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFrame {
    pub function: String,
    pub module: Option<String>,
    pub span: Span,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}
impl RuntimeError {
    fn locate(mut self, source: &str) -> Self {
        let prefix = source.get(..self.span.start).unwrap_or("");
        self.location = Some(SourceLocation {
            line: prefix.bytes().filter(|b| *b == b'\n').count() + 1,
            column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        });
        self
    }
}

type Result<T> = std::result::Result<T, RuntimeError>;

/// A reusable cancellation signal. Once cancelled, use a new token for a new run.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Per-execution limits. These do not bound heap memory or blocking host calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub steps: usize,
    /// Evaluation nesting, including expressions, function bodies and imports.
    /// Values above 64 are rejected to preserve the native stack safety ceiling.
    pub max_depth: usize,
    /// Bound for collection literals, eager transforms and geometry construction.
    /// Also checks host results/items after allocation; does not bound total memory.
    pub max_collection_items: usize,
    /// UTF-8 bytes in decoded string literals, concatenation results and record keys.
    /// Includes strings in host results/items; allocations inside host calls are not bounded.
    pub max_string_bytes: usize,
}
impl ExecutionLimits {
    pub const fn new(steps: usize) -> Self {
        Self {
            steps,
            max_depth: 64,
            max_collection_items: usize::MAX,
            max_string_bytes: usize::MAX,
        }
    }
}

// Integer operands within the exact f64 range can avoid libm fmod. The
// remainder has the dividend's sign, including zero; other operands use fmod.
pub(crate) fn exact_remainder(a: f64, b: f64) -> f64 {
    const EXACT_INTEGER: f64 = 9_007_199_254_740_991.;
    if a.abs() <= EXACT_INTEGER
        && b.abs() <= EXACT_INTEGER
        && b != 0.
        && a.trunc() == a
        && b.trunc() == b
    {
        ((a as i64 % b as i64) as f64).copysign(a)
    } else {
        a % b
    }
}

/// Runtime contracts for functions registered by the embedding application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
    Function(Vec<ValueType>, Box<ValueType>),
    User(String),
    HostObject(&'static str),
    Angle,
    Option(Box<ValueType>),
    Result(Box<ValueType>, Box<ValueType>),
    Tuple(Vec<ValueType>),
    Mesh,
    Quaternion,
    String,
    Matrix4,
    Number,
    Bool,
    Vector(usize),
    Polygon,
    List(Box<ValueType>),
    Sequence,
    Null,
}
impl ValueType {
    pub(crate) fn annotation(ty: &crate::Type<'_>) -> Result<Self> {
        Self::annotation_with(ty, &|_| None)
    }
    pub(crate) fn annotation_with(
        ty: &crate::Type<'_>,
        resolve: &impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let qualified = ty.qualified_name();
        if ty.arguments.is_empty()
            && let Some(name) = resolve(&qualified)
        {
            return Ok(Self::User(name));
        }
        let primitive = match qualified.as_ref() {
            "angle" => Some(Self::Angle),
            "number" | "f64" | "float" => Some(Self::Number),
            "bool" => Some(Self::Bool),
            "vec2" => Some(Self::Vector(2)),
            "vec3" => Some(Self::Vector(3)),
            "vec4" => Some(Self::Vector(4)),
            "polygon" => Some(Self::Polygon),
            "str" | "string" => Some(Self::String),
            "quat" => Some(Self::Quaternion),
            "mesh" => Some(Self::Mesh),
            "mat4" => Some(Self::Matrix4),
            "null" => Some(Self::Null),
            "sequence" => Some(Self::Sequence),
            _ => None,
        };
        if ty.arguments.is_empty()
            && let Some(primitive) = primitive
        {
            return Ok(primitive);
        }
        if ty.name.text == "Fn"
            && ty.arguments.len() == 2
            && let Self::Tuple(parameters) = Self::annotation_with(&ty.arguments[0], resolve)?
        {
            return Ok(Self::Function(
                parameters,
                Box::new(Self::annotation_with(&ty.arguments[1], resolve)?),
            ));
        }
        if ty.name.text == "Option" && ty.arguments.len() == 1 {
            return Ok(Self::Option(Box::new(Self::annotation_with(
                &ty.arguments[0],
                resolve,
            )?)));
        }
        if ty.name.text == "Result" && ty.arguments.len() == 2 {
            return Ok(Self::Result(
                Box::new(Self::annotation_with(&ty.arguments[0], resolve)?),
                Box::new(Self::annotation_with(&ty.arguments[1], resolve)?),
            ));
        }
        if ty.name.text == "tuple" {
            return Ok(Self::Tuple(
                ty.arguments
                    .iter()
                    .map(|ty| Self::annotation_with(ty, resolve))
                    .collect::<Result<Vec<_>>>()?,
            ));
        }
        if ty.name.text == "list" && ty.arguments.len() == 1 {
            return Ok(Self::List(Box::new(Self::annotation_with(
                &ty.arguments[0],
                resolve,
            )?)));
        }
        Err(RuntimeError {
            stack: Vec::new(),
            location: None,
            module: None,
            span: ty.name.span,
            message: format!("Unsupported type annotation: {}", ty.name.text),
        })
    }
    fn inferred(value: &Value<'_>) -> Option<Self> {
        Some(match value {
            Value::Number(_) => Self::Number,
            Value::Bool(_) => Self::Bool,
            Value::String(_) => Self::String,
            Value::Angle(_) => Self::Angle,
            Value::Vector(v) => Self::Vector(v.len()),
            Value::Matrix(_) => Self::Matrix4,
            Value::Quaternion(_) => Self::Quaternion,
            Value::Mesh(_) => Self::Mesh,
            Value::Polygon(_) => Self::Polygon,
            Value::UserData(data) => Self::User(data.type_name.clone()),
            _ => return None,
        })
    }
    pub fn accepts(&self, value: &Value<'_>) -> bool {
        match (self, value) {
            (Self::Function(parameters, result), Value::Function(function)) => {
                function.parameter_types.len() == parameters.len()
                    && function
                        .parameter_types
                        .iter()
                        .zip(parameters)
                        .all(|(actual, expected)| actual.as_ref() == Some(expected))
                    && function.result_type.as_ref() == Some(result.as_ref())
            }
            (Self::Function(parameters, _), Value::BytecodeFunction(function)) => {
                function.function.arity == parameters.len()
            }
            (Self::Function(parameters, result), Value::Host(function)) => {
                &function.parameters == parameters && &function.result == result.as_ref()
            }
            (Self::User(name), Value::UserData(data)) => name == &data.type_name,
            (Self::HostObject(name), Value::HostObject(object)) => {
                *name == object.type_name() && object.is_alive()
            }
            (Self::Sequence, Value::Sequence(_) | Value::Range { .. }) => true,
            (Self::Angle, Value::Angle(angle)) => angle.is_finite(),
            (Self::Option(_), Value::Variant("None", values)) => values.is_empty(),
            (Self::Option(ty), Value::Variant("Some", values)) => {
                values.len() == 1 && ty.accepts(&values[0])
            }
            (Self::Result(ty, _), Value::Variant("Ok", values))
            | (Self::Result(_, ty), Value::Variant("Err", values)) => {
                values.len() == 1 && ty.accepts(&values[0])
            }
            (Self::Tuple(types), Value::Tuple(values)) => {
                types.len() == values.len()
                    && types
                        .iter()
                        .zip(values)
                        .all(|(ty, value)| ty.accepts(value))
            }
            (Self::Mesh, Value::Mesh(_)) => true,
            (Self::Quaternion, Value::Quaternion(_)) => true,
            (Self::Matrix4, Value::Matrix(matrix)) => {
                matrix.rows().iter().flatten().all(|n| n.is_finite())
            }
            (Self::String, Value::String(_)) => true,
            (Self::Number, Value::Number(n)) => n.is_finite(),
            (Self::Bool, Value::Bool(_))
            | (Self::Null, Value::Null)
            | (Self::Polygon, Value::Polygon(_)) => true,
            (Self::Vector(size), Value::Vector(values)) => {
                values.len() == *size && values.iter().all(|n| n.is_finite())
            }
            (Self::List(element), Value::List(values)) => values.iter().all(|v| element.accepts(v)),
            _ => false,
        }
    }
}

pub type HostCallback =
    for<'s> fn(&[Value<'s>], &CancellationToken) -> std::result::Result<Value<'s>, String>;
type ContextualHostCallback =
    Rc<dyn for<'v> Fn(&[Value<'v>], &CancellationToken) -> std::result::Result<Value<'v>, String>>;
#[derive(Debug)]
pub struct HostFunction {
    pub name: &'static str,
    pub parameters: Vec<ValueType>,
    pub result: ValueType,
    pub callback: HostCallback,
}
impl PartialEq for HostFunction {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// A host function with captured scene or command-queue state. Use interior mutability
/// (for example `Rc<RefCell<Scene>>`) when the callback changes its context.
#[derive(Clone)]
pub struct HostRegistration {
    pub function: Rc<HostFunction>,
    callback: ContextualHostCallback,
}
impl HostRegistration {
    pub fn new<F>(
        name: &'static str,
        parameters: Vec<ValueType>,
        result: ValueType,
        callback: F,
    ) -> Self
    where
        F: for<'v> Fn(&[Value<'v>], &CancellationToken) -> std::result::Result<Value<'v>, String>
            + 'static,
    {
        fn placeholder<'v>(
            _: &[Value<'v>],
            _: &CancellationToken,
        ) -> std::result::Result<Value<'v>, String> {
            Err("Context host function is not registered in this execution".into())
        }
        Self {
            function: Rc::new(HostFunction {
                name,
                parameters,
                result,
                callback: placeholder,
            }),
            callback: Rc::new(callback),
        }
    }
}

/// One initialized script. Mutable captures and module caches survive calls.
/// Source and cancellation token must outlive the instance. Errors do not roll back mutations.
pub struct ScriptInstance<'a, 's> {
    runtime: Runtime<'a, 's>,
    environment: Environment<'s>,
    span: Span,
    initial_value: Value<'s>,
}
impl<'a, 's> ScriptInstance<'a, 's> {
    pub fn initial_value(&self) -> &Value<'s> {
        &self.initial_value
    }
    pub fn get(&self, name: &str) -> Option<Value<'s>> {
        match self.environment.get(name)? {
            Binding::Value(value) => Some(value.clone()),
            Binding::Cell(cell) => Some(self.runtime.cells[cell.index].0.clone()),
        }
    }
    /// Invoke a named callable with a fresh execution budget and the instance's token.
    pub fn call(
        &mut self,
        name: &str,
        arguments: &[Value<'s>],
        limits: ExecutionLimits,
    ) -> Result<Value<'s>> {
        self.runtime.reclaim_cells();
        if limits.max_depth > 64 {
            return self
                .runtime
                .error(self.span, "Maximum evaluation depth cannot exceed 64");
        }
        self.runtime.remaining = limits.steps;
        self.runtime.max_depth = limits.max_depth;
        self.runtime.max_collection_items = limits.max_collection_items;
        self.runtime.max_string_bytes = limits.max_string_bytes;
        self.runtime.check(self.span)?;
        let function = self.get(name).ok_or_else(|| RuntimeError {
            stack: Vec::new(),
            location: None,
            module: None,
            span: self.span,
            message: format!("Unknown function: {name}"),
        })?;
        for value in arguments {
            self.runtime.host_value_size(value, self.span, 0)?;
        }
        self.enforce_memory_limit()?;
        let value = self
            .runtime
            .call(function, Cow::Borrowed(arguments), self.span)?;
        self.enforce_memory_limit()?;
        Ok(value)
    }

    /// Incrementally execute top-level statements in this instance's environment.
    /// Useful for REPL and interactive notebook sessions.
    pub fn eval_chunk(&mut self, source: &'s str) -> Result<Option<Value<'s>>> {
        let mut parsed = crate::parse(source);
        let (references, contracts) = crate::analysis::compile_analysis(&mut parsed);
        if !parsed.is_valid() {
            let diagnostic = parsed.diagnostics.first();
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: diagnostic.map_or(parsed.module.span, |d| d.span),
                message: diagnostic.map_or_else(
                    || "Invalid Rush program".into(),
                    |d| format!("{}: {}", d.code, d.message),
                ),
            }
            .locate(source));
        }
        if parsed.module.items.is_empty() {
            return Ok(None);
        }
        let mut capture_refs: Vec<_> = references
            .into_iter()
            .map(|reference| CaptureReference {
                name: &source[reference.usage.start..reference.usage.end],
                usage: reference.usage,
                definition: reference.definition,
            })
            .collect();
        capture_refs.sort_by_key(|reference| reference.usage.start);
        self.runtime.current_references = Rc::new(capture_refs);
        if let Some(contracts_map) = self.runtime.contracts.get_mut(&None) {
            if let Some(cm) = Rc::get_mut(contracts_map) {
                cm.extend(contracts);
            } else {
                let mut new_contracts = (**contracts_map).clone();
                new_contracts.extend(contracts);
                *contracts_map = Rc::new(new_contracts);
            }
        }
        self.runtime.sources.insert(None, source);
        self.runtime.remaining = 1_000_000;
        self.runtime.check(parsed.module.span)?;

        let is_expr_or_show = matches!(
            parsed.module.items.last().map(|s| &s.kind),
            Some(StmtKind::Expr(_)) | Some(StmtKind::Show(_))
        );

        let (val, _) = self
            .runtime
            .statements(&parsed.module.items, &mut self.environment)
            .map_err(|e| e.locate(source))?;

        if is_expr_or_show {
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// List active user-defined variables and their values.
    pub fn list_variables(&self) -> Vec<(&str, Value<'s>)> {
        let mut list = Vec::new();
        for (name, binding) in self.environment.bindings.iter() {
            if builtin_catalog().iter().any(|(b, _)| b == name) {
                continue;
            }
            let val = match binding {
                Binding::Value(v) => v.clone(),
                Binding::Cell(cell) => {
                    if let Ok(idx) = self.runtime.cell_index(cell, Span::new(0, 0)) {
                        self.runtime.cells[idx].0.clone()
                    } else {
                        continue;
                    }
                }
            };
            list.push((*name, val));
        }
        list
    }
}

/// An analyzed program that can be evaluated repeatedly without reparsing.
pub struct Program<'s> {
    imports: Vec<Name<'s>>,
    contracts: Rc<HashMap<usize, crate::analysis::FunctionContract>>,
    parsed: crate::Parse<'s>,
    references: Rc<Vec<CaptureReference<'s>>>,
}
struct PreparedModules {
    interfaces: crate::analysis::ModuleInterfaces,
    root_contracts: Rc<HashMap<usize, crate::analysis::FunctionContract>>,
}
#[derive(Clone, Debug, PartialEq)]
struct CaptureReference<'s> {
    name: &'s str,
    usage: Span,
    definition: Option<Span>,
}
impl<'s> Program<'s> {
    /// Explicit public interface. None preserves the legacy returned-record interface.
    pub fn exports(&self) -> Option<Vec<Name<'s>>> {
        let mut found = false;
        let mut names = Vec::new();
        for statement in &self.parsed.module.items {
            if let StmtKind::Export(exports) = &statement.kind {
                found = true;
                names.extend(exports.iter().cloned());
            }
        }
        found.then_some(names)
    }
    /// All syntactic imports, including imports inside function and branch bodies.
    pub fn imports(&self) -> Vec<Name<'s>> {
        self.imports.clone()
    }
    pub(crate) fn import_refs(&self) -> &[Name<'s>] {
        &self.imports
    }
    fn collect_imports(statements: &[Stmt<'s>]) -> Vec<Name<'s>> {
        let mut imports = Vec::new();
        let mut pending = vec![statements];
        while let Some(statements) = pending.pop() {
            for statement in statements {
                match &statement.kind {
                    StmtKind::Import(name) => imports.push(name.clone()),
                    StmtKind::Function { body, .. }
                    | StmtKind::For { body, .. }
                    | StmtKind::While { body, .. } => pending.push(&body.stmts),
                    StmtKind::If {
                        then_block,
                        else_block,
                        ..
                    } => {
                        pending.push(&then_block.stmts);
                        if let Some(block) = else_block {
                            pending.push(&block.stmts);
                        }
                    }
                    _ => {}
                }
            }
        }
        imports
    }
    /// Validate the reachable module graph without running any module code.
    pub fn validate_modules(&self, modules: &[(&str, &Program<'_>)]) -> Result<()> {
        self.prepare_modules(modules).map(|_| ())
    }
    fn prepare_modules(&self, modules: &[(&str, &Program<'_>)]) -> Result<PreparedModules> {
        if modules.is_empty() && self.imports.is_empty() {
            return Ok(PreparedModules {
                interfaces: HashMap::new(),
                root_contracts: self.contracts.clone(),
            });
        }
        let mut registry = HashMap::new();
        for (name, program) in modules {
            if registry.insert(*name, *program).is_some() {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: None,
                    span: self.parsed.module.span,
                    message: format!("Duplicate module: {name}"),
                });
            }
        }
        let mut done = std::collections::HashSet::new();
        let mut active = std::collections::HashSet::new();
        let mut pending: Vec<(Option<&str>, &[Name<'_>], usize)> =
            vec![(None, self.import_refs(), 0)];
        while let Some((module, imports, index)) = pending.last_mut() {
            if *index == imports.len() {
                if let Some(name) = module {
                    active.remove(name);
                    done.insert(*name);
                }
                pending.pop();
                continue;
            }
            let import = &imports[*index];
            *index += 1;
            let Some(program) = registry.get(import.text) else {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: module.map(str::to_owned),
                    span: import.span,
                    message: format!("Unknown module: {}", import.text),
                });
            };
            if active.contains(import.text) {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: module.map(str::to_owned),
                    span: import.span,
                    message: format!("Cyclic module import: {}", import.text),
                });
            }
            if done.contains(import.text) {
                continue;
            }
            active.insert(import.text);
            let name = import.text;
            pending.push((Some(name), program.import_refs(), 0));
        }
        let reachable: Vec<_> = modules
            .iter()
            .copied()
            .filter(|(name, _)| done.contains(name))
            .collect();
        let interfaces = crate::analysis::module_interfaces(&reachable);
        let root = if self.imports.is_empty() {
            None
        } else {
            Some(crate::analysis::resolved_interface(
                &mut self.parsed.clone(),
                interfaces.clone(),
                None,
            ))
        };
        for (name, source, diagnostics) in std::iter::once((
            None,
            self.parsed.source,
            root.as_ref()
                .map(|i| i.diagnostics.as_slice())
                .unwrap_or(&[]),
        ))
        .chain(reachable.iter().map(|(name, program)| {
            (
                Some(*name),
                program.parsed.source,
                interfaces[*name].diagnostics.as_slice(),
            )
        })) {
            if let Some(d) = diagnostics.first() {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: name.map(str::to_owned),
                    span: d.span,
                    message: format!("{}: {}", d.code, d.message),
                }
                .locate(source));
            }
        }
        Ok(PreparedModules {
            root_contracts: root.map_or_else(|| self.contracts.clone(), |i| Rc::new(i.contracts())),
            interfaces,
        })
    }

    /// Require complete named-function contracts with the registered module interfaces available.
    pub fn validate_strict_modules(&self, modules: &[(&str, &Program<'_>)]) -> Result<()> {
        self.validate_modules(modules)?;
        let interfaces = crate::analysis::module_interfaces(modules);
        for (name, program) in
            std::iter::once((None, self)).chain(modules.iter().map(|(n, p)| (Some(*n), *p)))
        {
            let mut parsed = program.parsed.clone();
            crate::analysis::check_strict_modules(&mut parsed, interfaces.clone(), name);
            if let Some(d) = parsed.diagnostics.first() {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: name.map(str::to_owned),
                    span: d.span,
                    message: format!("{}: {}", d.code, d.message),
                }
                .locate(parsed.source));
            }
        }
        Ok(())
    }
    /// Compile with complete contracts for every named function. Ambiguous dynamic boundaries require annotations.
    pub fn compile_strict(source: &'s str) -> Result<Self> {
        let program = Self::compile(source)?;
        let mut parsed = program.parsed.clone();
        crate::analysis::check_strict(&mut parsed);
        if let Some(d) = parsed.diagnostics.first() {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: d.span,
                message: format!("{}: {}", d.code, d.message),
            }
            .locate(source));
        }
        Ok(program)
    }
    pub(crate) fn parsed_clone(&self) -> crate::Parse<'s> {
        self.parsed.clone()
    }
    pub fn compile(source: &'s str) -> Result<Self> {
        Self::compile_parsed(crate::parse(source))
    }
    fn compile_parsed(mut parsed: crate::Parse<'s>) -> Result<Self> {
        let source = parsed.source;
        let (references, contracts) = crate::analysis::compile_analysis(&mut parsed);
        if !parsed.is_valid() {
            let diagnostic = parsed.diagnostics.first();
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: diagnostic.map_or(parsed.module.span, |d| d.span),
                message: diagnostic.map_or_else(
                    || "Invalid Rush program".into(),
                    |d| format!("{}: {}", d.code, d.message),
                ),
            }
            .locate(source));
        }
        let mut references: Vec<_> = references
            .into_iter()
            .map(|reference| CaptureReference {
                name: &source[reference.usage.start..reference.usage.end],
                usage: reference.usage,
                definition: reference.definition,
            })
            .collect();
        references.sort_by_key(|reference| reference.usage.start);
        Ok(Self {
            imports: Self::collect_imports(&parsed.module.items),
            parsed,
            references: Rc::new(references),
            contracts: Rc::new(contracts),
        })
    }
    /// Numeric inputs permit parameter/time updates without changing the source.
    pub fn run(
        &self,
        budget: usize,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
    ) -> Result<Value<'s>> {
        self.run_with_host(budget, cancellation, inputs, &[])
    }
    /// Register host functions using the same callable values as Rush functions.
    pub fn run_with_host(
        &self,
        budget: usize,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
        functions: &[Rc<HostFunction>],
    ) -> Result<Value<'s>> {
        self.run_with_modules(budget, cancellation, inputs, functions, &[])
    }
    /// Modules are supplied explicitly; each module's final value must be a record.
    pub fn run_with_modules(
        &self,
        budget: usize,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
        functions: &[Rc<HostFunction>],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<Value<'s>> {
        self.run_with_limits(
            ExecutionLimits::new(budget),
            cancellation,
            inputs,
            functions,
            modules,
        )
    }
    /// Execute with explicit step, evaluation-depth and data-size limits, shared by all modules
    /// and Rush function calls. Existing run methods use a depth ceiling of 64.
    pub fn run_with_limits(
        &self,
        limits: ExecutionLimits,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
        functions: &[Rc<HostFunction>],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<Value<'s>> {
        if inputs.iter().any(|(_, value)| !value.is_finite()) {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: self.parsed.module.span,
                message: "Invalid or duplicate input parameter".into(),
            });
        }
        let values: Vec<_> = inputs
            .iter()
            .map(|(name, value)| (*name, Value::Number(*value)))
            .collect();
        self.run_with_values(limits, cancellation, &values, functions, modules)
    }
    /// Execute once with arbitrary values, including host objects and event records.
    pub fn run_with_values(
        &self,
        limits: ExecutionLimits,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, Value<'s>)],
        functions: &[Rc<HostFunction>],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<Value<'s>> {
        Ok(self
            .instantiate(limits, cancellation, inputs, functions, &[], modules)?
            .initial_value)
    }
    /// Initialize top-level code once. Context registrations share the ordinary host signatures.
    pub fn instantiate<'a>(
        &self,
        limits: ExecutionLimits,
        cancellation: &'a CancellationToken,
        inputs: &[(&'s str, Value<'s>)],
        functions: &[Rc<HostFunction>],
        contextual: &[HostRegistration],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<ScriptInstance<'a, 's>> {
        let prepared = self.prepare_modules(modules)?;
        if !functions.is_empty() || !contextual.is_empty() {
            let hosts: HashMap<_, _> = functions
                .iter()
                .cloned()
                .chain(
                    contextual
                        .iter()
                        .map(|registration| registration.function.clone()),
                )
                .map(|function| (function.name.to_owned(), function))
                .collect();
            for (name, program) in std::iter::once((None, self)).chain(
                modules
                    .iter()
                    .map(|(name, program)| (Some(*name), *program)),
            ) {
                let mut parsed = program.parsed.clone();
                crate::analysis::check_host_calls(
                    &mut parsed,
                    crate::InputLimits::conservative().max_diagnostics,
                    true,
                    hosts.clone(),
                );
                // Errors are fatal; non-fatal lints (e.g. region-escape
                // warnings) must not block a valid program.
                if !parsed.is_valid() {
                    let diagnostic = parsed.diagnostics.first();
                    return Err(RuntimeError {
                        module: name.map(str::to_owned),
                        span: diagnostic.map_or(parsed.module.span, |d| d.span),
                        message: diagnostic.map_or_else(
                            || "Invalid Rush program".into(),
                            |d| format!("{}: {}", d.code, d.message),
                        ),
                        stack: Vec::new(),
                        location: None,
                    }
                    .locate(parsed.source));
                }
            }
        }
        if limits.max_depth > 64 {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: self.parsed.module.span,
                message: "Maximum evaluation depth cannot exceed 64".into(),
            });
        }
        let root_contracts = prepared.root_contracts;
        let memory = memory::Budget::new(usize::MAX);
        let mut runtime = Runtime {
            module: None,
            early_return: None,
            coroutines: HashMap::new(),
            user_types: HashMap::new(),
            modules: HashMap::new(),
            module_cache: memory::Slots::new(&memory, 0)
                .expect("empty module cache requires no allocation"),
            loading: memory::Slots::new(&memory, 0)
                .expect("empty module stack requires no allocation"),
            module_globals: Environment::new(&memory),
            memory: memory.clone(),
            contracts: HashMap::from([(None, root_contracts)]),
            references: HashMap::from([(None, self.references.clone())]),
            current_references: self.references.clone(),
            cells: memory::Slots::new(&memory, 0)
                .expect("empty cell storage requires no allocation"),
            released_cells: Rc::new(RefCell::new(
                memory::Slots::new(&memory, 0).expect("empty release queue requires no allocation"),
            )),
            free_cells: memory::Slots::new(&memory, 0)
                .expect("empty free cell storage requires no allocation"),
            cell_ids: memory::Slots::new(&memory, 0)
                .expect("empty cell ID storage requires no allocation"),
            cell_allocations: 0,
            cell_collection_interval: 64,
            remaining: limits.steps,
            max_depth: limits.max_depth,
            max_collection_items: limits.max_collection_items,
            max_string_bytes: limits.max_string_bytes,
            depth: 0,
            sources: HashMap::from([(None, self.parsed.source)]),
            memory_limit: usize::MAX,
            initial_data_bytes: 0,
            instance_roots: None,
            environment_pool: Vec::new(),
            function_bodies: HashMap::new(),
            region_stack: Vec::new(),
            cell_regions: Vec::new(),
            next_region: 0,
            skip_call_region: false,
            region_stats: Vec::new(),
            cancellation,
            contextual: contextual
                .iter()
                .map(|entry| (Rc::as_ptr(&entry.function), entry.callback.clone()))
                .collect(),
        };
        runtime.check(self.parsed.module.span)?;
        let mut environment = Environment::new(&memory);
        for &(name, builtin) in builtin_catalog() {
            environment
                .insert(name, Value::Builtin(builtin))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        for function in functions
            .iter()
            .chain(contextual.iter().map(|entry| &entry.function))
        {
            if environment.contains_key(function.name) {
                return runtime.error(self.parsed.module.span, "Duplicate host function name");
            }
            environment
                .insert(function.name, Value::Host(function.clone()))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        for (name, value) in inputs {
            runtime.host_value_size(value, self.parsed.module.span, 0)?;
            if environment.contains_key(name) {
                return runtime.error(
                    self.parsed.module.span,
                    "Invalid or duplicate input parameter",
                );
            }
            environment
                .insert(name, value.clone())
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        runtime.module_globals = environment
            .try_clone()
            .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        for (name, program) in modules {
            runtime.sources.insert(Some(*name), program.parsed.source);
            if runtime
                .modules
                .insert(*name, program.parsed.module.clone())
                .is_some()
            {
                return runtime.error(self.parsed.module.span, "Duplicate module registration");
            }
            runtime.contracts.insert(
                Some(*name),
                prepared
                    .interfaces
                    .get(*name)
                    .map_or_else(|| program.contracts.clone(), |i| Rc::new(i.contracts())),
            );
            runtime
                .references
                .insert(Some(*name), program.references.clone());
        }
        let (initial_value, _) =
            runtime.module_statements(&self.parsed.module.items, &mut environment)?;
        runtime.instance_roots = Some(
            environment
                .try_clone()
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?,
        );
        let mut instance = ScriptInstance {
            runtime,
            environment,
            span: self.parsed.module.span,
            initial_value,
        };
        instance.initialize_usage();
        Ok(instance)
    }
}

/// Evaluate a module's final expression with a shared expression/call budget.
pub fn evaluate(source: &str, budget: usize) -> Result<Value<'_>> {
    Program::compile(source)?.run(budget, &CancellationToken::default(), &[])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    Next,
    Return,
    Break,
    Continue,
}

struct Runtime<'a, 's> {
    module: Option<&'s str>,
    early_return: Option<Value<'s>>,
    coroutines: HashMap<CoroutineId, coroutine::Coroutine<'s>>,
    user_types: HashMap<(Option<&'s str>, &'s str), String>,
    modules: HashMap<&'s str, crate::Module<'s>>,
    module_cache: memory::Slots<(&'s str, Value<'s>)>,
    loading: memory::Slots<&'s str>,
    module_globals: Environment<'s>,
    memory: memory::Budget,
    references: HashMap<Option<&'s str>, Rc<Vec<CaptureReference<'s>>>>,
    contracts: HashMap<Option<&'s str>, Rc<HashMap<usize, crate::analysis::FunctionContract>>>,
    current_references: Rc<Vec<CaptureReference<'s>>>,
    cells: memory::Slots<(Value<'s>, Option<ValueType>)>,
    released_cells: Rc<RefCell<memory::Slots<usize>>>,
    free_cells: memory::Slots<usize>,
    cell_ids: memory::Slots<Weak<CellId>>,
    cell_allocations: usize,
    cell_collection_interval: usize,
    remaining: usize,
    depth: usize,
    max_depth: usize,
    max_collection_items: usize,
    max_string_bytes: usize,
    sources: HashMap<Option<&'s str>, &'s str>,
    memory_limit: usize,
    initial_data_bytes: usize,
    instance_roots: Option<Environment<'s>>,
    environment_pool: Vec<Environment<'s>>,
    /// Shared function bodies keyed by (module, expression start), built once
    /// per program point instead of deep-cloning AST on every evaluation.
    function_bodies: HashMap<(Option<&'s str>, usize), Rc<FunctionBody<'s>>>,
    /// Active region (arena) frames, innermost last. Region id 0 is the
    /// global arena and has no frame.
    region_stack: Vec<RegionFrame>,
    /// Region id per cell slot, parallel to `cells`.
    cell_regions: Vec<u32>,
    next_region: u32,
    /// One-shot: the next `call_user` runs without an implicit call region
    /// because `arena_run` already supplied the region for it.
    skip_call_region: bool,
    /// Aggregated per-name metrics; frames reference entries by index.
    /// Lookups are linear — the number of distinct region names is tiny.
    region_stats: Vec<RegionStats>,
    cancellation: &'a CancellationToken,
    contextual: HashMap<*const HostFunction, ContextualHostCallback>,
}
/// One active region (arena) frame.
#[derive(Clone, Debug)]
struct RegionFrame {
    id: u32,
    /// Live-cell ceiling; `None` for an unbounded region.
    budget: Option<usize>,
    /// Cells currently alive with this region's stamp.
    live: usize,
    /// Index into `Runtime::region_stats` for this region's name.
    stats: usize,
    /// Cell slots stamped to this region, so exit sweeps only its own cells
    /// instead of scanning the whole table. May hold stale entries (freed or
    /// re-stamped slots); the exit pass re-checks the stamp and liveness.
    cells: Vec<usize>,
}
/// Aggregated allocation metrics for one region name, or for all anonymous
/// regions under `name: None`. Counters accumulate across every entry into
/// the region, including inside coroutines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionStats {
    /// Region name as written in source; `None` aggregates anonymous blocks.
    pub name: Option<String>,
    /// How many times the region was entered.
    pub entries: u64,
    /// Total cell allocations stamped to the region.
    pub allocated: u64,
    /// Allocations that reused a previously freed cell slot.
    pub reused_slots: u64,
    /// Maximum simultaneously live cells observed; use it to size budgets.
    pub peak_live: usize,
    /// Cells promoted to the parent region because they were still reachable
    /// at region exit.
    pub promoted: u64,
}
impl RegionStats {
    /// Budget recommendation from the observed peak: the smallest power of
    /// two above `peak_live`, giving at least 2x headroom for load spikes.
    /// Treat it as a starting point, not a proof; rerun under realistic load.
    #[must_use]
    pub fn suggested_budget(&self) -> usize {
        (self.peak_live + 1).next_power_of_two()
    }
}

enum AccessStep<'s> {
    Index(Value<'s>, Span),
    Member(String, Span),
}

fn path_error(module: Option<&str>, span: Span, message: impl Into<String>) -> RuntimeError {
    RuntimeError {
        stack: Vec::new(),
        location: None,
        module: module.map(str::to_owned),
        span,
        message: message.into(),
    }
}

fn mutate_path<'s>(
    module: Option<&str>,
    current: &mut Value<'s>,
    steps: &[AccessStep<'s>],
    assigned: Value<'s>,
    span: Span,
) -> Result<()> {
    if steps.is_empty() {
        *current = assigned;
        return Ok(());
    }
    let (head, rest) = steps.split_first().unwrap();
    match head {
        AccessStep::Index(index_val, step_span) => match current {
            Value::List(items) => {
                let Value::Number(num) = index_val else {
                    return Err(path_error(module, *step_span, "Index must be an integer"));
                };
                if *num < 0.0 || num.fract() != 0.0 || *num >= items.len() as f64 {
                    return Err(path_error(module, *step_span, "Index out of bounds"));
                }
                let idx = *num as usize;
                if rest.is_empty() {
                    items[idx] = assigned;
                    Ok(())
                } else {
                    mutate_path(module, &mut items[idx], rest, assigned, span)
                }
            }
            Value::Record(fields) => {
                let Value::String(key) = index_val else {
                    return Err(path_error(
                        module,
                        *step_span,
                        "Record index must be a string",
                    ));
                };
                if rest.is_empty() {
                    fields.insert(key.clone(), assigned);
                    Ok(())
                } else {
                    let sub = fields.get_mut(key).ok_or_else(|| {
                        path_error(module, *step_span, format!("Unknown field: {key}"))
                    })?;
                    mutate_path(module, sub, rest, assigned, span)
                }
            }
            Value::Vector(components) => {
                let Value::Number(num) = index_val else {
                    return Err(path_error(module, *step_span, "Index must be an integer"));
                };
                if *num < 0.0 || num.fract() != 0.0 || *num >= components.len() as f64 {
                    return Err(path_error(module, *step_span, "Index out of bounds"));
                }
                let idx = *num as usize;
                if rest.is_empty() {
                    let Value::Number(val) = assigned else {
                        return Err(path_error(
                            module,
                            span,
                            "Vector component must be a number",
                        ));
                    };
                    components[idx] = val;
                    Ok(())
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Vector components do not support nested access",
                    ))
                }
            }
            _ => Err(path_error(
                module,
                *step_span,
                "Indexing requires a list, record or vector",
            )),
        },
        AccessStep::Member(field_name, step_span) => match current {
            Value::Record(fields) => {
                if rest.is_empty() {
                    fields.insert(field_name.clone(), assigned);
                    Ok(())
                } else {
                    let sub = fields.get_mut(field_name).ok_or_else(|| {
                        path_error(module, *step_span, format!("Unknown field: {field_name}"))
                    })?;
                    mutate_path(module, sub, rest, assigned, span)
                }
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first_mut() {
                    if rest.is_empty() {
                        if !fields.contains_key(field_name) {
                            return Err(path_error(
                                module,
                                *step_span,
                                format!("Unknown struct field: {field_name}"),
                            ));
                        }
                        fields.insert(field_name.clone(), assigned);
                        Ok(())
                    } else {
                        let sub = fields.get_mut(field_name).ok_or_else(|| {
                            path_error(
                                module,
                                *step_span,
                                format!("Unknown struct field: {field_name}"),
                            )
                        })?;
                        mutate_path(module, sub, rest, assigned, span)
                    }
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Member access requires a struct or record",
                    ))
                }
            }
            Value::Vector(components) => {
                let axis = match field_name.as_str() {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return Err(path_error(module, *step_span, "Unknown vector component")),
                };
                if axis >= components.len() {
                    return Err(path_error(
                        module,
                        *step_span,
                        "Vector component out of bounds",
                    ));
                }
                if rest.is_empty() {
                    let Value::Number(val) = assigned else {
                        return Err(path_error(
                            module,
                            span,
                            "Vector component must be a number",
                        ));
                    };
                    components[axis] = val;
                    Ok(())
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Vector components do not support nested access",
                    ))
                }
            }
            _ => Err(path_error(
                module,
                *step_span,
                "Member access requires a record, struct or vector",
            )),
        },
    }
}

impl<'s> Runtime<'_, 's> {
    fn cell_index(&self, cell: &Rc<CellId>, span: Span) -> Result<usize> {
        if cell.released.as_ptr() != Rc::as_ptr(&self.released_cells) {
            return self.error(span, "Mutable capture belongs to another execution");
        }
        if !self
            .cell_ids
            .get(cell.index)
            .is_some_and(|id| id.as_ptr() == Rc::as_ptr(cell))
        {
            return self.error(span, "Mutable capture is no longer available");
        }
        Ok(cell.index)
    }
    fn capture(
        &self,
        span: Span,
        environment: &Environment<'s>,
    ) -> Result<memory::Shared<Environment<'s>>> {
        let references = &self.current_references;
        let start = references.partition_point(|reference| reference.usage.start < span.start);
        let mut captured = Environment::new(&self.memory);
        for reference in &references[start..] {
            if reference.usage.start >= span.end {
                break;
            }
            if reference.definition.is_some_and(|definition| {
                definition.start >= span.start && definition.end <= span.end
            }) {
                continue;
            }
            if !captured.contains_key(reference.name)
                && let Some(binding) = environment.get(reference.name)
            {
                captured
                    .insert_binding(reference.name, binding.clone())
                    .map_err(|e| self.environment_error(span, e))?;
            }
        }
        memory::Shared::new(&self.memory, captured).map_err(|e| self.environment_error(span, e))
    }
    fn reclaim_cells(&mut self) {
        // Dropping a value can release captured bindings. Drain again without
        // holding a RefCell borrow across that drop.
        loop {
            let released = self.released_cells.borrow_mut().pop();
            let Some(index) = released else {
                break;
            };
            let region = self.cell_regions[index];
            if region != 0
                && let Some(frame) = self
                    .region_stack
                    .iter_mut()
                    .rev()
                    .find(|frame| frame.id == region)
            {
                frame.live = frame.live.saturating_sub(1);
            }
            self.cells[index] = (Value::Null, None);
            self.free_cells
                .push(index)
                .expect("free list reserved with cell");
        }
    }
    fn shared_body(
        &mut self,
        span: Span,
        build: impl FnOnce() -> FunctionBody<'s>,
    ) -> Rc<FunctionBody<'s>> {
        self.function_bodies
            .entry((self.module, span.start))
            .or_insert_with(|| Rc::new(build()))
            .clone()
    }
    /// Take a call environment from the pool or allocate a fresh child scope.
    fn take_call_environment(
        &mut self,
        parent: &memory::Shared<Environment<'s>>,
    ) -> Environment<'s> {
        match self.environment_pool.pop() {
            Some(mut environment) => {
                environment.reset_child(parent.clone());
                environment
            }
            None => Environment::child(parent.clone()),
        }
    }
    /// Return a call environment to the pool after releasing its bindings, so
    /// repeated closure calls reuse the allocated binding storage.
    fn recycle_call_environment(&mut self, mut environment: Environment<'s>) {
        const POOL_LIMIT: usize = 64;
        if self.environment_pool.len() < POOL_LIMIT {
            environment.release_bindings();
            environment.parent = None;
            self.environment_pool.push(environment);
        }
    }
    fn allocate_cell(
        &mut self,
        value: Value<'s>,
        contract: Option<ValueType>,
        span: Span,
    ) -> Result<Rc<CellId>> {
        self.reclaim_cells();
        self.enforce_cell_limit(&value, span)?;
        self.cell_allocations += 1;
        if self.cell_allocations >= self.cell_collection_interval {
            self.collect_cell_cycles(span)?;
            self.cell_allocations = 0;
            self.cell_collection_interval = (self.cells.len() - self.free_cells.len())
                .saturating_mul(2)
                .max(64);
        }
        let mut reused = true;
        let index = if let Some(index) = self.free_cells.pop() {
            self.cells[index] = (value, contract);
            index
        } else {
            reused = false;
            let index = self.cells.len();
            self.cells
                .reserve(1)
                .and_then(|()| self.cell_ids.reserve(1))
                .and_then(|()| self.free_cells.reserve(index + 1 - self.free_cells.len()))
                .and_then(|()| {
                    let mut released = self.released_cells.borrow_mut();
                    let additional = index + 1 - released.len();
                    released.reserve(additional)
                })
                .map_err(|error| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: format!("Runtime cell allocation failed: {error:?}"),
                })?;
            // Reserve every table, including reclamation queues, before
            // publishing a cell. Dropping a cell ID must never allocate.
            self.cells
                .push((value, contract))
                .expect("reserved cell slot");
            self.cell_ids
                .push(Weak::new())
                .expect("reserved cell ID slot");
            index
        };
        let id = Rc::new(CellId {
            index,
            released: Rc::downgrade(&self.released_cells),
        });
        self.cell_ids[index] = Rc::downgrade(&id);
        let region = self.region_stack.last().map_or(0, |frame| frame.id);
        if index == self.cell_regions.len() {
            self.cell_regions.push(region);
        } else {
            self.cell_regions[index] = region;
        }
        if let Some(frame) = self.region_stack.last_mut() {
            frame.live += 1;
            frame.cells.push(index);
            let stats = &mut self.region_stats[frame.stats];
            stats.allocated += 1;
            stats.reused_slots += reused as u64;
            stats.peak_live = stats.peak_live.max(frame.live);
            if frame.budget.is_some_and(|budget| frame.live > budget) {
                return self.error(span, "Region cell budget exceeded");
            }
        }
        Ok(id)
    }
    /// Parked coroutine regions: frames above `base` leave the shared stack
    /// while the task is suspended.
    fn take_regions_above(&mut self, base: usize) -> Vec<RegionFrame> {
        self.region_stack.split_off(base)
    }
    /// A resumed coroutine puts its parked region frames back on top.
    fn restore_regions(&mut self, frames: Vec<RegionFrame>) {
        self.region_stack.extend(frames);
    }
    /// Drop region frames of a terminated coroutine without sweeping; their
    /// cells are reclaimed through the normal release path.
    fn truncate_regions(&mut self, base: usize) {
        self.region_stack.truncate(base);
    }
    /// Evaluate an optional region budget expression in the parent region.
    fn eval_region_budget(
        &mut self,
        budget: Option<&Expr<'s>>,
        environment: &Environment<'s>,
    ) -> Result<Option<usize>> {
        let Some(expression) = budget else {
            return Ok(None);
        };
        match self.expr(expression, environment)? {
            Value::Number(number)
                if number.is_finite()
                    && number >= 0.0
                    && number.fract() == 0.0
                    && number <= usize::MAX as f64 =>
            {
                Ok(Some(number as usize))
            }
            _ => self.error(
                expression.span,
                "Region budget must be a non-negative integer",
            ),
        }
    }
    /// Open a region (arena) frame; cells allocated inside get its stamp and
    /// count towards the metrics of its name.
    fn enter_region(&mut self, budget: Option<usize>, name: Option<&str>) -> u32 {
        self.next_region = self.next_region.checked_add(1).expect("region id overflow");
        let region = self.next_region;
        let stats = match self
            .region_stats
            .iter()
            .position(|entry| entry.name.as_deref() == name)
        {
            Some(index) => index,
            None => {
                let index = self.region_stats.len();
                self.region_stats.push(RegionStats {
                    name: name.map(str::to_owned),
                    ..RegionStats::default()
                });
                index
            }
        };
        self.region_stats[stats].entries += 1;
        self.region_stack.push(RegionFrame {
            id: region,
            budget,
            live: 0,
            stats,
            cells: Vec::new(),
        });
        region
    }
    /// Close a region frame: cells no longer referenced after scope release
    /// are swept in bulk; escaping values promote to the parent region and
    /// count against its budget.
    fn exit_region(&mut self, region: u32, span: Span) -> Result<()> {
        let frame = self.region_stack.pop().expect("region frame");
        debug_assert_eq!(frame.id, region);
        self.reclaim_cells();
        let parent = self.region_stack.last().map_or(0, |frame| frame.id);
        let mut promoted = 0_usize;
        for index in frame.cells {
            // Stale entries belong to freed or re-stamped slots.
            if self.cell_regions.get(index).copied() != Some(region) {
                continue;
            }
            if let Some(id) = self.cell_ids[index].upgrade() {
                self.cell_regions[index] = parent;
                promoted += 1;
                if let Some(parent_frame) = self.region_stack.last_mut() {
                    parent_frame.cells.push(index);
                }
                drop(id);
            }
        }
        self.region_stats[frame.stats].promoted += promoted as u64;
        if promoted > 0
            && let Some(parent_frame) = self.region_stack.last_mut()
        {
            parent_frame.live += promoted;
            if parent_frame
                .budget
                .is_some_and(|budget| parent_frame.live > budget)
            {
                return self.error(span, "Region cell budget exceeded");
            }
        }
        Ok(())
    }
    fn host_value_size(&mut self, value: &Value<'s>, span: Span, depth: usize) -> Result<()> {
        if self.max_collection_items == usize::MAX && self.max_string_bytes == usize::MAX {
            return Ok(());
        }
        self.charge(1, span)?;
        if depth > 64 {
            return self.error(span, "Host value nesting limit exceeded");
        }
        match value {
            Value::String(text) => self.string_growth(0, text.len(), span)?,
            Value::List(items) | Value::Tuple(items) => {
                self.collection_growth(0, items.len(), span)?;
                for item in items {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            // Option/Result payloads have fixed arity, not collection length.
            // Their nested data still needs validation.
            Value::UserData(data) => {
                for item in &data.values {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            Value::Variant(_, items) => {
                for item in items {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            Value::Record(fields) => {
                self.collection_growth(0, fields.len(), span)?;
                for (key, value) in fields {
                    self.string_growth(0, key.len(), span)?;
                    self.host_value_size(value, span, depth + 1)?;
                }
            }
            Value::Polygon(polygon) => self.collection_growth(0, polygon.points().len(), span)?,
            Value::Mesh(mesh) => {
                self.collection_growth(0, mesh.vertices().len(), span)?;
                self.collection_growth(0, mesh.triangles().len(), span)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn string_growth(&self, current: usize, added: usize, span: Span) -> Result<()> {
        if current
            .checked_add(added)
            .is_none_or(|size| size > self.max_string_bytes)
        {
            return self.error(span, "String byte limit exceeded");
        }
        Ok(())
    }
    fn collection_growth(&self, current: usize, added: usize, span: Span) -> Result<()> {
        if current
            .checked_add(added)
            .is_none_or(|size| size > self.max_collection_items)
        {
            return self.error(span, "Collection item limit exceeded");
        }
        Ok(())
    }
    fn runtime_contract(&self, ty: &ValueType) -> ValueType {
        match ty {
            ValueType::User(name) if name.starts_with("<entry>::") => ValueType::User(format!(
                "{}::{}",
                self.module.unwrap_or("<entry>"),
                &name[9..]
            )),
            ValueType::Function(parameters, result) => ValueType::Function(
                parameters
                    .iter()
                    .map(|t| self.runtime_contract(t))
                    .collect(),
                Box::new(self.runtime_contract(result)),
            ),
            ValueType::List(t) => ValueType::List(Box::new(self.runtime_contract(t))),
            ValueType::Option(t) => ValueType::Option(Box::new(self.runtime_contract(t))),
            ValueType::Result(t, e) => ValueType::Result(
                Box::new(self.runtime_contract(t)),
                Box::new(self.runtime_contract(e)),
            ),
            ValueType::Tuple(items) => {
                ValueType::Tuple(items.iter().map(|t| self.runtime_contract(t)).collect())
            }
            other => other.clone(),
        }
    }
    fn annotation(&self, ty: &crate::Type<'_>) -> Result<ValueType> {
        ValueType::annotation_with(ty, &|name| {
            self.user_types
                .get(&(self.module, name))
                .cloned()
                .or_else(|| {
                    let (module, ty) = name.split_once('.')?;
                    let module = module.trim();
                    let ty = ty.trim();
                    let index = self
                        .module_cache
                        .binary_search_by_key(&module, |(name, _)| *name)
                        .ok()?;
                    let Value::Record(exports) = &self.module_cache[index].1 else {
                        return None;
                    };
                    let value = exports.get(ty)?;
                    let constructor = match value {
                        Value::Function(function) => Some(function),
                        Value::Record(variants) => variants.values().find_map(|v| {
                            if let Value::Function(f) = v {
                                Some(f)
                            } else {
                                None
                            }
                        }),
                        _ => None,
                    }?;
                    if let FunctionBody::Constructor { type_name, .. } = &*constructor.body {
                        Some(type_name.clone())
                    } else {
                        None
                    }
                })
        })
        .map_err(|mut error| {
            error.module = self.module.map(str::to_owned);
            error
        })
    }
    fn check(&self, span: Span) -> Result<()> {
        if self.cancellation.is_cancelled() {
            self.error(span, "Execution cancelled")
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, amount: usize, span: Span) -> Result<()> {
        self.check(span)?;
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Execution limit exceeded".into(),
            })?;
        Ok(())
    }
    fn sequence_cursor(&mut self, value: Value<'s>, span: Span) -> Result<SequenceCursor<'s>> {
        let sequence = match value {
            Value::Sequence(sequence) => sequence,
            Value::BytecodeSequence(seq) => {
                let source = match &seq.source {
                    crate::bytecode::BytecodeSequenceSource::Range { start, end, step } => {
                        SequenceSource::Range {
                            start: *start,
                            end: *end,
                            step: *step,
                        }
                    }
                    crate::bytecode::BytecodeSequenceSource::List(items) => SequenceSource::List(
                        memory::Buffer::from_iter(&self.memory, items.iter().cloned()).map_err(
                            |e| RuntimeError {
                                stack: Vec::new(),
                                location: None,
                                module: self.module.map(str::to_owned),
                                span,
                                message: format!(
                                    "Runtime sequence source allocation failed: {e:?}"
                                ),
                            },
                        )?,
                    ),
                };
                let mut stages = SequenceStages::default();
                for stage in &seq.stages {
                    stages
                        .append(
                            &self.memory,
                            SequenceStage {
                                callback: stage.callback.clone(),
                                filter: stage.filter,
                                span: stage.span,
                                module: self.module,
                            },
                        )
                        .map_err(|e| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: format!("Runtime sequence stage allocation failed: {e:?}"),
                        })?;
                }
                Rc::new(Sequence { source, stages })
            }
            Value::Range { start, end, step } => Rc::new(Sequence {
                source: SequenceSource::Range { start, end, step },
                stages: SequenceStages::default(),
            }),
            Value::List(items) => Rc::new(Sequence {
                source: SequenceSource::List(
                    memory::Buffer::from_iter(&self.memory, items).map_err(|e| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Runtime sequence source allocation failed: {e:?}"),
                    })?,
                ),
                stages: SequenceStages::default(),
            }),
            _ => return self.error(span, "Expected a list or sequence"),
        };
        Ok(SequenceCursor {
            sequence,
            index: 0,
            previous: None,
            host: None,
            finished: false,
        })
    }

    fn sequence_next(
        &mut self,
        cursor: &mut SequenceCursor<'s>,
        span: Span,
    ) -> Result<Option<Value<'s>>> {
        if cursor.finished {
            return Ok(None);
        }
        'candidate: loop {
            self.charge(1, span)?;
            let mut item = match &cursor.sequence.source {
                SequenceSource::Host(source) => {
                    if cursor.host.is_none() {
                        cursor.host =
                            Some(source.factory.open(self.cancellation).map_err(|message| {
                                RuntimeError {
                                    stack: Vec::new(),
                                    location: None,
                                    module: self.module.map(str::to_owned),
                                    span,
                                    message,
                                }
                            })?);
                        self.check(span)?;
                    }
                    let next = cursor
                        .host
                        .as_mut()
                        .unwrap()
                        .next(self.cancellation)
                        .map_err(|message| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message,
                        })?;
                    self.check(span)?;
                    let Some(item) = next else {
                        cursor.finished = true;
                        cursor.host = None;
                        return Ok(None);
                    };
                    self.host_value_size(&item, span, 0)?;
                    if !source.item_type.accepts(&item) {
                        return self
                            .error(span, "Host sequence item does not match its declared type");
                    }
                    item
                }
                SequenceSource::List(items) => match items.get(cursor.index) {
                    Some(item) => item.clone(),
                    None => return Ok(None),
                },
                SequenceSource::Range { start, end, step } => {
                    let current = (cursor.index as f64).mul_add(*step, *start);
                    if cursor.previous == Some(current) || !current.is_finite() {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    if if *step > 0.0 {
                        current >= *end
                    } else {
                        current <= *end
                    } {
                        return Ok(None);
                    }
                    cursor.previous = Some(current);
                    Value::Number(current)
                }
            };
            cursor.index = cursor.index.checked_add(1).ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Sequence index overflow".into(),
            })?;
            for stage in cursor.sequence.stages.iter() {
                let previous_module = self.module;
                self.module = stage.module;
                let result = self
                    .call(
                        stage.callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        stage.span,
                    )
                    .and_then(|value| {
                        if stage.filter && !matches!(value, Value::Bool(_)) {
                            self.error(stage.span, "Filter callback must return a boolean")
                        } else {
                            Ok(value)
                        }
                    });
                self.module = previous_module;
                let result = result?;
                if stage.filter {
                    if result == Value::Bool(false) {
                        continue 'candidate;
                    }
                } else {
                    item = result;
                }
            }
            return Ok(Some(item));
        }
    }

    fn equal(&mut self, left: &Value<'_>, right: &Value<'_>, span: Span) -> Result<bool> {
        // Scalar fast path: identical step accounting without a worklist
        // allocation. Compound and mismatched values use the general walk.
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => {
                self.charge(1, span)?;
                return Ok(a == b);
            }
            (Value::Bool(a), Value::Bool(b)) => {
                self.charge(1, span)?;
                return Ok(a == b);
            }
            (Value::Null, Value::Null) => {
                self.charge(1, span)?;
                return Ok(true);
            }
            (Value::String(a), Value::String(b)) => {
                self.charge(1, span)?;
                self.charge(a.len().max(b.len()), span)?;
                return Ok(a == b);
            }
            (Value::Angle(a), Value::Angle(b)) => {
                self.charge(1, span)?;
                return Ok(a == b);
            }
            _ => {}
        }
        let mut pending = vec![(left, right)];
        while let Some((left, right)) = pending.pop() {
            self.charge(1, span)?;
            match (left, right) {
                (
                    Value::Sequence(_)
                    | Value::BytecodeSequence(_)
                    | Value::BytecodeIterator(_)
                    | Value::Function(_)
                    | Value::BytecodeFunction(_)
                    | Value::Host(_)
                    | Value::Builtin(_),
                    _,
                )
                | (
                    _,
                    Value::Sequence(_)
                    | Value::BytecodeSequence(_)
                    | Value::BytecodeIterator(_)
                    | Value::Function(_)
                    | Value::BytecodeFunction(_)
                    | Value::Host(_)
                    | Value::Builtin(_),
                ) => {
                    return self.error(span, "Functions and lazy sequences cannot be compared");
                }
                (Value::UserData(a), Value::UserData(b)) => {
                    if a.type_name != b.type_name
                        || a.variant != b.variant
                        || a.values.len() != b.values.len()
                    {
                        return Ok(false);
                    }
                    self.charge(a.values.len(), span)?;
                    pending.extend(a.values.iter().zip(&b.values));
                }
                (Value::Variant(a, left), Value::Variant(b, right)) => {
                    if a != b || left.len() != right.len() {
                        return Ok(false);
                    }
                    self.charge(left.len(), span)?;
                    pending.extend(left.iter().zip(right));
                }
                (Value::List(a), Value::List(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    self.charge(a.len(), span)?;
                    pending.extend(a.iter().zip(b));
                }
                (Value::Record(a), Value::Record(b)) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    self.charge(a.len(), span)?;
                    for ((ka, va), (kb, vb)) in a.iter().zip(b) {
                        self.charge(ka.len().max(kb.len()), span)?;
                        if ka != kb {
                            return Ok(false);
                        }
                        pending.push((va, vb));
                    }
                }
                (Value::String(a), Value::String(b)) => {
                    self.charge(a.len().max(b.len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                (Value::Mesh(a), Value::Mesh(b)) => {
                    self.charge(a.vertices().len().max(b.vertices().len()), span)?;
                    self.charge(a.triangles().len().max(b.triangles().len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                (Value::Polygon(a), Value::Polygon(b)) => {
                    self.charge(a.points().len().max(b.points().len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                _ => {
                    if left != right {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
    fn match_value(
        &mut self,
        pattern: &Expr<'s>,
        value: &Value<'s>,
        local: &mut Environment<'s>,
    ) -> Result<bool> {
        self.charge(1, pattern.span)?;
        match &pattern.kind {
            ExprKind::Name(name) => {
                if name.text != "_" {
                    local
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(pattern.span, e))?;
                }
                Ok(true)
            }
            ExprKind::Tuple(patterns) => {
                let Value::Tuple(values) = value else {
                    return Ok(false);
                };
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (pattern, value) in patterns.iter().zip(values) {
                    if !self.match_value(pattern, value, local)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            ExprKind::Map(patterns) => {
                let fields = match value {
                    Value::Record(fields) => fields,
                    Value::UserData(data) if data.variant.is_none() => {
                        let Some(Value::Record(fields)) = data.values.first() else {
                            return Ok(false);
                        };
                        fields
                    }
                    _ => return Ok(false),
                };
                for (key, pattern) in patterns {
                    let ExprKind::Name(name) = &key.kind else {
                        return Ok(false);
                    };
                    let Some(value) = fields.get(name.text) else {
                        return Ok(false);
                    };
                    if !self.match_value(pattern, value, local)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            ExprKind::Call { callee, arguments } => {
                let values = match (&callee.kind, value) {
                    (ExprKind::Name(name), Value::Variant(tag, values)) if name.text == *tag => {
                        values
                    }
                    (_, Value::UserData(data)) => {
                        let Value::Function(constructor) = self.expr(callee, local)? else {
                            return Ok(false);
                        };
                        let FunctionBody::Constructor {
                            type_name: expected,
                            variant: tag,
                            ..
                        } = &*constructor.body
                        else {
                            return Ok(false);
                        };
                        if expected != &data.type_name || tag != &data.variant {
                            return Ok(false);
                        }
                        &data.values
                    }
                    _ => return Ok(false),
                };
                if arguments.len() != values.len() {
                    return Ok(false);
                }
                for (pattern, value) in arguments.iter().zip(values) {
                    if !self.match_value(pattern, value, local)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => {
                let literal = self.expr(pattern, local)?;
                self.equal(&literal, value, pattern.span)
            }
        }
    }
    fn bind_pattern(
        &self,
        pattern: &Expr<'s>,
        value: &Value<'s>,
        bindings: &mut Environment<'s>,
    ) -> Result<()> {
        self.check(pattern.span)?;
        if let (ExprKind::Map(_), Value::UserData(data)) = (&pattern.kind, value)
            && data.variant.is_none()
            && let Some(record @ Value::Record(_)) = data.values.first()
        {
            return self.bind_pattern(pattern, record, bindings);
        }
        match (&pattern.kind, value) {
            (ExprKind::Name(name), _) => {
                if name.text != "_" {
                    bindings
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(pattern.span, e))?;
                }
                Ok(())
            }
            (ExprKind::Map(entries), Value::Record(fields)) => {
                for (key, pattern) in entries {
                    let ExprKind::Name(name) = &key.kind else {
                        return self.error(key.span, "Record pattern keys must be names");
                    };
                    let Some(value) = fields.get(name.text) else {
                        return self.error(key.span, "Missing record pattern field");
                    };
                    self.bind_pattern(pattern, value, bindings)?;
                }
                Ok(())
            }
            (ExprKind::Tuple(patterns), Value::Tuple(values)) if patterns.len() == values.len() => {
                for (pattern, value) in patterns.iter().zip(values) {
                    self.bind_pattern(pattern, value, bindings)?;
                }
                Ok(())
            }
            _ => self.error(pattern.span, "Value does not match binding pattern"),
        }
    }
    fn module_statements(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let result = self.statements(statements, environment)?;
        let exports: Vec<_> = statements
            .iter()
            .filter_map(|statement| {
                if let StmtKind::Export(names) = &statement.kind {
                    Some(names)
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        if exports.is_empty() {
            return Ok(result);
        }
        self.collection_growth(
            0,
            exports.len(),
            statements.first().map_or(Span::new(0, 0), |s| s.span),
        )?;
        let mut values = BTreeMap::new();
        for name in exports {
            self.string_growth(0, name.text.len(), name.span)?;
            let value = self.expr(
                &Expr {
                    span: name.span,
                    kind: ExprKind::Name(name.clone()),
                },
                environment,
            )?;
            values.insert(name.text.to_owned(), value);
        }
        Ok((Value::Record(values), Flow::Next))
    }
    fn scoped_module(
        &mut self,
        statements: &[Stmt<'s>],
        mut environment: Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let result = self.module_statements(statements, &mut environment);
        drop(environment);
        self.reclaim_cells();
        result
    }
    fn scoped_statements(
        &mut self,
        statements: &[Stmt<'s>],
        mut environment: Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let result = self.statements(statements, &mut environment);
        drop(environment);
        self.reclaim_cells();
        result
    }
    fn declare_type(
        &mut self,
        statement: &Stmt<'s>,
        environment: &mut Environment<'s>,
    ) -> Result<()> {
        match &statement.kind {
            StmtKind::Struct { name, fields } => {
                let id = format!("{}::{}", self.module.unwrap_or("<entry>"), name.text);
                self.user_types.insert((self.module, name.text), id.clone());
                let fields = fields
                    .iter()
                    .map(|(name, ty)| Ok((name.text.to_owned(), self.annotation(ty)?)))
                    .collect::<Result<Vec<_>>>()?;
                let constructor = self.constructor(name, id, None, fields, Vec::new())?;
                environment
                    .insert(name.text, constructor)
                    .map_err(|e| self.environment_error(name.span, e))?;
            }
            StmtKind::Enum { name, variants } => {
                let id = format!("{}::{}", self.module.unwrap_or("<entry>"), name.text);
                self.user_types.insert((self.module, name.text), id.clone());
                let mut constructors = BTreeMap::new();
                self.collection_growth(0, variants.len(), name.span)?;
                for (variant, types) in variants {
                    self.string_growth(0, variant.text.len(), variant.span)?;
                    let types = types
                        .iter()
                        .map(|ty| self.annotation(ty))
                        .collect::<Result<Vec<_>>>()?;
                    constructors.insert(
                        variant.text.to_owned(),
                        self.constructor(
                            variant,
                            id.clone(),
                            Some(variant.text.to_owned()),
                            Vec::new(),
                            types,
                        )?,
                    );
                }
                environment
                    .insert(name.text, Value::Record(constructors))
                    .map_err(|e| self.environment_error(name.span, e))?;
            }
            _ => unreachable!("type declaration"),
        }
        Ok(())
    }
    fn statements(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let mut value = Value::Null;
        for statement in statements {
            self.check(statement.span)?;
            if self.remaining == 0 {
                return self.error(statement.span, "Execution limit exceeded");
            }
            self.remaining -= 1;
            match &statement.kind {
                StmtKind::Struct { .. } | StmtKind::Enum { .. } => {
                    self.declare_type(statement, environment)?;
                    value = Value::Null;
                }
                StmtKind::Export(_) => value = Value::Null,
                StmtKind::Import(name) => {
                    value = Value::Null;
                    if let Ok(index) = self
                        .module_cache
                        .binary_search_by_key(&name.text, |(key, _)| *key)
                    {
                        environment
                            .insert(name.text, self.module_cache[index].1.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                        continue;
                    }
                    if self.loading.contains(&name.text) {
                        return self.error(name.span, "Cyclic module import");
                    }
                    let module =
                        self.modules
                            .get(name.text)
                            .cloned()
                            .ok_or_else(|| RuntimeError {
                                stack: Vec::new(),
                                location: None,
                                module: self.module.map(str::to_owned),
                                span: name.span,
                                message: format!("Unknown module: {}", name.text),
                            })?;
                    if self.depth >= self.max_depth {
                        return self.error(name.span, "Execution limit exceeded");
                    }
                    if let Err(error) = self.loading.reserve(1) {
                        return self.error(
                            name.span,
                            &format!("Runtime module stack allocation failed: {error:?}"),
                        );
                    }
                    // Reserve an export slot for this module and every active
                    // ancestor before executing any module body. Nested imports
                    // must not consume their callers' reserved capacity.
                    if let Err(error) = self.module_cache.reserve(self.loading.len() + 1) {
                        return self.error(
                            name.span,
                            &format!("Runtime module cache allocation failed: {error:?}"),
                        );
                    }
                    let module_environment = self
                        .module_globals
                        .try_clone()
                        .map_err(|e| self.environment_error(name.span, e))?;
                    self.loading
                        .push(name.text)
                        .expect("module stack reserved before execution");
                    self.depth += 1;
                    let previous_module = self.module.replace(name.text);
                    let previous_references = std::mem::replace(
                        &mut self.current_references,
                        self.references[&Some(name.text)].clone(),
                    );
                    let result = self.scoped_module(&module.items, module_environment);
                    self.module = previous_module;
                    self.current_references = previous_references;
                    self.depth -= 1;
                    self.loading.pop();
                    let (exports, _) = result?;
                    if !matches!(exports, Value::Record(_)) {
                        return self.error(name.span, "Module must evaluate to an export record");
                    }
                    self.enforce_retained_limit(name.span, Some(&exports))?;
                    let index = self
                        .module_cache
                        .binary_search_by_key(&name.text, |(key, _)| *key)
                        .expect_err("an active module cannot already be cached");
                    self.module_cache
                        .push((name.text, exports.clone()))
                        .expect("module export slot reserved before execution");
                    self.module_cache[index..].rotate_right(1);
                    environment
                        .insert(name.text, exports)
                        .map_err(|e| self.environment_error(name.span, e))?;
                }

                StmtKind::Destructure {
                    pattern,
                    value: expression,
                } => {
                    value = self.expr(expression, environment)?;
                    let mut bindings = Environment::new(&self.memory);
                    self.bind_pattern(pattern, &value, &mut bindings)?;
                    environment
                        .extend(bindings)
                        .map_err(|e| self.environment_error(statement.span, e))?;
                }
                StmtKind::Declaration {
                    role: _,
                    name,
                    constant,
                    value: expression,
                    ty,
                } => {
                    let contract = ty.as_ref().map(|ty| self.annotation(ty)).transpose()?;
                    value = self.expr(expression, environment)?;
                    if contract.as_ref().is_some_and(|ty| !ty.accepts(&value)) {
                        return self
                            .error(expression.span, "Value does not match its type annotation");
                    }
                    if *constant {
                        environment
                            .insert(name.text, value.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                    } else {
                        let cell = self.allocate_cell(
                            value.clone(),
                            contract.or_else(|| ValueType::inferred(&value)),
                            expression.span,
                        )?;
                        environment
                            .insert_binding(name.text, Binding::Cell(cell))
                            .map_err(|e| self.environment_error(name.span, e))?;
                    }
                }
                StmtKind::Function {
                    name,
                    parameters,
                    body,
                    result,
                } => {
                    let shared_body =
                        self.shared_body(statement.span, || FunctionBody::Block(body.clone()));
                    value = Value::Function(Rc::new(Closure {
                        parameters: parameters.iter().map(|p| p.pattern.clone()).collect(),
                        parameter_types: parameters
                            .iter()
                            .enumerate()
                            .map(|(i, p)| {
                                p.ty.as_ref()
                                    .map(|ty| self.annotation(ty))
                                    .transpose()
                                    .map(|ty| {
                                        ty.or_else(|| {
                                            self.contracts
                                                .get(&self.module)
                                                .and_then(|m| m.get(&name.span.start))
                                                .and_then(|c| c.0.get(i))
                                                .cloned()
                                                .flatten()
                                                .map(|ty| self.runtime_contract(&ty))
                                        })
                                    })
                            })
                            .collect::<Result<Vec<_>>>()?,
                        result_type: result
                            .as_ref()
                            .map(|ty| self.annotation(ty))
                            .transpose()?
                            .or_else(|| {
                                self.contracts
                                    .get(&self.module)
                                    .and_then(|m| m.get(&name.span.start))
                                    .and_then(|c| c.1.as_ref())
                                    .map(|ty| self.runtime_contract(ty))
                            }),
                        name: Some(name.text),
                        module: self.module,
                        body: shared_body,
                        environment: self.capture(statement.span, environment)?,
                        references: self.current_references.clone(),
                    }));
                    environment
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(name.span, e))?;
                }
                StmtKind::Return(expression) => {
                    return Ok((
                        match expression {
                            Some(expression) => self.expr(expression, environment)?,
                            None => Value::Null,
                        },
                        Flow::Return,
                    ));
                }
                StmtKind::Break => return Ok((Value::Null, Flow::Break)),
                StmtKind::Continue => return Ok((Value::Null, Flow::Continue)),
                StmtKind::While { condition, body } => {
                    // The outer binding map is loop-invariant (bodies only
                    // mutate through shared cells), so snapshot it once and
                    // give every iteration a cheap pooled child scope.
                    let base = memory::Shared::new(
                        &self.memory,
                        environment
                            .try_clone()
                            .map_err(|e| self.environment_error(statement.span, e))?,
                    )
                    .map_err(|e| self.environment_error(statement.span, e))?;
                    loop {
                        let Value::Bool(keep_going) = self.expr(condition, environment)? else {
                            return self.error(condition.span, "Condition must be boolean");
                        };
                        if !keep_going {
                            break;
                        }
                        let mut scope = self.take_call_environment(&base);
                        // Implicit per-iteration arena: scratch cells die in
                        // bulk at the end of the iteration; escaping values
                        // promote like in an explicit `region` block.
                        let iteration = self.enter_region(None, None);
                        let result = self.statements(&body.stmts, &mut scope);
                        self.recycle_call_environment(scope);
                        self.reclaim_cells();
                        self.exit_region(iteration, statement.span)?;
                        let result = result?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = Value::Null;
                }
                StmtKind::For {
                    binding,
                    iterable,
                    body,
                } => {
                    let source = self.expr(iterable, environment)?;
                    let mut cursor = self.sequence_cursor(source, iterable.span)?;
                    let base = memory::Shared::new(
                        &self.memory,
                        environment
                            .try_clone()
                            .map_err(|e| self.environment_error(statement.span, e))?,
                    )
                    .map_err(|e| self.environment_error(statement.span, e))?;
                    while let Some(item) = self.sequence_next(&mut cursor, iterable.span)? {
                        let mut scope = self.take_call_environment(&base);
                        scope
                            .insert(binding.text, item)
                            .map_err(|e| self.environment_error(binding.span, e))?;
                        // Same implicit per-iteration arena as in `while`.
                        let iteration = self.enter_region(None, None);
                        let result = self.statements(&body.stmts, &mut scope);
                        self.recycle_call_environment(scope);
                        self.reclaim_cells();
                        self.exit_region(iteration, statement.span)?;
                        let result = result?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = Value::Null;
                }
                StmtKind::Show(expression) | StmtKind::Expr(expression) => {
                    value = self.expr(expression, environment)?
                }
                StmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    let Value::Bool(condition) = self.expr(condition, environment)? else {
                        return self.error(statement.span, "Condition must be boolean");
                    };
                    let block = if condition {
                        Some(then_block)
                    } else {
                        else_block.as_ref()
                    };
                    if let Some(block) = block {
                        let result = self.scoped_statements(
                            &block.stmts,
                            environment
                                .try_clone()
                                .map_err(|e| self.environment_error(statement.span, e))?,
                        )?;
                        if result.1 != Flow::Next {
                            return Ok(result);
                        }
                        value = result.0;
                    } else {
                        value = Value::Null;
                    }
                }
                StmtKind::Region {
                    name,
                    strict: _,
                    budget,
                    body,
                } => {
                    let budget = self.eval_region_budget(budget.as_ref(), environment)?;
                    let scope = environment
                        .try_clone()
                        .map_err(|e| self.environment_error(statement.span, e))?;
                    let region = self.enter_region(budget, name.as_ref().map(|name| name.text));
                    let result = self.scoped_statements(&body.stmts, scope);
                    self.exit_region(region, statement.span)?;
                    let result = result?;
                    if result.1 != Flow::Next {
                        return Ok(result);
                    }
                    value = result.0;
                }
                _ => return self.error(statement.span, "Statement is not executable yet"),
            }
        }
        Ok((value, Flow::Next))
    }
    fn environment_error(&self, span: Span, error: memory::AllocationError) -> RuntimeError {
        RuntimeError {
            stack: Vec::new(),
            location: None,
            module: self.module.map(str::to_owned),
            span,
            message: format!("Runtime environment allocation failed: {error:?}"),
        }
    }
    fn error<T>(&self, span: Span, message: &str) -> Result<T> {
        Err(RuntimeError {
            stack: Vec::new(),
            location: None,
            module: self.module.map(str::to_owned),
            span,
            message: message.into(),
        })
    }
    fn expr(&mut self, expression: &Expr<'s>, environment: &Environment<'s>) -> Result<Value<'s>> {
        self.check(expression.span)?;
        if self.remaining == 0 || self.depth >= self.max_depth {
            return self.error(expression.span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        self.depth += 1;
        let result = self.inner(expression, environment);
        self.depth -= 1;
        result
    }
    fn inner(&mut self, expression: &Expr<'s>, environment: &Environment<'s>) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Try(value) => match self.expr(value, environment)? {
                Value::Variant("Some" | "Ok", mut items) if items.len() == 1 => Ok(items.remove(0)),
                value @ Value::Variant("None" | "Err", _) => {
                    self.early_return = Some(value);
                    self.error(span, "Internal early return")
                }
                _ => self.error(span, "? requires Option or Result"),
            },

            ExprKind::String(text) => {
                self.charge(text.len().saturating_sub(2), span)?;
                let mut decoded = String::new();
                for character in crate::string_literal::characters(text) {
                    let character = match character {
                        Ok(character) => character,
                        Err(message) => return self.error(span, message),
                    };
                    self.string_growth(decoded.len(), character.len_utf8(), span)?;
                    decoded.push(character);
                }
                Ok(Value::String(decoded))
            }
            ExprKind::Interpolate(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        crate::InterpolationPart::Literal(s) => {
                            self.string_growth(out.len(), s.len(), span)?;
                            out.push_str(s);
                        }
                        crate::InterpolationPart::Expr(sub) => {
                            let val = self.expr(sub, environment)?;
                            let mut rendered = String::new();
                            format_value_for_display(&val, &mut rendered, 0);
                            self.string_growth(out.len(), rendered.len(), span)?;
                            out.push_str(&rendered);
                        }
                    }
                }
                Ok(Value::String(out))
            }
            ExprKind::Map(entries) => {
                self.collection_growth(0, entries.len(), span)?;
                let mut fields = BTreeMap::new();
                for (key, expression) in entries {
                    let key = match &key.kind {
                        ExprKind::Name(name) => {
                            self.string_growth(0, name.text.len(), key.span)?;
                            name.text.to_owned()
                        }
                        _ => match self.expr(key, environment)? {
                            Value::String(key) => key,
                            _ => {
                                return self.error(key.span, "Record key must be a name or string");
                            }
                        },
                    };
                    self.string_growth(0, key.len(), span)?;
                    if fields.contains_key(&key) {
                        return self.error(span, "Duplicate record field");
                    }
                    fields.insert(key, self.expr(expression, environment)?);
                }
                Ok(Value::Record(fields))
            }
            ExprKind::Number(text) => (if text.contains('_') {
                Cow::Owned(text.replace('_', ""))
            } else {
                Cow::Borrowed(*text)
            })
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(Value::Number)
            .ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Unsupported or non-finite number".into(),
            }),
            ExprKind::Bool(value) => Ok(Value::Bool(*value)),
            ExprKind::Null => Ok(Value::Null),
            ExprKind::Name(name) => match environment.get(name.text) {
                Some(Binding::Value(value)) => Ok(value.clone()),
                Some(Binding::Cell(cell)) => Ok(self.cells[self.cell_index(cell, span)?].0.clone()),
                None => self.error(span, &format!("Unknown name: {}", name.text)),
            },
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                let (root_name, steps) = self.resolve_lvalue_path(target, environment)?;
                let Some(Binding::Cell(cell)) = environment.get(root_name.text) else {
                    return self.error(root_name.span, "Assignment requires a mutable variable");
                };
                let index = self.cell_index(cell, root_name.span)?;
                let assigned = if *operator == "=" {
                    self.expr(value, environment)?
                } else {
                    let op = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        _ => return self.error(span, "Unknown assignment operator"),
                    };
                    self.binary_expression(op, target, value, environment, span)?
                };
                if steps.is_empty() {
                    if self.cells[index]
                        .1
                        .as_ref()
                        .is_some_and(|ty| !ty.accepts(&assigned))
                    {
                        return self.error(span, "Assignment violates variable type");
                    }
                    let previous = std::mem::replace(&mut self.cells[index].0, assigned.clone());
                    if let Err(error) = self.enforce_retained_limit(span, None) {
                        self.cells[index].0 = previous;
                        return Err(error);
                    }
                } else {
                    let previous = self.cells[index].0.clone();
                    if let Err(err) = mutate_path(
                        self.module,
                        &mut self.cells[index].0,
                        &steps,
                        assigned.clone(),
                        span,
                    ) {
                        self.cells[index].0 = previous;
                        return Err(err);
                    }
                    if self.cells[index]
                        .1
                        .as_ref()
                        .is_some_and(|ty| !ty.accepts(&self.cells[index].0))
                    {
                        self.cells[index].0 = previous;
                        return self.error(span, "Assignment violates variable type");
                    }
                    if let Err(error) = self.enforce_retained_limit(span, None) {
                        self.cells[index].0 = previous;
                        return Err(error);
                    }
                }
                Ok(assigned)
            }
            ExprKind::Lambda { parameters, body } => {
                let body = self.shared_body(expression.span, || {
                    FunctionBody::Expression((**body).clone())
                });
                Ok(Value::Function(Rc::new(Closure {
                    parameters: parameters.clone(),
                    parameter_types: vec![None; parameters.len()],
                    result_type: None,
                    body,
                    name: None,
                    module: self.module,
                    environment: self.capture(expression.span, environment)?,
                    references: self.current_references.clone(),
                })))
            }
            ExprKind::Tuple(items) | ExprKind::List(items) => {
                self.collection_growth(0, items.len(), span)?;
                let values = items
                    .iter()
                    .map(|item| self.expr(item, environment))
                    .collect::<Result<Vec<_>>>()?;
                Ok(if matches!(expression.kind, ExprKind::Tuple(_)) {
                    Value::Tuple(values)
                } else {
                    Value::List(values)
                })
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                let Value::Bool(condition) = self.expr(condition, environment)? else {
                    return self.error(span, "Condition must be boolean");
                };
                self.expr(if condition { then_value } else { else_value }, environment)
            }
            ExprKind::Member { object, field } => {
                let object = self.expr(object, environment)?;
                if let Value::Mesh(mesh) = &object {
                    return match field.text {
                        "vertices" => {
                            self.collection_growth(0, mesh.vertices().len(), span)?;
                            self.charge(mesh.vertices().len(), span)?;
                            Ok(Value::List(
                                mesh.vertices()
                                    .iter()
                                    .map(|v| Value::Vector(v.to_vec()))
                                    .collect(),
                            ))
                        }
                        "triangles" => {
                            self.collection_growth(0, mesh.triangles().len(), span)?;
                            if !mesh.triangles().is_empty() {
                                self.collection_growth(0, 3, span)?;
                            }
                            self.charge(mesh.triangles().len(), span)?;
                            Ok(Value::List(
                                mesh.triangles()
                                    .iter()
                                    .map(|t| {
                                        Value::List(
                                            t.iter().map(|i| Value::Number(*i as f64)).collect(),
                                        )
                                    })
                                    .collect(),
                            ))
                        }
                        _ => self.error(field.span, "Unknown mesh field"),
                    };
                }
                if let Value::UserData(data) = &object
                    && data.variant.is_none()
                    && let Some(Value::Record(fields)) = data.values.first()
                {
                    return fields.get(field.text).cloned().ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span: field.span,
                        message: format!("Unknown struct field: {}", field.text),
                    });
                }
                if let Value::Record(fields) = object {
                    return fields.get(field.text).cloned().ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span: field.span,
                        message: format!("Unknown field: {}", field.text),
                    });
                }
                if !matches!(object, Value::Vector(_)) {
                    return self.error(field.span, "Value does not support member access");
                }
                let axis = match field.text {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return self.error(field.span, "Unknown vector component"),
                };
                match object {
                    Value::Vector(values) => values
                        .get(axis)
                        .copied()
                        .map(Value::Number)
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span: field.span,
                            message: "Vector component out of bounds".into(),
                        }),
                    _ => self.error(span, "Member access requires a vector"),
                }
            }
            ExprKind::Index { object, index } => {
                let object = self.expr(object, environment)?;
                let index = self.expr(index, environment)?;
                if let Value::Record(fields) = &object {
                    let Value::String(key) = index else {
                        return self.error(span, "Record index must be a string");
                    };
                    return fields.get(&key).cloned().ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Unknown field: {key}"),
                    });
                }
                let Value::Number(index) = index else {
                    return self.error(span, "Index must be an integer");
                };
                if index < 0.0 || index.fract() != 0.0 || index >= usize::MAX as f64 {
                    return self.error(span, "Invalid collection index");
                }
                let result = match object {
                    Value::List(values) | Value::Tuple(values) => {
                        values.get(index as usize).cloned()
                    }
                    Value::Vector(values) => values.get(index as usize).copied().map(Value::Number),
                    _ => return self.error(span, "Indexing requires a list or vector"),
                };
                result.ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "Index out of bounds".into(),
                })
            }
            ExprKind::Match { value, arms } => {
                let value = self.expr(value, environment)?;
                for arm in arms {
                    let mut local = environment
                        .try_clone()
                        .map_err(|e| self.environment_error(span, e))?;
                    let matched = self.match_value(&arm.pattern, &value, &mut local)?;
                    if matched {
                        if let Some(guard) = &arm.guard {
                            match self.expr(guard, &local)? {
                                Value::Bool(true) => {}
                                Value::Bool(false) => continue,
                                _ => return self.error(guard.span, "Match guard must be boolean"),
                            }
                        }
                        return self.expr(&arm.value, &local);
                    }
                }
                self.error(span, "No matching pattern")
            }
            ExprKind::Pipeline { input, stages } => {
                let mut value = self.expr(input, environment)?;
                for stage in stages {
                    let (callee, supplied) = match &stage.kind {
                        ExprKind::Call { callee, arguments } => {
                            (callee.as_ref(), arguments.as_slice())
                        }
                        _ => (stage, &[][..]),
                    };
                    let function = self.expr(callee, environment)?;
                    let mut arguments = vec![value];
                    for argument in supplied {
                        arguments.push(self.expr(argument, environment)?);
                    }
                    value = self.call(function, arguments.into(), stage.span)?;
                }
                Ok(value)
            }
            ExprKind::Call { callee, arguments } => {
                // `move(binding)` empties a mutable cell and yields its value.
                // A user binding named `move` shadows this form.
                if let ExprKind::Name(name) = &callee.kind
                    && name.text == "move"
                    && environment.get(name.text).is_none()
                {
                    let [target] = arguments.as_slice() else {
                        return self.error(span, "move requires exactly one argument");
                    };
                    let ExprKind::Name(target) = &target.kind else {
                        return self.error(target.span, "move requires a mutable binding name");
                    };
                    return self.move_cell(target, environment);
                }
                let function = self.expr(callee, environment)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expr(argument, environment))
                    .collect::<Result<Vec<_>>>()?;
                self.call(function, arguments.into(), span)
            }
            ExprKind::Unary { operator, value } => {
                match (*operator, self.expr(value, environment)?) {
                    ("-", Value::Number(n)) => Ok(Value::Number(-n)),
                    ("+", Value::Number(n)) => Ok(Value::Number(n)),
                    ("-", Value::Vector(mut values)) => {
                        for value in &mut values {
                            *value = -*value;
                        }
                        self.vector(values, span)
                    }
                    ("+", Value::Vector(values)) => self.vector(values, span),
                    ("!" | "not", Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => self.error(span, "Invalid unary operand"),
                }
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => self.binary_inner(operator, left, right, environment, span),
            _ => self.error(span, "Expression is not executable yet"),
        }
    }
    /// Take the value out of a mutable binding's cell, leaving `Null` behind.
    fn move_cell(&mut self, name: &Name<'s>, environment: &Environment<'s>) -> Result<Value<'s>> {
        let Some(binding) = environment.get(name.text) else {
            return self.error(name.span, &format!("Unknown name: {}", name.text));
        };
        match binding {
            Binding::Value(_) => self.error(name.span, "move requires a mutable binding"),
            Binding::Cell(id) => Ok(std::mem::replace(&mut self.cells[id.index].0, Value::Null)),
        }
    }
    fn resolve_lvalue_path(
        &mut self,
        expr: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<(Name<'s>, Vec<AccessStep<'s>>)> {
        match &expr.kind {
            ExprKind::Name(name) => Ok((name.clone(), Vec::new())),
            ExprKind::Index { object, index } => {
                let (root, mut steps) = self.resolve_lvalue_path(object, environment)?;
                let index_val = self.expr(index, environment)?;
                steps.push(AccessStep::Index(index_val, index.span));
                Ok((root, steps))
            }
            ExprKind::Member { object, field } => {
                let (root, mut steps) = self.resolve_lvalue_path(object, environment)?;
                steps.push(AccessStep::Member(field.text.to_owned(), field.span));
                Ok((root, steps))
            }
            _ => self.error(expr.span, "Assignment requires a variable, member or index"),
        }
    }
    fn binary_expression(
        &mut self,
        operator: &str,
        left: &Expr<'s>,
        right: &Expr<'s>,
        environment: &Environment<'s>,
        span: Span,
    ) -> Result<Value<'s>> {
        // Preserve the step/depth of the synthetic binary expression formerly
        // used by compound assignment, while borrowing both operands' AST.
        self.check(span)?;
        if self.remaining == 0 || self.depth >= self.max_depth {
            return self.error(span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        self.depth += 1;
        let result = self.binary_inner(operator, left, right, environment, span);
        self.depth -= 1;
        result
    }
    #[inline(always)]
    fn binary_inner(
        &mut self,
        operator: &str,
        left: &Expr<'s>,
        right: &Expr<'s>,
        environment: &Environment<'s>,
        span: Span,
    ) -> Result<Value<'s>> {
        let left = self.expr(left, environment)?;
        if matches!(operator, "and" | "&&" | "or" | "||") {
            let Value::Bool(left) = left else {
                return self.error(span, "Boolean operand required");
            };
            if left == matches!(operator, "or" | "||") {
                return Ok(Value::Bool(left));
            }
            let right = self.expr(right, environment)?;
            return if matches!(right, Value::Bool(_)) {
                Ok(right)
            } else {
                self.error(span, "Boolean operand required")
            };
        }
        let right = self.expr(right, environment)?;
        if matches!(operator, "==" | "!=") {
            let equal = self.equal(&left, &right, span)?;
            return Ok(Value::Bool(if operator == "==" { equal } else { !equal }));
        }
        // Numeric fast path: skip the string/angle/matrix/vector guard chain
        // for the common number-operator-number case.
        if let (Value::Number(a), Value::Number(b)) = (&left, &right) {
            let (a, b) = (*a, *b);
            let number = match operator {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => a / b,
                "%" => exact_remainder(a, b),
                "**" => a.powf(b),
                "<" => return Ok(Value::Bool(a < b)),
                ">" => return Ok(Value::Bool(a > b)),
                "<=" => return Ok(Value::Bool(a <= b)),
                ">=" => return Ok(Value::Bool(a >= b)),
                _ => return self.error(span, "Unsupported binary operator"),
            };
            return if number.is_finite() {
                Ok(Value::Number(number))
            } else {
                self.error(span, "Non-finite arithmetic result")
            };
        }
        if let (Value::String(a), Value::String(b), "+") = (&left, &right, operator) {
            self.string_growth(a.len(), b.len(), span)?;
            self.charge(a.len(), span)?;
            self.charge(b.len(), span)?;
            return Ok(Value::String(format!("{a}{b}")));
        }
        if matches!(left, Value::Angle(_)) || matches!(right, Value::Angle(_)) {
            let radians = match (&left, &right, operator) {
                (Value::Angle(a), Value::Angle(b), "+") => a + b,
                (Value::Angle(a), Value::Angle(b), "-") => a - b,
                (Value::Angle(a), Value::Number(b), "*")
                | (Value::Number(b), Value::Angle(a), "*") => a * b,
                (Value::Angle(a), Value::Number(b), "/") => a / b,
                _ => return self.error(span, "Invalid angle operands"),
            };
            if !radians.is_finite() {
                return self.error(span, "Non-finite angle");
            }
            return Ok(Value::Angle(radians));
        }
        if let (Value::Quaternion(a), Value::Quaternion(b), "*") = (&left, &right, operator) {
            return Ok(Value::Quaternion(Box::new(a.compose(b))));
        }
        if let (Value::Matrix(a), Value::Matrix(b), "*") = (&left, &right, operator) {
            return a
                .multiply(b)
                .map(|matrix| Value::Matrix(Box::new(matrix)))
                .ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "Matrix multiplication overflow".into(),
                });
        }
        if matches!(left, Value::Vector(_)) || matches!(right, Value::Vector(_)) {
            let components = match (&left, &right, operator) {
                (Value::Vector(a), Value::Vector(b), "+" | "-") if a.len() == b.len() => a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| if operator == "+" { a + b } else { a - b })
                    .collect(),
                (Value::Vector(a), Value::Number(b), "*" | "/") => a
                    .iter()
                    .map(|a| if operator == "*" { a * b } else { a / b })
                    .collect(),
                (Value::Number(a), Value::Vector(b), "*") => b.iter().map(|b| a * b).collect(),
                _ => return self.error(span, "Invalid vector operands or dimensions"),
            };
            return self.vector(components, span);
        }
        let (Value::Number(a), Value::Number(b)) = (left, right) else {
            return self.error(span, "Numeric operands required");
        };
        let number = match operator {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => a / b,
            "%" => exact_remainder(a, b),
            "**" => a.powf(b),
            "==" => return Ok(Value::Bool(a == b)),
            "!=" => return Ok(Value::Bool(a != b)),
            "<" => return Ok(Value::Bool(a < b)),
            ">" => return Ok(Value::Bool(a > b)),
            "<=" => return Ok(Value::Bool(a <= b)),
            ">=" => return Ok(Value::Bool(a >= b)),
            _ => return self.error(span, "Unsupported binary operator"),
        };
        if number.is_finite() {
            Ok(Value::Number(number))
        } else {
            self.error(span, "Non-finite arithmetic result")
        }
    }
    fn vector(&self, values: Vec<f64>, span: Span) -> Result<Value<'s>> {
        if values.iter().all(|x| x.is_finite()) {
            Ok(Value::Vector(values))
        } else {
            self.error(span, "Non-finite vector result")
        }
    }
    fn math(&self, builtin: Builtin, arguments: &[Value<'s>], span: Span) -> Result<Value<'s>> {
        if matches!(builtin, Builtin::Vec2 | Builtin::Vec3 | Builtin::Vec4) {
            let dimension = match builtin {
                Builtin::Vec2 => 2,
                Builtin::Vec3 => 3,
                _ => 4,
            };
            if arguments.len() != dimension {
                return self.error(span, "Incorrect vector dimension");
            }
            let values = arguments
                .iter()
                .map(|v| {
                    if let Value::Number(n) = v {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .collect::<Option<Vec<_>>>();
            return match values {
                Some(values) => self.vector(values, span),
                None => self.error(span, "Vector components must be numbers"),
            };
        }
        match (builtin, arguments) {
            (Builtin::Slerp, [Value::Quaternion(a), Value::Quaternion(b), Value::Number(t)]) => {
                return a
                    .slerp(b, *t)
                    .map(|q| Value::Quaternion(Box::new(q)))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "slerp requires a finite fraction between 0 and 1".into(),
                    });
            }
            (
                Builtin::AxisAngle,
                [
                    Value::Vector(axis),
                    Value::Number(angle) | Value::Angle(angle),
                ],
            ) if axis.len() == 3 => {
                return crate::Quaternion::axis_angle([axis[0], axis[1], axis[2]], *angle)
                    .map(|q| Value::Quaternion(Box::new(q)))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Rotation requires a finite nonzero axis and angle".into(),
                    });
            }
            (Builtin::RotationMatrix, [Value::Quaternion(q)]) => {
                return Ok(Value::Matrix(Box::new(q.matrix())));
            }
            (Builtin::Identity, []) => {
                return Ok(Value::Matrix(Box::new(crate::Matrix4::identity())));
            }
            (Builtin::Translation | Builtin::Scaling, [Value::Vector(v)]) if v.len() == 3 => {
                let mut matrix = crate::Matrix4::identity();
                for (axis, value) in v.iter().enumerate() {
                    if builtin == Builtin::Translation {
                        matrix.0[axis][3] = *value;
                    } else {
                        matrix.0[axis][axis] = *value;
                    }
                }
                return Ok(Value::Matrix(Box::new(matrix)));
            }
            (
                Builtin::RotationX | Builtin::RotationY | Builtin::RotationZ,
                [Value::Number(angle) | Value::Angle(angle)],
            ) => {
                let mut matrix = crate::Matrix4::identity();
                let (sin, cos) = angle.sin_cos();
                let (a, b) = match builtin {
                    Builtin::RotationX => (1, 2),
                    Builtin::RotationY => (2, 0),
                    _ => (0, 1),
                };
                matrix.0[a][a] = cos;
                matrix.0[a][b] = -sin;
                matrix.0[b][a] = sin;
                matrix.0[b][b] = cos;
                return Ok(Value::Matrix(Box::new(matrix)));
            }
            (
                Builtin::TransformPoint | Builtin::TransformDirection,
                [Value::Matrix(matrix), Value::Vector(v)],
            ) => {
                return matrix
                    .apply(v, builtin == Builtin::TransformPoint)
                    .map(Value::Vector)
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Invalid matrix transformation".into(),
                    });
            }
            _ => {}
        }
        if let (Builtin::Degrees | Builtin::Radians, [Value::Number(value)]) = (builtin, arguments)
        {
            let radians = if builtin == Builtin::Degrees {
                value.to_radians()
            } else {
                *value
            };
            if !radians.is_finite() {
                return self.error(span, "Non-finite angle");
            }
            return Ok(Value::Angle(radians));
        }
        let shape = match (builtin, arguments) {
            (Builtin::Polygon, [Value::List(points)]) => {
                self.collection_growth(0, points.len(), span)?;
                let points = points
                    .iter()
                    .map(|p| match p {
                        Value::Vector(v) if v.len() == 2 => Some([v[0], v[1]]),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>();
                Some(match points {
                    Some(points) => crate::Polygon::new(points),
                    None => Err("Polygon points must be vec2 values"),
                })
            }
            (Builtin::Translate, [Value::Polygon(polygon), Value::Vector(offset)])
                if offset.len() == 2 =>
            {
                self.collection_growth(0, polygon.points().len(), span)?;
                Some(polygon.translated([offset[0], offset[1]]))
            }
            (
                Builtin::Rotate,
                [
                    Value::Polygon(polygon),
                    Value::Number(angle) | Value::Angle(angle),
                ],
            ) => {
                self.collection_growth(0, polygon.points().len(), span)?;
                Some(polygon.rotated(*angle))
            }
            _ => None,
        };
        if let Some(shape) = shape {
            return shape.map(Value::Polygon).map_err(|message| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: message.into(),
            });
        }
        let number = match (builtin, arguments) {
            (Builtin::Random, [Value::Number(seed), Value::Number(index)]) => {
                crate::noise::random(*seed, *index).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "random requires nonnegative exact integer seed and index".into(),
                })?
            }
            (Builtin::Noise, [Value::Number(x), Value::Number(seed)]) => {
                crate::noise::noise(*x, *seed).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message:
                        "noise requires a bounded coordinate and nonnegative exact integer seed"
                            .into(),
                })?
            }

            (Builtin::Cross, [Value::Vector(a), Value::Vector(b)])
                if a.len() == 3 && b.len() == 3 =>
            {
                return self.vector(
                    vec![
                        a[1] * b[2] - a[2] * b[1],
                        a[2] * b[0] - a[0] * b[2],
                        a[0] * b[1] - a[1] * b[0],
                    ],
                    span,
                );
            }
            (Builtin::Lerp, [Value::Vector(a), Value::Vector(b), Value::Number(t)])
                if a.len() == b.len() =>
            {
                return self.vector(
                    a.iter()
                        .zip(b)
                        .map(|(a, b)| a * (1.0 - t) + b * t)
                        .collect(),
                    span,
                );
            }
            (Builtin::Lerp, [Value::Number(a), Value::Number(b), Value::Number(t)]) => {
                a * (1.0 - t) + b * t
            }
            (Builtin::Clamp, [Value::Number(x), Value::Number(low), Value::Number(high)])
                if low <= high =>
            {
                x.clamp(*low, *high)
            }
            (Builtin::Smoothstep, [Value::Number(low), Value::Number(high), Value::Number(x)])
                if low < high =>
            {
                let t = if x <= low {
                    0.0
                } else if x >= high {
                    1.0
                } else {
                    let width = high - low;
                    if width.is_finite() {
                        (x - low) / width
                    } else {
                        // Halving first keeps opposite finite extremes representable.
                        (x * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)
                    }
                };
                t * t * (3.0 - 2.0 * t)
            }

            (Builtin::Dot, [Value::Vector(a), Value::Vector(b)]) if a.len() == b.len() => {
                a.iter().zip(b).map(|(a, b)| a * b).sum()
            }
            (Builtin::Length | Builtin::Normalize, [Value::Vector(v)]) => {
                if builtin == Builtin::Normalize {
                    let scale = v.iter().fold(0.0_f64, |scale, x| scale.max(x.abs()));
                    if scale == 0.0 || v.iter().any(|x| !x.is_finite()) {
                        return self.error(span, "Cannot normalize this vector");
                    }
                    let length = v.iter().fold(0.0_f64, |length, x| length.hypot(x / scale));
                    return self.vector(v.iter().map(|x| (x / scale) / length).collect(), span);
                }
                v.iter().fold(0.0_f64, |length, x| length.hypot(*x))
            }
            (Builtin::Sin, [Value::Number(x) | Value::Angle(x)]) => x.sin(),
            (Builtin::Cos, [Value::Number(x) | Value::Angle(x)]) => x.cos(),
            (Builtin::Sqrt, [Value::Number(x)]) => x.sqrt(),
            (Builtin::Deg, [Value::Number(x)]) => x.to_radians(),
            _ => return self.error(span, "Invalid mathematical arguments"),
        };
        if number.is_finite() {
            Ok(Value::Number(number))
        } else {
            self.error(span, "Non-finite mathematical result")
        }
    }
    fn call(
        &mut self,
        function: Value<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        self.check(span)?;
        if self.remaining == 0 {
            return self.error(span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        // Capture only borrowed/copyable labels on the successful path. Stack
        // strings and source coordinates are needed only when a call fails.
        let (name, builtin) = match &function {
            Value::Function(f) => (f.name.unwrap_or("<lambda>").to_owned(), None),
            Value::BytecodeFunction(f) => (
                f.function.name.as_deref().unwrap_or("<lambda>").to_owned(),
                None,
            ),
            Value::Host(f) => (f.name.to_owned(), None),
            Value::Builtin(b) => (String::new(), Some(*b)),
            _ => ("<non-callable>".to_owned(), None),
        };
        let module = self.module;
        let result = match function {
            Value::Function(function) => self.call_user(function, arguments, span),
            Value::BytecodeFunction(closure) => {
                let mut vm = crate::bytecode::Vm::new(self.remaining);
                let static_args: Vec<_> = arguments.iter().filter_map(|a| a.to_static()).collect();
                if static_args.len() != arguments.len() {
                    return self.error(span, "Non-static arguments in bytecode call");
                }
                match vm.run_closure(&closure, &static_args) {
                    Ok(val) => {
                        self.remaining = vm.fuel();
                        Ok(val)
                    }
                    Err(e) => self.error(span, &e.message),
                }
            }
            other => self.call_inner(other, arguments, span),
        };
        result.map_err(|error| {
            let name = builtin.map_or(name, |b| format!("{b:?}"));
            self.call_error(error, name, module, span)
        })
    }
    fn call_error(
        &self,
        mut error: RuntimeError,
        name: String,
        module: Option<&'s str>,
        span: Span,
    ) -> RuntimeError {
        let source = self.sources.get(&module).copied().unwrap_or("");
        let prefix = source.get(..span.start).unwrap_or("");
        let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        if error.location.is_none()
            && let Some((_, source)) = self
                .sources
                .iter()
                .find(|(module, _)| module.map(str::to_owned) == error.module)
        {
            error = error.locate(source);
        }
        error.stack.push(CallFrame {
            function: name,
            module: module.map(str::to_owned),
            span,
            line,
            column,
        });
        error
    }
    fn call_inner(
        &mut self,
        function: Value<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        if let Value::Host(function) = function {
            if arguments.len() != function.parameters.len()
                || !function
                    .parameters
                    .iter()
                    .zip(arguments.iter())
                    .all(|(ty, value)| ty.accepts(value))
            {
                return self.error(span, "Host function arguments do not match its signature");
            }
            let result = match self.contextual.get(&Rc::as_ptr(&function)) {
                Some(callback) => callback(&arguments, self.cancellation),
                None => (function.callback)(&arguments, self.cancellation),
            }
            .map_err(|message| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message,
            })?;
            self.check(span)?;
            self.host_value_size(&result, span, 0)?;
            if !function.result.accepts(&result) {
                return self.error(span, "Host function returned an invalid value");
            }
            return Ok(result);
        }
        if let Value::Builtin(builtin) = function {
            for &(index, expected) in builtin.callback_arities() {
                let valid = match arguments.get(index) {
                    Some(Value::Function(function)) => Some(function.parameters.len() == expected),
                    Some(Value::BytecodeFunction(function)) => {
                        Some(function.function.arity == expected)
                    }
                    Some(Value::Host(function)) => Some(function.parameters.len() == expected),
                    Some(Value::Builtin(function)) => Some(function.arity().contains(&expected)),
                    _ => None,
                };
                if valid == Some(false) {
                    return self.error(span, "Callback argument count does not match operation");
                }
            }
            if builtin == Builtin::Iter {
                let [source] = arguments.as_ref() else {
                    return self.error(span, "iter requires one list or sequence");
                };
                let cursor = self.sequence_cursor(source.clone(), span)?;
                return Ok(Value::Sequence(cursor.sequence));
            }
            if builtin == Builtin::Collect {
                let [source, Value::Number(limit)] = arguments.as_ref() else {
                    return self.error(span, "collect requires a sequence and maximum item count");
                };
                if !limit.is_finite()
                    || *limit < 0.0
                    || limit.fract() != 0.0
                    || *limit >= usize::MAX as f64
                {
                    return self.error(
                        span,
                        "Collection limit must be a nonnegative representable integer",
                    );
                }
                if *limit as usize > self.max_collection_items {
                    return self.error(span, "Collection item limit exceeded");
                }
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                let mut output = Vec::new();
                for _ in 0..*limit as usize {
                    let Some(item) = self.sequence_next(&mut cursor, span)? else {
                        break;
                    };
                    self.charge(1, span)?;
                    output.push(item);
                }
                return Ok(Value::List(output));
            }
            if matches!(builtin, Builtin::Map | Builtin::Filter)
                && matches!(
                    arguments.first(),
                    Some(Value::Range { .. } | Value::Sequence(_) | Value::BytecodeSequence(_))
                )
            {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "map/filter require a sequence and callback");
                };
                if !matches!(
                    callback,
                    Value::Function(_)
                        | Value::BytecodeFunction(_)
                        | Value::Builtin(_)
                        | Value::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let cursor = self.sequence_cursor(source.clone(), span)?;
                self.charge(cursor.sequence.stages.len() + 1, span)?;
                let mut sequence = (*cursor.sequence).clone();
                sequence
                    .stages
                    .append(
                        &self.memory,
                        SequenceStage {
                            callback: callback.clone(),
                            filter: builtin == Builtin::Filter,
                            span,
                            module: self.module,
                        },
                    )
                    .map_err(|e| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Runtime sequence allocation failed: {e:?}"),
                    })?;
                return Ok(Value::Sequence(Rc::new(sequence)));
            }
            if matches!(builtin, Builtin::Any | Builtin::All) {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "any/all require a list or sequence and predicate");
                };
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                if !matches!(
                    callback,
                    Value::Function(_)
                        | Value::BytecodeFunction(_)
                        | Value::Builtin(_)
                        | Value::Host(_)
                ) {
                    return self.error(span, "Predicate must be callable");
                }
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let Value::Bool(result) = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                    else {
                        return self.error(span, "Predicate must return a boolean");
                    };
                    if result == (builtin == Builtin::Any) {
                        return Ok(Value::Bool(result));
                    }
                }
                return Ok(Value::Bool(builtin == Builtin::All));
            }
            if builtin == Builtin::Get {
                if arguments.len() != 2 {
                    return self.error(span, "get requires a collection and key");
                }
                let value = match (&arguments[0], &arguments[1]) {
                    (Value::Record(fields), Value::String(key)) => fields.get(key).cloned(),
                    (Value::List(items) | Value::Tuple(items), Value::Number(index)) => {
                        if !index.is_finite() || index.fract() != 0.0 || *index < 0.0 {
                            return self
                                .error(span, "Collection index must be a non-negative integer");
                        }
                        if *index >= items.len() as f64 {
                            None
                        } else {
                            items.get(*index as usize).cloned()
                        }
                    }
                    _ => {
                        return self.error(
                            span,
                            "get requires a record and string key, or list/tuple and integer index",
                        );
                    }
                };
                return Ok(match value {
                    Some(value) => Value::Variant("Some", vec![value]),
                    None => Value::Variant("None", vec![]),
                });
            }
            if builtin == Builtin::Len {
                if arguments.len() != 1 {
                    return self.error(span, "len requires one argument");
                }
                let count = match &arguments[0] {
                    Value::List(items) | Value::Tuple(items) => items.len(),
                    Value::Record(fields) => fields.len(),
                    Value::String(text) => {
                        self.charge(text.len(), span)?;
                        text.chars().count()
                    }
                    _ => return self.error(span, "len requires a list, tuple, record or string"),
                };
                return Ok(Value::Number(count as f64));
            }
            if builtin == Builtin::Assert {
                if !builtin.arity().contains(&arguments.len()) {
                    return self.error(span, "Incorrect argument count");
                }
                let message = match arguments.get(1) {
                    None => "Assertion failed",
                    Some(Value::String(message)) => message.as_str(),
                    _ => return self.error(span, "Assertion message must be a string"),
                };
                return match arguments[0] {
                    Value::Bool(true) => Ok(Value::Null),
                    Value::Bool(false) => self.error(span, message),
                    _ => self.error(span, "Assertion condition must be a boolean"),
                };
            }
            if !builtin.arity().contains(&arguments.len()) {
                return self.error(span, "Incorrect builtin argument count");
            }

            if builtin == Builtin::JsonParse {
                if arguments.len() != 1 {
                    return self.error(span, "json_parse requires one argument");
                }
                let Value::String(text) = &arguments[0] else {
                    return self.error(span, "json_parse requires a string");
                };
                self.charge(text.len(), span)?;
                return Ok(match json::parse(text) {
                    Ok(val) => Value::Variant("Ok", vec![val]),
                    Err(err) => Value::Variant("Err", vec![Value::String(err)]),
                });
            }
            if builtin == Builtin::JsonStringify {
                if arguments.len() != 1 {
                    return self.error(span, "json_stringify requires one argument");
                }
                let encoded = json::stringify(&arguments[0]).map_err(|e| {
                    self.call_error(
                        RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: format!("json_stringify failed: {e}"),
                        },
                        "json_stringify".into(),
                        self.module,
                        span,
                    )
                })?;
                self.charge(encoded.len(), span)?;
                return Ok(Value::String(encoded));
            }

            if builtin == Builtin::Trim {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "trim requires a string");
                };
                let res = s.trim().to_string();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::TrimStart {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "trim_start requires a string");
                };
                let res = s.trim_start().to_string();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::TrimEnd {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "trim_end requires a string");
                };
                let res = s.trim_end().to_string();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::ToLower {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "to_lower requires a string");
                };
                let res = s.to_lowercase();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::ToUpper {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "to_upper requires a string");
                };
                let res = s.to_uppercase();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::StartsWith {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "starts_with requires a string as first argument");
                };
                let Value::String(prefix) = &arguments[1] else {
                    return self.error(span, "starts_with requires a string prefix");
                };
                return Ok(Value::Bool(s.starts_with(prefix.as_str())));
            }
            if builtin == Builtin::EndsWith {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "ends_with requires a string as first argument");
                };
                let Value::String(suffix) = &arguments[1] else {
                    return self.error(span, "ends_with requires a string suffix");
                };
                return Ok(Value::Bool(s.ends_with(suffix.as_str())));
            }
            if builtin == Builtin::Contains {
                match (&arguments[0], &arguments[1]) {
                    (Value::String(s), Value::String(needle)) => {
                        return Ok(Value::Bool(s.contains(needle.as_str())));
                    }
                    (Value::List(items), needle) => {
                        return Ok(Value::Bool(items.contains(needle)));
                    }
                    (Value::Record(fields), Value::String(key)) => {
                        return Ok(Value::Bool(fields.contains_key(key)));
                    }
                    _ => {
                        return self.error(
                            span,
                            "contains requires (string, string), (list, value), or (record, string)",
                        );
                    }
                }
            }
            if builtin == Builtin::Replace {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "replace requires a string as first argument");
                };
                let Value::String(from) = &arguments[1] else {
                    return self.error(span, "replace requires a string from-pattern");
                };
                let Value::String(to) = &arguments[2] else {
                    return self.error(span, "replace requires a string to-pattern");
                };
                let res = s.replace(from.as_str(), to.as_str());
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::Split {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "split requires a string as first argument");
                };
                let Value::String(delimiter) = &arguments[1] else {
                    return self.error(span, "split requires a string delimiter");
                };
                let raw_parts: Vec<String> = if delimiter.is_empty() {
                    s.chars().map(|c| c.to_string()).collect()
                } else {
                    s.split(delimiter.as_str())
                        .map(|part| part.to_string())
                        .collect()
                };
                self.collection_growth(0, raw_parts.len(), span)?;
                let mut parts = Vec::with_capacity(raw_parts.len());
                for part in raw_parts {
                    self.string_growth(0, part.len(), span)?;
                    parts.push(Value::String(part));
                }
                return Ok(Value::List(parts));
            }
            if builtin == Builtin::Join {
                let Value::List(items) = &arguments[0] else {
                    return self.error(span, "join requires a list as first argument");
                };
                let Value::String(separator) = &arguments[1] else {
                    return self.error(span, "join requires a string separator");
                };
                let mut out = String::new();
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.string_growth(out.len(), separator.len(), span)?;
                        out.push_str(separator);
                    }
                    let mut piece = String::new();
                    format_value_for_display(item, &mut piece, 0);
                    self.string_growth(out.len(), piece.len(), span)?;
                    out.push_str(&piece);
                }
                return Ok(Value::String(out));
            }

            // Explicit region escape hatch: identity at runtime, the promotion
            // itself is automatic; analysis recognizes the wrapper as intent.
            if builtin == Builtin::Promote {
                let mut arguments = arguments.into_owned();
                return Ok(arguments.swap_remove(0));
            }

            // First-class region handles: `arena(name, budget?)` builds a plain
            // descriptor record, `arena_run` executes a callback inside a fresh
            // frame of that region, `arena_stats` reads its aggregated metrics.
            if builtin == Builtin::Arena {
                let mut arguments = arguments.into_owned().into_iter();
                let Some(Value::String(name)) = arguments.next() else {
                    return self.error(span, "arena requires a name string");
                };
                if name.is_empty() {
                    return self.error(span, "arena name must not be empty");
                }
                let budget = match arguments.next() {
                    None | Some(Value::Null) => Value::Null,
                    Some(Value::Number(value))
                        if value.is_finite() && value.fract() == 0.0 && value >= 0.0 =>
                    {
                        Value::Number(value)
                    }
                    _ => {
                        return self.error(span, "arena budget must be a non-negative integer");
                    }
                };
                self.collection_growth(0, 2, span)?;
                return Ok(Value::Record(BTreeMap::from([
                    ("name".into(), Value::String(name)),
                    ("budget".into(), budget),
                ])));
            }
            if matches!(builtin, Builtin::ArenaRun | Builtin::ArenaStats) {
                let mut arguments = arguments.into_owned().into_iter();
                let descriptor = arguments.next().unwrap();
                let Value::Record(fields) = &descriptor else {
                    return self.error(span, "an arena record from arena(...) is required");
                };
                let Some(Value::String(name)) = fields.get("name") else {
                    return self.error(span, "arena record requires a name string");
                };
                if builtin == Builtin::ArenaStats {
                    let stats = self
                        .region_stats
                        .iter()
                        .find(|entry| entry.name.as_deref() == Some(name.as_str()));
                    let metric = |pick: fn(&RegionStats) -> u64| {
                        stats.map_or(0.0, |entry| pick(entry) as f64)
                    };
                    self.collection_growth(0, 7, span)?;
                    return Ok(Value::Record(BTreeMap::from([
                        ("name".into(), Value::String(name.clone())),
                        ("entries".into(), Value::Number(metric(|s| s.entries))),
                        ("allocated".into(), Value::Number(metric(|s| s.allocated))),
                        (
                            "reused_slots".into(),
                            Value::Number(metric(|s| s.reused_slots)),
                        ),
                        (
                            "peak_live".into(),
                            Value::Number(stats.map_or(0.0, |s| s.peak_live as f64)),
                        ),
                        ("promoted".into(), Value::Number(metric(|s| s.promoted))),
                        (
                            "suggested_budget".into(),
                            Value::Number(stats.map_or(0.0, |s| s.suggested_budget() as f64)),
                        ),
                    ])));
                }
                let callback = arguments.next().unwrap();
                if !matches!(
                    callback,
                    Value::Function(_) | Value::Builtin(_) | Value::Host(_)
                ) {
                    return self.error(span, "arena_run requires a callable");
                }
                let budget = match fields.get("budget") {
                    None | Some(Value::Null) => None,
                    Some(Value::Number(value))
                        if value.is_finite() && value.fract() == 0.0 && *value >= 0.0 =>
                    {
                        Some(*value as usize)
                    }
                    _ => {
                        return self.error(span, "arena budget must be a non-negative integer");
                    }
                };
                let region = self.enter_region(budget, Some(name));
                // The callback's own implicit call region would intercept its
                // scratch cells before the arena sees them; the arena frame
                // replaces it for exactly this one call.
                self.skip_call_region = true;
                let result = self.call(callback.clone(), Cow::Borrowed(&[]), span);
                self.skip_call_region = false;
                self.exit_region(region, span)?;
                return result;
            }

            if matches!(
                builtin,
                Builtin::Some | Builtin::None | Builtin::Ok | Builtin::Err
            ) {
                let (tag, count) = match builtin {
                    Builtin::Some => ("Some", 1),
                    Builtin::None => ("None", 0),
                    Builtin::Ok => ("Ok", 1),
                    _ => ("Err", 1),
                };
                if arguments.len() != count {
                    return self.error(span, "Invalid variant argument count");
                }
                return Ok(Value::Variant(tag, arguments.into_owned()));
            }

            if builtin == Builtin::Mesh {
                let [Value::List(points), Value::List(faces)] = arguments.as_ref() else {
                    return self.error(span, "mesh requires vertex and triangle lists");
                };
                self.collection_growth(0, points.len(), span)?;
                self.collection_growth(0, faces.len(), span)?;
                let mut vertices = Vec::new();
                for point in points {
                    self.charge(1, span)?;
                    let Value::Vector(v) = point else {
                        return self.error(span, "Mesh vertex must be vec3");
                    };
                    if v.len() != 3 {
                        return self.error(span, "Mesh vertex must be vec3");
                    }
                    vertices.push([v[0], v[1], v[2]]);
                }
                let mut triangles = Vec::new();
                for face in faces {
                    self.charge(1, span)?;
                    let Value::List(indices) = face else {
                        return self.error(span, "Mesh triangle must be a list of three indices");
                    };
                    if indices.len() != 3 {
                        return self.error(span, "Mesh triangle must have three indices");
                    }
                    let mut triangle = [0; 3];
                    for (slot, index) in triangle.iter_mut().zip(indices) {
                        let Value::Number(index) = index else {
                            return self.error(span, "Mesh index must be an integer");
                        };
                        if !index.is_finite()
                            || index.fract() != 0.0
                            || *index < 0.0
                            || *index >= vertices.len() as f64
                        {
                            return self.error(span, "Mesh index out of bounds or non-integer");
                        }
                        *slot = *index as usize;
                    }
                    triangles.push(triangle);
                }
                return crate::Mesh::new(vertices, triangles)
                    .map(|mesh| Value::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Transform {
                let [Value::Mesh(mesh), Value::Matrix(matrix)] = arguments.as_ref() else {
                    return self.error(span, "transform requires a mesh and mat4");
                };
                self.collection_growth(0, mesh.vertices().len(), span)?;
                self.collection_growth(0, mesh.triangles().len(), span)?;
                let mut vertices = Vec::new();
                for vertex in mesh.vertices() {
                    self.charge(1, span)?;
                    let transformed = matrix.apply(vertex, true).ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Mesh transformation overflow".into(),
                    })?;
                    vertices.push([transformed[0], transformed[1], transformed[2]]);
                }
                self.charge(mesh.triangles().len(), span)?;
                return crate::Mesh::new(vertices, mesh.triangles().to_vec())
                    .map(|mesh| Value::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::GridMesh {
                let [Value::List(xs), Value::List(ys), callback] = arguments.as_ref() else {
                    return self.error(
                        span,
                        "grid_mesh requires x values, y values and a point function",
                    );
                };
                if xs.len() < 2 || ys.len() < 2 {
                    return self.error(span, "Grid requires at least two coordinates per axis");
                }
                let count = xs.len().checked_mul(ys.len()).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "Grid size overflow".into(),
                })?;
                self.collection_growth(0, count, span)?;
                let faces = (xs.len() - 1)
                    .checked_mul(ys.len() - 1)
                    .and_then(|n| n.checked_mul(2))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Grid size overflow".into(),
                    })?;
                self.collection_growth(0, faces, span)?;
                self.charge(count, span)?;
                let mut vertices = Vec::new();
                for y in ys {
                    for x in xs {
                        let point = self.call(
                            callback.clone(),
                            Cow::Borrowed(&[x.clone(), y.clone()]),
                            span,
                        )?;
                        let Value::Vector(point) = point else {
                            return self.error(span, "Grid callback must return vec3");
                        };
                        if point.len() != 3 {
                            return self.error(span, "Grid callback must return vec3");
                        }
                        vertices.push([point[0], point[1], point[2]]);
                    }
                }
                let mut triangles = Vec::new();
                for row in 0..ys.len() - 1 {
                    for column in 0..xs.len() - 1 {
                        self.charge(2, span)?;
                        let a = row * xs.len() + column;
                        let b = a + 1;
                        let c = a + xs.len();
                        let d = c + 1;
                        triangles.extend([[a, b, d], [a, d, c]]);
                    }
                }
                return crate::Mesh::new(vertices, triangles)
                    .map(|mesh| Value::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Zip {
                let [Value::List(a), Value::List(b)] = arguments.as_ref() else {
                    return self.error(span, "zip requires two lists");
                };
                self.collection_growth(0, a.len().min(b.len()), span)?;
                if !a.is_empty() && !b.is_empty() {
                    self.collection_growth(0, 2, span)?;
                }
                let mut values = Vec::new();
                for (a, b) in a.iter().zip(b) {
                    self.check(span)?;
                    if self.remaining == 0 {
                        return self.error(span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    values.push(Value::Tuple(vec![a.clone(), b.clone()]));
                }
                return Ok(Value::List(values));
            }
            if matches!(builtin, Builtin::Range | Builtin::RangeIter) {
                let (start, end, step) = match arguments.as_ref() {
                    [Value::Number(start), Value::Number(end)] => (*start, *end, 1.0),
                    [
                        Value::Number(start),
                        Value::Number(end),
                        Value::Number(step),
                    ] => (*start, *end, *step),
                    _ => return self.error(span, "range expects start, end and optional step"),
                };
                if step == 0.0 {
                    return self.error(span, "Range step must be nonzero");
                }
                if !start.is_finite() || !end.is_finite() || !step.is_finite() {
                    return self.error(span, "Range arguments must be finite");
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
                    self.check(span)?;
                    if self.remaining == 0 {
                        return self.error(span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    if values.len() >= self.max_collection_items {
                        return self.error(span, "Collection item limit exceeded");
                    }
                    values.push(Value::Number(current));
                    let next = (values.len() as f64).mul_add(step, start);
                    if !next.is_finite() || next == current {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    current = next;
                }
                return Ok(Value::List(values));
            }

            if matches!(builtin, Builtin::GroupBy | Builtin::FoldBy) {
                let mut arguments = arguments.into_owned().into_iter();
                let source = arguments.next().unwrap();
                let callback = arguments.next().unwrap();
                if !matches!(
                    callback,
                    Value::Function(_)
                        | Value::BytecodeFunction(_)
                        | Value::Builtin(_)
                        | Value::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let initial = arguments.next().unwrap_or(Value::Null);
                let reducer = arguments.next();
                if reducer.as_ref().is_some_and(|f| {
                    !matches!(
                        f,
                        Value::Function(_)
                            | Value::BytecodeFunction(_)
                            | Value::Builtin(_)
                            | Value::Host(_)
                    )
                }) {
                    return self.error(span, "Reducer must be callable");
                }
                let field = if reducer.is_some() { "value" } else { "values" };
                let mut cursor = self.sequence_cursor(source, span)?;
                let mut positions = BTreeMap::<String, usize>::new();
                let mut groups = Vec::<(String, Value<'s>)>::new();
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let key = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?;
                    let Value::String(key) = key else {
                        return self.error(span, "Group key must be a string");
                    };
                    self.string_growth(0, key.len(), span)?;
                    let position = if let Some(position) = positions.get(&key) {
                        *position
                    } else {
                        self.collection_growth(groups.len(), 1, span)?;
                        self.collection_growth(0, 2, span)?;
                        self.string_growth(0, field.len(), span)?;
                        let position = groups.len();
                        positions.insert(key.clone(), position);
                        groups.push((
                            key,
                            if reducer.is_some() {
                                initial.clone()
                            } else {
                                Value::List(Vec::new())
                            },
                        ));
                        position
                    };
                    if let Some(reducer) = &reducer {
                        let accumulator = std::mem::replace(&mut groups[position].1, Value::Null);
                        groups[position].1 =
                            self.call(reducer.clone(), Cow::Borrowed(&[accumulator, item]), span)?;
                    } else if let Value::List(values) = &mut groups[position].1 {
                        self.collection_growth(values.len(), 1, span)?;
                        values.push(item);
                    }
                }
                let mut output = Vec::new();
                for (key, values) in groups {
                    self.charge(1, span)?;
                    output.push(Value::Record(BTreeMap::from([
                        ("key".into(), Value::String(key)),
                        (field.into(), values),
                    ])));
                }
                return Ok(Value::List(output));
            }
            if !matches!(
                builtin,
                Builtin::Map | Builtin::FlatMap | Builtin::Filter | Builtin::Fold
            ) {
                return self.math(builtin, &arguments, span);
            }
            let expected = if builtin == Builtin::Fold { 3 } else { 2 };
            if arguments.len() != expected {
                return self.error(span, "Incorrect argument count");
            }
            let mut arguments = arguments.into_owned().into_iter();
            let source = arguments.next().unwrap();
            let mut cursor = self.sequence_cursor(source, span)?;
            let mut accumulator = if builtin == Builtin::Fold {
                arguments.next().unwrap()
            } else {
                Value::Null
            };
            let callback = arguments.next().unwrap();
            if !matches!(
                callback,
                Value::Function(_)
                    | Value::BytecodeFunction(_)
                    | Value::Builtin(_)
                    | Value::Host(_)
            ) {
                return self.error(span, "Callback must be callable");
            }
            let mut output = Vec::new();
            while let Some(item) = self.sequence_next(&mut cursor, span)? {
                let result = if builtin == Builtin::Fold {
                    self.call(
                        callback.clone(),
                        Cow::Borrowed(&[accumulator, item.clone()]),
                        span,
                    )?
                } else {
                    self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                };
                accumulator = Value::Null;
                match builtin {
                    Builtin::Map => {
                        self.collection_growth(output.len(), 1, span)?;
                        output.push(result);
                    }
                    Builtin::FlatMap => match result {
                        Value::List(values) => {
                            self.charge(values.len(), span)?;
                            self.collection_growth(output.len(), values.len(), span)?;
                            output.extend(values);
                        }
                        _ => return self.error(span, "Flat map callback must return a list"),
                    },
                    Builtin::Filter => match result {
                        Value::Bool(true) => {
                            self.collection_growth(output.len(), 1, span)?;
                            output.push(item);
                        }
                        Value::Bool(false) => {}
                        _ => return self.error(span, "Filter callback must return a boolean"),
                    },
                    Builtin::Fold => accumulator = result,
                    _ => unreachable!(),
                }
            }
            return if builtin == Builtin::Fold {
                Ok(accumulator)
            } else {
                Ok(Value::List(output))
            };
        }
        self.error(span, "Value is not callable")
    }

    fn constructor(
        &self,
        name: &Name<'s>,
        type_name: String,
        variant: Option<String>,
        fields: Vec<(String, ValueType)>,
        types: Vec<ValueType>,
    ) -> Result<Value<'s>> {
        let count = if variant.is_none() { 1 } else { types.len() };
        Ok(Value::Function(Rc::new(Closure {
            module: self.module,
            name: Some(name.text),
            parameters: (0..count)
                .map(|_| Expr {
                    span: name.span,
                    kind: ExprKind::Name(Name {
                        text: "_",
                        span: name.span,
                    }),
                })
                .collect(),
            parameter_types: if variant.is_none() {
                vec![None]
            } else {
                types.into_iter().map(Some).collect()
            },
            result_type: Some(ValueType::User(type_name.clone())),
            body: Rc::new(FunctionBody::Constructor {
                type_name,
                variant,
                fields,
            }),
            environment: memory::Shared::new(&self.memory, Environment::new(&self.memory))
                .map_err(|e| self.environment_error(name.span, e))?,
            references: Rc::new(Vec::new()),
        })))
    }
    fn call_constructor(
        &mut self,
        function: &Closure<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        if let FunctionBody::Constructor {
            type_name,
            variant,
            fields,
        } = &*function.body
        {
            if variant.is_none() {
                let Value::Record(supplied) = &arguments[0] else {
                    return self.error(span, "Struct constructor requires a record");
                };
                if supplied.len() != fields.len()
                    || fields
                        .iter()
                        .any(|(name, ty)| supplied.get(name).is_none_or(|value| !ty.accepts(value)))
                {
                    return self.error(span, "Struct fields do not match declaration");
                }
            }
            self.collection_growth(
                0,
                if variant.is_none() {
                    fields.len()
                } else {
                    arguments.len()
                },
                span,
            )?;
            for value in arguments.iter() {
                self.host_value_size(value, span, 0)?;
            }
            return Ok(Value::UserData(Box::new(UserData {
                type_name: type_name.clone(),
                variant: variant.clone(),
                values: arguments.into_owned(),
            })));
        }
        unreachable!("constructor body")
    }
    fn call_user(
        &mut self,
        function: Rc<Closure<'s>>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        // Consumed upfront so no early return can leak the one-shot flag.
        let skip_call_region = std::mem::take(&mut self.skip_call_region);
        if arguments.len() != function.parameters.len() {
            return self.error(span, "Incorrect argument count");
        }
        if function
            .parameter_types
            .iter()
            .zip(arguments.iter())
            .any(|(ty, value)| ty.as_ref().is_some_and(|ty| !ty.accepts(value)))
        {
            return self.error(span, "Argument does not match its type annotation");
        }
        if matches!(&*function.body, FunctionBody::Constructor { .. }) {
            return self.call_constructor(&function, arguments, span);
        }
        let mut environment = self.take_call_environment(&function.environment);
        if let Some(name) = function.name
            && let Err(allocation) = environment.insert(name, Value::Function(function.clone()))
        {
            let error = self.environment_error(span, allocation);
            self.recycle_call_environment(environment);
            return Err(error);
        }
        let previous_module = self.module;
        let previous_references =
            std::mem::replace(&mut self.current_references, function.references.clone());
        self.module = function.module;
        for (parameter, argument) in function.parameters.iter().zip(arguments.iter()) {
            if let Err(error) = self.bind_pattern(parameter, argument, &mut environment) {
                self.module = previous_module;
                self.current_references = previous_references;
                self.recycle_call_environment(environment);
                return Err(error);
            }
        }
        if matches!(&*function.body, FunctionBody::Block(_)) && self.depth >= self.max_depth {
            self.module = previous_module;
            self.current_references = previous_references;
            self.recycle_call_environment(environment);
            return self.error(span, "Execution limit exceeded");
        }
        // Implicit per-call region: temporary cells die in bulk when the call
        // returns, while captured ones promote into the caller's region.
        // Skipped when arena_run already supplied this call's region.
        let call_region = if skip_call_region {
            None
        } else {
            Some(self.enter_region(None, None))
        };
        let result = match &*function.body {
            FunctionBody::Constructor { .. } => unreachable!("constructors return above"),
            FunctionBody::Expression(body) => self.expr(body, &environment),
            FunctionBody::Block(body) => {
                self.depth += 1;
                let result =
                    self.statements(&body.stmts, &mut environment)
                        .map(|(value, returned)| {
                            if returned == Flow::Return {
                                value
                            } else {
                                Value::Null
                            }
                        });
                self.depth -= 1;
                result
            }
        };
        let result = if result.is_err() {
            if let Some(value) = self.early_return.take() {
                Ok(value)
            } else {
                result
            }
        } else {
            result
        };
        self.module = previous_module;
        self.current_references = previous_references;
        self.recycle_call_environment(environment);
        // exit_region reclaims dead cells internally before sweeping; without
        // a call region (arena_run owns it) reclaim explicitly.
        if let Some(call_region) = call_region {
            self.exit_region(call_region, span)?;
        } else {
            self.reclaim_cells();
        }
        let value = result?;
        if function
            .result_type
            .as_ref()
            .is_some_and(|ty| !ty.accepts(&value))
        {
            return self.error(span, "Return value does not match its type annotation");
        }
        Ok(value)
    }
}

/// The runtime registration table, shared with editor name validation.
pub fn builtin_catalog() -> &'static [(&'static str, Builtin)] {
    &[
        ("assert", Builtin::Assert),
        ("json_parse", Builtin::JsonParse),
        ("json_stringify", Builtin::JsonStringify),
        ("trim", Builtin::Trim),
        ("trim_start", Builtin::TrimStart),
        ("trim_end", Builtin::TrimEnd),
        ("split", Builtin::Split),
        ("join", Builtin::Join),
        ("starts_with", Builtin::StartsWith),
        ("ends_with", Builtin::EndsWith),
        ("contains", Builtin::Contains),
        ("replace", Builtin::Replace),
        ("to_lower", Builtin::ToLower),
        ("to_upper", Builtin::ToUpper),
        ("len", Builtin::Len),
        ("get", Builtin::Get),
        ("any", Builtin::Any),
        ("all", Builtin::All),
        ("degrees", Builtin::Degrees),
        ("radians", Builtin::Radians),
        ("Some", Builtin::Some),
        ("None", Builtin::None),
        ("Ok", Builtin::Ok),
        ("Err", Builtin::Err),
        ("random", Builtin::Random),
        ("noise", Builtin::Noise),
        ("transform", Builtin::Transform),
        ("grid_mesh", Builtin::GridMesh),
        ("mesh", Builtin::Mesh),
        ("slerp", Builtin::Slerp),
        ("axis_angle", Builtin::AxisAngle),
        ("rotation_matrix", Builtin::RotationMatrix),
        ("identity", Builtin::Identity),
        ("promote", Builtin::Promote),
        ("arena", Builtin::Arena),
        ("arena_run", Builtin::ArenaRun),
        ("arena_stats", Builtin::ArenaStats),
        ("translation", Builtin::Translation),
        ("scaling", Builtin::Scaling),
        ("rotation_x", Builtin::RotationX),
        ("rotation_y", Builtin::RotationY),
        ("rotation_z", Builtin::RotationZ),
        ("transform_point", Builtin::TransformPoint),
        ("transform_direction", Builtin::TransformDirection),
        ("polygon", Builtin::Polygon),
        ("translate", Builtin::Translate),
        ("rotate", Builtin::Rotate),
        ("zip", Builtin::Zip),
        ("range", Builtin::Range),
        ("range_iter", Builtin::RangeIter),
        ("iter", Builtin::Iter),
        ("collect", Builtin::Collect),
        ("cross", Builtin::Cross),
        ("lerp", Builtin::Lerp),
        ("clamp", Builtin::Clamp),
        ("smoothstep", Builtin::Smoothstep),
        ("vec2", Builtin::Vec2),
        ("vec3", Builtin::Vec3),
        ("vec4", Builtin::Vec4),
        ("dot", Builtin::Dot),
        ("length", Builtin::Length),
        ("normalize", Builtin::Normalize),
        ("sin", Builtin::Sin),
        ("cos", Builtin::Cos),
        ("sqrt", Builtin::Sqrt),
        ("deg", Builtin::Deg),
        ("map", Builtin::Map),
        ("flat_map", Builtin::FlatMap),
        ("filter", Builtin::Filter),
        ("fold", Builtin::Fold),
        ("group_by", Builtin::GroupBy),
        ("fold_by", Builtin::FoldBy),
    ]
}

#[cfg(test)]
mod remainder_tests {
    use super::exact_remainder;
    #[test]
    fn optimized_remainder_matches_ieee_bits() {
        let edge = [
            0.,
            -0.,
            1.,
            -1.,
            3.,
            -3.,
            0.25,
            -0.25,
            9_007_199_254_740_990.,
            -9_007_199_254_740_990.,
            9_007_199_254_740_991.,
            -9_007_199_254_740_991.,
            9_007_199_254_740_992.,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for a in edge {
            for b in edge {
                compare(a, b);
            }
        }
        let mut state = 0x1234567812345678u64;
        for _ in 0..100_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = (state as i64 % 9_007_199_254_740_991) as f64;
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let b = (state as i64 % 9_007_199_254_740_991) as f64;
            compare(a, b);
            compare(f64::from_bits(state), a);
        }
    }
    fn compare(a: f64, b: f64) {
        let expected = a % b;
        let actual = exact_remainder(a, b);
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert_eq!(actual.to_bits(), expected.to_bits(), "{a} % {b}");
        }
    }
}
