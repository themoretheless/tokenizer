//! Bounded expression evaluator. Closures capture immutable snapshots.
use crate::{Block, Expr, ExprKind, Name, Stmt, StmtKind};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use themoretheless_tokenizer_core::Span;
mod cell_gc;
mod instance;
pub use instance::{OwnedScriptInstance, ScriptState, StateValue};
mod host_value;
mod memory;
pub use host_value::Value;

#[derive(Clone, Debug, PartialEq)]
enum Binding<'s> {
    RuntimeValue(RuntimeValue<'s>),
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
        value: RuntimeValue<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        self.insert_binding(name, Binding::RuntimeValue(value))
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
enum RuntimeValue<'s> {
    HostObject(crate::HostObject),
    /// Angle stored in radians, distinct from an ordinary number.
    Angle(f64),
    Variant(&'static str, Vec<RuntimeValue<'s>>),
    Mesh(Rc<crate::Mesh>),
    Quaternion(Box<crate::Quaternion>),
    String(String),
    Record(BTreeMap<String, RuntimeValue<'s>>),
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
    List(Vec<RuntimeValue<'s>>),
    Tuple(Vec<RuntimeValue<'s>>),
    Function(Rc<Closure<'s>>),
    Builtin(Builtin),
    Host(Rc<HostFunction>),
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
}

/// Repeatable sequence with deferred transformations. Construct through Rush builtins.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence<'s> {
    source: SequenceSource<'s>,
    stages: SequenceStages<'s>,
}

#[derive(Clone, Debug, PartialEq)]
enum SequenceSource<'s> {
    Range { start: f64, end: f64, step: f64 },
    List(memory::Buffer<RuntimeValue<'s>>),
    Host(HostSource),
}

#[derive(Clone, Debug, PartialEq)]
struct SequenceStage<'s> {
    callback: RuntimeValue<'s>,
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
            | Self::Deg => (1, 1),
            Self::Range | Self::RangeIter => (2, 3),
            Self::Assert => (1, 2),
            Self::GridMesh
            | Self::Slerp
            | Self::Lerp
            | Self::Clamp
            | Self::Smoothstep
            | Self::Vec3
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
    body: FunctionBody<'s>,
    name: Option<&'s str>,
    environment: memory::Shared<Environment<'s>>,
    references: Rc<Vec<CaptureReference<'s>>>,
}

#[derive(Clone, Debug, PartialEq)]
enum FunctionBody<'s> {
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

/// Runtime contracts for functions registered by the embedding application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
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
        let primitive = match ty.name.text {
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
        if ty.name.text == "Option" && ty.arguments.len() == 1 {
            return Ok(Self::Option(Box::new(Self::annotation(&ty.arguments[0])?)));
        }
        if ty.name.text == "Result" && ty.arguments.len() == 2 {
            return Ok(Self::Result(
                Box::new(Self::annotation(&ty.arguments[0])?),
                Box::new(Self::annotation(&ty.arguments[1])?),
            ));
        }
        if ty.name.text == "tuple" {
            return Ok(Self::Tuple(
                ty.arguments
                    .iter()
                    .map(Self::annotation)
                    .collect::<Result<Vec<_>>>()?,
            ));
        }
        if ty.name.text == "list" && ty.arguments.len() == 1 {
            return Ok(Self::List(Box::new(Self::annotation(&ty.arguments[0])?)));
        }
        Err(RuntimeError {
            stack: Vec::new(),
            location: None,
            module: None,
            span: ty.name.span,
            message: format!("Unsupported type annotation: {}", ty.name.text),
        })
    }
    pub fn accepts(&self, value: &Value<'_>) -> bool {
        match (self, value) {
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
    fn accepts_runtime(&self, value: &RuntimeValue<'_>) -> bool {
        match (self, value) {
            (Self::HostObject(name), RuntimeValue::HostObject(object)) => {
                *name == object.type_name() && object.is_alive()
            }
            (Self::Sequence, RuntimeValue::Sequence(_) | RuntimeValue::Range { .. }) => true,
            (Self::Angle, RuntimeValue::Angle(angle)) => angle.is_finite(),
            (Self::Option(_), RuntimeValue::Variant("None", values)) => values.is_empty(),
            (Self::Option(ty), RuntimeValue::Variant("Some", values)) => {
                values.len() == 1 && ty.accepts_runtime(&values[0])
            }
            (Self::Result(ty, _), RuntimeValue::Variant("Ok", values))
            | (Self::Result(_, ty), RuntimeValue::Variant("Err", values)) => {
                values.len() == 1 && ty.accepts_runtime(&values[0])
            }
            (Self::Tuple(types), RuntimeValue::Tuple(values)) => {
                types.len() == values.len()
                    && types
                        .iter()
                        .zip(values)
                        .all(|(ty, value)| ty.accepts_runtime(value))
            }
            (Self::Mesh, RuntimeValue::Mesh(_)) => true,
            (Self::Quaternion, RuntimeValue::Quaternion(_)) => true,
            (Self::Matrix4, RuntimeValue::Matrix(matrix)) => {
                matrix.rows().iter().flatten().all(|n| n.is_finite())
            }
            (Self::String, RuntimeValue::String(_)) => true,
            (Self::Number, RuntimeValue::Number(n)) => n.is_finite(),
            (Self::Bool, RuntimeValue::Bool(_))
            | (Self::Null, RuntimeValue::Null)
            | (Self::Polygon, RuntimeValue::Polygon(_)) => true,
            (Self::Vector(size), RuntimeValue::Vector(values)) => {
                values.len() == *size && values.iter().all(|n| n.is_finite())
            }
            (Self::List(element), RuntimeValue::List(values)) => {
                values.iter().all(|v| element.accepts_runtime(v))
            }
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
            Binding::RuntimeValue(value) => Some(value.clone().into()),
            Binding::Cell(cell) => Some(self.runtime.cells[cell.index].0.clone().into()),
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
            self.runtime
                .host_value_size(&value.clone().into(), self.span, 0)?;
        }
        self.enforce_memory_limit()?;
        let value = self.runtime.call(
            function.into(),
            Cow::Owned(arguments.iter().cloned().map(Into::into).collect()),
            self.span,
        )?;
        self.enforce_memory_limit()?;
        Ok(value.into())
    }
}

/// An analyzed program that can be evaluated repeatedly without reparsing.
pub struct Program<'s> {
    parsed: crate::Parse<'s>,
    references: Rc<Vec<CaptureReference<'s>>>,
}
#[derive(Clone, Debug, PartialEq)]
struct CaptureReference<'s> {
    name: &'s str,
    usage: Span,
    definition: Option<Span>,
}
impl<'s> Program<'s> {
    /// All syntactic imports, including imports inside function and branch bodies.
    pub fn imports(&self) -> Vec<Name<'s>> {
        let mut imports = Vec::new();
        let mut pending = vec![self.parsed.module.items.as_slice()];
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
        let mut pending: Vec<(Option<&str>, Vec<Name<'_>>, usize)> =
            vec![(None, self.imports(), 0)];
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
            pending.push((Some(name), program.imports(), 0));
        }
        Ok(())
    }

    pub fn compile(source: &'s str) -> Result<Self> {
        let (parsed, references) = crate::analyze_references(source);
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
            });
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
            parsed,
            references: Rc::new(references),
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
        if limits.max_depth > 64 {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: self.parsed.module.span,
                message: "Maximum evaluation depth cannot exceed 64".into(),
            });
        }
        let memory = memory::Budget::new(usize::MAX);
        let mut runtime = Runtime {
            module: None,
            modules: HashMap::new(),
            module_cache: memory::Slots::new(&memory, 0)
                .expect("empty module cache requires no allocation"),
            loading: memory::Slots::new(&memory, 0)
                .expect("empty module stack requires no allocation"),
            module_globals: Environment::new(&memory),
            memory: memory.clone(),
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
                .insert(name, RuntimeValue::Builtin(builtin))
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
                .insert(function.name, RuntimeValue::Host(function.clone()))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        for (name, value) in inputs {
            runtime.host_value_size(&value.clone().into(), self.parsed.module.span, 0)?;
            if environment.contains_key(name) {
                return runtime.error(
                    self.parsed.module.span,
                    "Invalid or duplicate input parameter",
                );
            }
            environment
                .insert(name, value.clone().into())
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
            runtime
                .references
                .insert(Some(*name), program.references.clone());
        }
        let (initial_value, _) = runtime.statements(&self.parsed.module.items, &mut environment)?;
        runtime.instance_roots = Some(
            environment
                .try_clone()
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?,
        );
        let mut instance = ScriptInstance {
            runtime,
            environment,
            span: self.parsed.module.span,
            initial_value: initial_value.into(),
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
    modules: HashMap<&'s str, crate::Module<'s>>,
    module_cache: memory::Slots<(&'s str, RuntimeValue<'s>)>,
    loading: memory::Slots<&'s str>,
    module_globals: Environment<'s>,
    memory: memory::Budget,
    references: HashMap<Option<&'s str>, Rc<Vec<CaptureReference<'s>>>>,
    current_references: Rc<Vec<CaptureReference<'s>>>,
    cells: memory::Slots<(RuntimeValue<'s>, Option<ValueType>)>,
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
    cancellation: &'a CancellationToken,
    contextual: HashMap<*const HostFunction, ContextualHostCallback>,
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
            self.cells[index] = (RuntimeValue::Null, None);
            self.free_cells
                .push(index)
                .expect("free list reserved with cell");
        }
    }
    fn allocate_cell(
        &mut self,
        value: RuntimeValue<'s>,
        contract: Option<ValueType>,
        span: Span,
    ) -> Result<Rc<CellId>> {
        self.reclaim_cells();
        self.enforce_cell_limit(&value, span)?;
        self.cell_allocations += 1;
        if self.cell_allocations >= self.cell_collection_interval {
            self.collect_cell_cycles(span)?;
            self.cell_allocations = 0;
            self.cell_collection_interval = (self.cells.len() - self.free_cells.len()).max(64);
        }
        let index = if let Some(index) = self.free_cells.pop() {
            self.cells[index] = (value, contract);
            index
        } else {
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
        Ok(id)
    }
    fn host_value_size(
        &mut self,
        value: &RuntimeValue<'s>,
        span: Span,
        depth: usize,
    ) -> Result<()> {
        if self.max_collection_items == usize::MAX && self.max_string_bytes == usize::MAX {
            return Ok(());
        }
        self.charge(1, span)?;
        if depth > 64 {
            return self.error(span, "Host value nesting limit exceeded");
        }
        match value {
            RuntimeValue::String(text) => self.string_growth(0, text.len(), span)?,
            RuntimeValue::List(items) | RuntimeValue::Tuple(items) => {
                self.collection_growth(0, items.len(), span)?;
                for item in items {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            // Option/Result payloads have fixed arity, not collection length.
            // Their nested data still needs validation.
            RuntimeValue::Variant(_, items) => {
                for item in items {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            RuntimeValue::Record(fields) => {
                self.collection_growth(0, fields.len(), span)?;
                for (key, value) in fields {
                    self.string_growth(0, key.len(), span)?;
                    self.host_value_size(value, span, depth + 1)?;
                }
            }
            RuntimeValue::Polygon(polygon) => {
                self.collection_growth(0, polygon.points().len(), span)?
            }
            RuntimeValue::Mesh(mesh) => {
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
    fn annotation(&self, ty: &crate::Type<'_>) -> Result<ValueType> {
        ValueType::annotation(ty).map_err(|mut error| {
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
    fn sequence_cursor(
        &mut self,
        value: RuntimeValue<'s>,
        span: Span,
    ) -> Result<SequenceCursor<'s>> {
        let sequence = match value {
            RuntimeValue::Sequence(sequence) => sequence,
            RuntimeValue::Range { start, end, step } => Rc::new(Sequence {
                source: SequenceSource::Range { start, end, step },
                stages: SequenceStages::default(),
            }),
            RuntimeValue::List(items) => Rc::new(Sequence {
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
    ) -> Result<Option<RuntimeValue<'s>>> {
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
                    let item: RuntimeValue = item.into();
                    self.host_value_size(&item, span, 0)?;
                    if !source.item_type.accepts_runtime(&item) {
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
                    RuntimeValue::Number(current)
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
                        if stage.filter && !matches!(value, RuntimeValue::Bool(_)) {
                            self.error(stage.span, "Filter callback must return a boolean")
                        } else {
                            Ok(value)
                        }
                    });
                self.module = previous_module;
                let result = result?;
                if stage.filter {
                    if result == RuntimeValue::Bool(false) {
                        continue 'candidate;
                    }
                } else {
                    item = result;
                }
            }
            return Ok(Some(item));
        }
    }

    fn equal(
        &mut self,
        left: &RuntimeValue<'_>,
        right: &RuntimeValue<'_>,
        span: Span,
    ) -> Result<bool> {
        let mut pending = vec![(left, right)];
        while let Some((left, right)) = pending.pop() {
            self.charge(1, span)?;
            match (left, right) {
                (
                    RuntimeValue::Sequence(_)
                    | RuntimeValue::Function(_)
                    | RuntimeValue::Host(_)
                    | RuntimeValue::Builtin(_),
                    _,
                )
                | (
                    _,
                    RuntimeValue::Sequence(_)
                    | RuntimeValue::Function(_)
                    | RuntimeValue::Host(_)
                    | RuntimeValue::Builtin(_),
                ) => {
                    return self.error(span, "Functions and lazy sequences cannot be compared");
                }
                (RuntimeValue::Variant(a, left), RuntimeValue::Variant(b, right)) => {
                    if a != b || left.len() != right.len() {
                        return Ok(false);
                    }
                    self.charge(left.len(), span)?;
                    pending.extend(left.iter().zip(right));
                }
                (RuntimeValue::List(a), RuntimeValue::List(b))
                | (RuntimeValue::Tuple(a), RuntimeValue::Tuple(b)) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    self.charge(a.len(), span)?;
                    pending.extend(a.iter().zip(b));
                }
                (RuntimeValue::Record(a), RuntimeValue::Record(b)) => {
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
                (RuntimeValue::String(a), RuntimeValue::String(b)) => {
                    self.charge(a.len().max(b.len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                (RuntimeValue::Mesh(a), RuntimeValue::Mesh(b)) => {
                    self.charge(a.vertices().len().max(b.vertices().len()), span)?;
                    self.charge(a.triangles().len().max(b.triangles().len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                (RuntimeValue::Polygon(a), RuntimeValue::Polygon(b)) => {
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
        value: &RuntimeValue<'s>,
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
                let RuntimeValue::Tuple(values) = value else {
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
                let RuntimeValue::Record(fields) = value else {
                    return Ok(false);
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
                let ExprKind::Name(name) = &callee.kind else {
                    return Ok(false);
                };
                let RuntimeValue::Variant(tag, values) = value else {
                    return Ok(false);
                };
                if name.text != *tag || arguments.len() != values.len() {
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
        value: &RuntimeValue<'s>,
        bindings: &mut Environment<'s>,
    ) -> Result<()> {
        self.check(pattern.span)?;
        match (&pattern.kind, value) {
            (ExprKind::Name(name), _) => {
                if name.text != "_" {
                    bindings
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(pattern.span, e))?;
                }
                Ok(())
            }
            (ExprKind::Map(entries), RuntimeValue::Record(fields)) => {
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
            (ExprKind::Tuple(patterns), RuntimeValue::Tuple(values))
                if patterns.len() == values.len() =>
            {
                for (pattern, value) in patterns.iter().zip(values) {
                    self.bind_pattern(pattern, value, bindings)?;
                }
                Ok(())
            }
            _ => self.error(pattern.span, "RuntimeValue does not match binding pattern"),
        }
    }
    fn scoped_statements(
        &mut self,
        statements: &[Stmt<'s>],
        mut environment: Environment<'s>,
    ) -> Result<(RuntimeValue<'s>, Flow)> {
        let result = self.statements(statements, &mut environment);
        drop(environment);
        self.reclaim_cells();
        result
    }
    fn statements(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<(RuntimeValue<'s>, Flow)> {
        let mut value = RuntimeValue::Null;
        for statement in statements {
            self.check(statement.span)?;
            if self.remaining == 0 {
                return self.error(statement.span, "Execution limit exceeded");
            }
            self.remaining -= 1;
            match &statement.kind {
                StmtKind::Import(name) => {
                    value = RuntimeValue::Null;
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
                    let result = self.scoped_statements(&module.items, module_environment);
                    self.module = previous_module;
                    self.current_references = previous_references;
                    self.depth -= 1;
                    self.loading.pop();
                    let (exports, _) = result?;
                    if !matches!(exports, RuntimeValue::Record(_)) {
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
                    name,
                    constant,
                    value: expression,
                    ty,
                } => {
                    let contract = ty.as_ref().map(|ty| self.annotation(ty)).transpose()?;
                    value = self.expr(expression, environment)?;
                    if contract
                        .as_ref()
                        .is_some_and(|ty| !ty.accepts_runtime(&value))
                    {
                        return self.error(
                            expression.span,
                            "RuntimeValue does not match its type annotation",
                        );
                    }
                    if *constant {
                        environment
                            .insert(name.text, value.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                    } else {
                        let cell = self.allocate_cell(value.clone(), contract, expression.span)?;
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
                    value = RuntimeValue::Function(Rc::new(Closure {
                        parameters: parameters.iter().map(|p| p.pattern.clone()).collect(),
                        parameter_types: parameters
                            .iter()
                            .map(|p| p.ty.as_ref().map(|ty| self.annotation(ty)).transpose())
                            .collect::<Result<Vec<_>>>()?,
                        result_type: result.as_ref().map(|ty| self.annotation(ty)).transpose()?,
                        name: Some(name.text),
                        module: self.module,
                        body: FunctionBody::Block(body.clone()),
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
                            None => RuntimeValue::Null,
                        },
                        Flow::Return,
                    ));
                }
                StmtKind::Break => return Ok((RuntimeValue::Null, Flow::Break)),
                StmtKind::Continue => return Ok((RuntimeValue::Null, Flow::Continue)),
                StmtKind::While { condition, body } => {
                    loop {
                        let RuntimeValue::Bool(keep_going) = self.expr(condition, environment)?
                        else {
                            return self.error(condition.span, "Condition must be boolean");
                        };
                        if !keep_going {
                            break;
                        }
                        let result = self.scoped_statements(
                            &body.stmts,
                            environment
                                .try_clone()
                                .map_err(|e| self.environment_error(statement.span, e))?,
                        )?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = RuntimeValue::Null;
                }
                StmtKind::For {
                    binding,
                    iterable,
                    body,
                } => {
                    let source = self.expr(iterable, environment)?;
                    let mut cursor = self.sequence_cursor(source, iterable.span)?;
                    while let Some(item) = self.sequence_next(&mut cursor, iterable.span)? {
                        let mut local = environment
                            .try_clone()
                            .map_err(|e| self.environment_error(statement.span, e))?;
                        local
                            .insert(binding.text, item)
                            .map_err(|e| self.environment_error(binding.span, e))?;
                        let result = self.scoped_statements(&body.stmts, local)?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = RuntimeValue::Null;
                }
                StmtKind::Expr(expression) => value = self.expr(expression, environment)?,
                StmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    let RuntimeValue::Bool(condition) = self.expr(condition, environment)? else {
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
                        value = RuntimeValue::Null;
                    }
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
    fn expr(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<RuntimeValue<'s>> {
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
    fn inner(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<RuntimeValue<'s>> {
        let span = expression.span;
        match &expression.kind {
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
                Ok(RuntimeValue::String(decoded))
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
                            RuntimeValue::String(key) => key,
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
                Ok(RuntimeValue::Record(fields))
            }
            ExprKind::Number(text) => (if text.contains('_') {
                Cow::Owned(text.replace('_', ""))
            } else {
                Cow::Borrowed(*text)
            })
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(RuntimeValue::Number)
            .ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Unsupported or non-finite number".into(),
            }),
            ExprKind::Bool(value) => Ok(RuntimeValue::Bool(*value)),
            ExprKind::Null => Ok(RuntimeValue::Null),
            ExprKind::Name(name) => match environment.get(name.text) {
                Some(Binding::RuntimeValue(value)) => Ok(value.clone()),
                Some(Binding::Cell(cell)) => Ok(self.cells[self.cell_index(cell, span)?].0.clone()),
                None => self.error(span, &format!("Unknown name: {}", name.text)),
            },
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                let ExprKind::Name(name) = &target.kind else {
                    return self.error(span, "Only variable assignment is supported");
                };
                let Some(Binding::Cell(cell)) = environment.get(name.text) else {
                    return self.error(span, "Assignment requires a mutable variable");
                };
                let index = self.cell_index(cell, target.span)?;
                let assigned = if *operator == "=" {
                    self.expr(value, environment)?
                } else {
                    let operator = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        _ => return self.error(span, "Unknown assignment operator"),
                    };
                    self.expr(
                        &Expr {
                            span,
                            kind: ExprKind::Binary {
                                operator,
                                left: target.clone(),
                                right: value.clone(),
                            },
                        },
                        environment,
                    )?
                };
                if self.cells[index]
                    .1
                    .as_ref()
                    .is_some_and(|ty| !ty.accepts_runtime(&assigned))
                {
                    return self.error(span, "Assignment violates variable type");
                }
                let previous = std::mem::replace(&mut self.cells[index].0, assigned.clone());
                if let Err(error) = self.enforce_retained_limit(span, None) {
                    self.cells[index].0 = previous;
                    return Err(error);
                }
                Ok(assigned)
            }
            ExprKind::Lambda { parameters, body } => Ok(RuntimeValue::Function(Rc::new(Closure {
                parameters: parameters.clone(),
                parameter_types: vec![None; parameters.len()],
                result_type: None,
                body: FunctionBody::Expression(*body.clone()),
                name: None,
                module: self.module,
                environment: self.capture(expression.span, environment)?,
                references: self.current_references.clone(),
            }))),
            ExprKind::Tuple(items) | ExprKind::List(items) => {
                self.collection_growth(0, items.len(), span)?;
                let values = items
                    .iter()
                    .map(|item| self.expr(item, environment))
                    .collect::<Result<Vec<_>>>()?;
                Ok(if matches!(expression.kind, ExprKind::Tuple(_)) {
                    RuntimeValue::Tuple(values)
                } else {
                    RuntimeValue::List(values)
                })
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                let RuntimeValue::Bool(condition) = self.expr(condition, environment)? else {
                    return self.error(span, "Condition must be boolean");
                };
                self.expr(if condition { then_value } else { else_value }, environment)
            }
            ExprKind::Member { object, field } => {
                let object = self.expr(object, environment)?;
                if let RuntimeValue::Mesh(mesh) = &object {
                    return match field.text {
                        "vertices" => {
                            self.collection_growth(0, mesh.vertices().len(), span)?;
                            self.charge(mesh.vertices().len(), span)?;
                            Ok(RuntimeValue::List(
                                mesh.vertices()
                                    .iter()
                                    .map(|v| RuntimeValue::Vector(v.to_vec()))
                                    .collect(),
                            ))
                        }
                        "triangles" => {
                            self.collection_growth(0, mesh.triangles().len(), span)?;
                            if !mesh.triangles().is_empty() {
                                self.collection_growth(0, 3, span)?;
                            }
                            self.charge(mesh.triangles().len(), span)?;
                            Ok(RuntimeValue::List(
                                mesh.triangles()
                                    .iter()
                                    .map(|t| {
                                        RuntimeValue::List(
                                            t.iter()
                                                .map(|i| RuntimeValue::Number(*i as f64))
                                                .collect(),
                                        )
                                    })
                                    .collect(),
                            ))
                        }
                        _ => self.error(field.span, "Unknown mesh field"),
                    };
                }
                if let RuntimeValue::Record(fields) = object {
                    return fields.get(field.text).cloned().ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span: field.span,
                        message: format!("Unknown field: {}", field.text),
                    });
                }
                if !matches!(object, RuntimeValue::Vector(_)) {
                    return self.error(field.span, "RuntimeValue does not support member access");
                }
                let axis = match field.text {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return self.error(field.span, "Unknown vector component"),
                };
                match object {
                    RuntimeValue::Vector(values) => values
                        .get(axis)
                        .copied()
                        .map(RuntimeValue::Number)
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
                if let RuntimeValue::Record(fields) = &object {
                    let RuntimeValue::String(key) = index else {
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
                let RuntimeValue::Number(index) = index else {
                    return self.error(span, "Index must be an integer");
                };
                if index < 0.0 || index.fract() != 0.0 || index >= usize::MAX as f64 {
                    return self.error(span, "Invalid collection index");
                }
                let result = match object {
                    RuntimeValue::List(values) | RuntimeValue::Tuple(values) => {
                        values.get(index as usize).cloned()
                    }
                    RuntimeValue::Vector(values) => values
                        .get(index as usize)
                        .copied()
                        .map(RuntimeValue::Number),
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
                                RuntimeValue::Bool(true) => {}
                                RuntimeValue::Bool(false) => continue,
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
                let function = self.expr(callee, environment)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expr(argument, environment))
                    .collect::<Result<Vec<_>>>()?;
                self.call(function, arguments.into(), span)
            }
            ExprKind::Unary { operator, value } => {
                match (*operator, self.expr(value, environment)?) {
                    ("-", RuntimeValue::Number(n)) => Ok(RuntimeValue::Number(-n)),
                    ("+", RuntimeValue::Number(n)) => Ok(RuntimeValue::Number(n)),
                    ("-", RuntimeValue::Vector(mut values)) => {
                        for value in &mut values {
                            *value = -*value;
                        }
                        self.vector(values, span)
                    }
                    ("+", RuntimeValue::Vector(values)) => self.vector(values, span),
                    ("!" | "not", RuntimeValue::Bool(b)) => Ok(RuntimeValue::Bool(!b)),
                    _ => self.error(span, "Invalid unary operand"),
                }
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let left = self.expr(left, environment)?;
                if matches!(*operator, "and" | "&&" | "or" | "||") {
                    let RuntimeValue::Bool(left) = left else {
                        return self.error(span, "Boolean operand required");
                    };
                    if left == matches!(*operator, "or" | "||") {
                        return Ok(RuntimeValue::Bool(left));
                    }
                    let right = self.expr(right, environment)?;
                    return if matches!(right, RuntimeValue::Bool(_)) {
                        Ok(right)
                    } else {
                        self.error(span, "Boolean operand required")
                    };
                }
                let right = self.expr(right, environment)?;
                if matches!(*operator, "==" | "!=") {
                    let equal = self.equal(&left, &right, span)?;
                    return Ok(RuntimeValue::Bool(if *operator == "==" {
                        equal
                    } else {
                        !equal
                    }));
                }
                if let (RuntimeValue::String(a), RuntimeValue::String(b), "+") =
                    (&left, &right, *operator)
                {
                    self.string_growth(a.len(), b.len(), span)?;
                    self.charge(a.len(), span)?;
                    self.charge(b.len(), span)?;
                    return Ok(RuntimeValue::String(format!("{a}{b}")));
                }
                if matches!(left, RuntimeValue::Angle(_)) || matches!(right, RuntimeValue::Angle(_))
                {
                    let radians = match (&left, &right, *operator) {
                        (RuntimeValue::Angle(a), RuntimeValue::Angle(b), "+") => a + b,
                        (RuntimeValue::Angle(a), RuntimeValue::Angle(b), "-") => a - b,
                        (RuntimeValue::Angle(a), RuntimeValue::Number(b), "*")
                        | (RuntimeValue::Number(b), RuntimeValue::Angle(a), "*") => a * b,
                        (RuntimeValue::Angle(a), RuntimeValue::Number(b), "/") => a / b,
                        _ => return self.error(span, "Invalid angle operands"),
                    };
                    if !radians.is_finite() {
                        return self.error(span, "Non-finite angle");
                    }
                    return Ok(RuntimeValue::Angle(radians));
                }
                if let (RuntimeValue::Quaternion(a), RuntimeValue::Quaternion(b), "*") =
                    (&left, &right, *operator)
                {
                    return Ok(RuntimeValue::Quaternion(Box::new(a.compose(b))));
                }
                if let (RuntimeValue::Matrix(a), RuntimeValue::Matrix(b), "*") =
                    (&left, &right, *operator)
                {
                    return a
                        .multiply(b)
                        .map(|matrix| RuntimeValue::Matrix(Box::new(matrix)))
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: "Matrix multiplication overflow".into(),
                        });
                }
                if matches!(left, RuntimeValue::Vector(_))
                    || matches!(right, RuntimeValue::Vector(_))
                {
                    let components = match (&left, &right, *operator) {
                        (RuntimeValue::Vector(a), RuntimeValue::Vector(b), "+" | "-")
                            if a.len() == b.len() =>
                        {
                            a.iter()
                                .zip(b)
                                .map(|(a, b)| if *operator == "+" { a + b } else { a - b })
                                .collect()
                        }
                        (RuntimeValue::Vector(a), RuntimeValue::Number(b), "*" | "/") => a
                            .iter()
                            .map(|a| if *operator == "*" { a * b } else { a / b })
                            .collect(),
                        (RuntimeValue::Number(a), RuntimeValue::Vector(b), "*") => {
                            b.iter().map(|b| a * b).collect()
                        }
                        _ => return self.error(span, "Invalid vector operands or dimensions"),
                    };
                    return self.vector(components, span);
                }
                let (RuntimeValue::Number(a), RuntimeValue::Number(b)) = (left, right) else {
                    return self.error(span, "Numeric operands required");
                };
                let number = match *operator {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    "%" => a % b,
                    "**" => a.powf(b),
                    "==" => return Ok(RuntimeValue::Bool(a == b)),
                    "!=" => return Ok(RuntimeValue::Bool(a != b)),
                    "<" => return Ok(RuntimeValue::Bool(a < b)),
                    ">" => return Ok(RuntimeValue::Bool(a > b)),
                    "<=" => return Ok(RuntimeValue::Bool(a <= b)),
                    ">=" => return Ok(RuntimeValue::Bool(a >= b)),
                    _ => return self.error(span, "Unsupported binary operator"),
                };
                if number.is_finite() {
                    Ok(RuntimeValue::Number(number))
                } else {
                    self.error(span, "Non-finite arithmetic result")
                }
            }
            _ => self.error(span, "Expression is not executable yet"),
        }
    }
    fn vector(&self, values: Vec<f64>, span: Span) -> Result<RuntimeValue<'s>> {
        if values.iter().all(|x| x.is_finite()) {
            Ok(RuntimeValue::Vector(values))
        } else {
            self.error(span, "Non-finite vector result")
        }
    }
    fn math(
        &self,
        builtin: Builtin,
        arguments: &[RuntimeValue<'s>],
        span: Span,
    ) -> Result<RuntimeValue<'s>> {
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
                    if let RuntimeValue::Number(n) = v {
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
            (
                Builtin::Slerp,
                [
                    RuntimeValue::Quaternion(a),
                    RuntimeValue::Quaternion(b),
                    RuntimeValue::Number(t),
                ],
            ) => {
                return a
                    .slerp(b, *t)
                    .map(|q| RuntimeValue::Quaternion(Box::new(q)))
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
                    RuntimeValue::Vector(axis),
                    RuntimeValue::Number(angle) | RuntimeValue::Angle(angle),
                ],
            ) if axis.len() == 3 => {
                return crate::Quaternion::axis_angle([axis[0], axis[1], axis[2]], *angle)
                    .map(|q| RuntimeValue::Quaternion(Box::new(q)))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Rotation requires a finite nonzero axis and angle".into(),
                    });
            }
            (Builtin::RotationMatrix, [RuntimeValue::Quaternion(q)]) => {
                return Ok(RuntimeValue::Matrix(Box::new(q.matrix())));
            }
            (Builtin::Identity, []) => {
                return Ok(RuntimeValue::Matrix(Box::new(crate::Matrix4::identity())));
            }
            (Builtin::Translation | Builtin::Scaling, [RuntimeValue::Vector(v)])
                if v.len() == 3 =>
            {
                let mut matrix = crate::Matrix4::identity();
                for (axis, value) in v.iter().enumerate() {
                    if builtin == Builtin::Translation {
                        matrix.0[axis][3] = *value;
                    } else {
                        matrix.0[axis][axis] = *value;
                    }
                }
                return Ok(RuntimeValue::Matrix(Box::new(matrix)));
            }
            (
                Builtin::RotationX | Builtin::RotationY | Builtin::RotationZ,
                [RuntimeValue::Number(angle) | RuntimeValue::Angle(angle)],
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
                return Ok(RuntimeValue::Matrix(Box::new(matrix)));
            }
            (
                Builtin::TransformPoint | Builtin::TransformDirection,
                [RuntimeValue::Matrix(matrix), RuntimeValue::Vector(v)],
            ) => {
                return matrix
                    .apply(v, builtin == Builtin::TransformPoint)
                    .map(RuntimeValue::Vector)
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
        if let (Builtin::Degrees | Builtin::Radians, [RuntimeValue::Number(value)]) =
            (builtin, arguments)
        {
            let radians = if builtin == Builtin::Degrees {
                value.to_radians()
            } else {
                *value
            };
            if !radians.is_finite() {
                return self.error(span, "Non-finite angle");
            }
            return Ok(RuntimeValue::Angle(radians));
        }
        let shape = match (builtin, arguments) {
            (Builtin::Polygon, [RuntimeValue::List(points)]) => {
                self.collection_growth(0, points.len(), span)?;
                let points = points
                    .iter()
                    .map(|p| match p {
                        RuntimeValue::Vector(v) if v.len() == 2 => Some([v[0], v[1]]),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>();
                Some(match points {
                    Some(points) => crate::Polygon::new(points),
                    None => Err("Polygon points must be vec2 values"),
                })
            }
            (
                Builtin::Translate,
                [RuntimeValue::Polygon(polygon), RuntimeValue::Vector(offset)],
            ) if offset.len() == 2 => {
                self.collection_growth(0, polygon.points().len(), span)?;
                Some(polygon.translated([offset[0], offset[1]]))
            }
            (
                Builtin::Rotate,
                [
                    RuntimeValue::Polygon(polygon),
                    RuntimeValue::Number(angle) | RuntimeValue::Angle(angle),
                ],
            ) => {
                self.collection_growth(0, polygon.points().len(), span)?;
                Some(polygon.rotated(*angle))
            }
            _ => None,
        };
        if let Some(shape) = shape {
            return shape
                .map(RuntimeValue::Polygon)
                .map_err(|message| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: message.into(),
                });
        }
        let number = match (builtin, arguments) {
            (Builtin::Random, [RuntimeValue::Number(seed), RuntimeValue::Number(index)]) => {
                crate::noise::random(*seed, *index).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "random requires nonnegative exact integer seed and index".into(),
                })?
            }
            (Builtin::Noise, [RuntimeValue::Number(x), RuntimeValue::Number(seed)]) => {
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

            (Builtin::Cross, [RuntimeValue::Vector(a), RuntimeValue::Vector(b)])
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
            (
                Builtin::Lerp,
                [
                    RuntimeValue::Vector(a),
                    RuntimeValue::Vector(b),
                    RuntimeValue::Number(t),
                ],
            ) if a.len() == b.len() => {
                return self.vector(
                    a.iter()
                        .zip(b)
                        .map(|(a, b)| a * (1.0 - t) + b * t)
                        .collect(),
                    span,
                );
            }
            (
                Builtin::Lerp,
                [
                    RuntimeValue::Number(a),
                    RuntimeValue::Number(b),
                    RuntimeValue::Number(t),
                ],
            ) => a * (1.0 - t) + b * t,
            (
                Builtin::Clamp,
                [
                    RuntimeValue::Number(x),
                    RuntimeValue::Number(low),
                    RuntimeValue::Number(high),
                ],
            ) if low <= high => x.clamp(*low, *high),
            (
                Builtin::Smoothstep,
                [
                    RuntimeValue::Number(low),
                    RuntimeValue::Number(high),
                    RuntimeValue::Number(x),
                ],
            ) if low < high => {
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

            (Builtin::Dot, [RuntimeValue::Vector(a), RuntimeValue::Vector(b)])
                if a.len() == b.len() =>
            {
                a.iter().zip(b).map(|(a, b)| a * b).sum()
            }
            (Builtin::Length | Builtin::Normalize, [RuntimeValue::Vector(v)]) => {
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
            (Builtin::Sin, [RuntimeValue::Number(x) | RuntimeValue::Angle(x)]) => x.sin(),
            (Builtin::Cos, [RuntimeValue::Number(x) | RuntimeValue::Angle(x)]) => x.cos(),
            (Builtin::Sqrt, [RuntimeValue::Number(x)]) => x.sqrt(),
            (Builtin::Deg, [RuntimeValue::Number(x)]) => x.to_radians(),
            _ => return self.error(span, "Invalid mathematical arguments"),
        };
        if number.is_finite() {
            Ok(RuntimeValue::Number(number))
        } else {
            self.error(span, "Non-finite mathematical result")
        }
    }
    fn call(
        &mut self,
        function: RuntimeValue<'s>,
        arguments: Cow<'_, [RuntimeValue<'s>]>,
        span: Span,
    ) -> Result<RuntimeValue<'s>> {
        self.check(span)?;
        if self.remaining == 0 {
            return self.error(span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        let name = match &function {
            RuntimeValue::Function(f) => f.name.unwrap_or("<lambda>").to_owned(),
            RuntimeValue::Host(f) => f.name.to_owned(),
            RuntimeValue::Builtin(b) => format!("{b:?}"),
            _ => "<non-callable>".to_owned(),
        };
        let module = self.module.map(str::to_owned);
        let source = self.sources.get(&self.module).copied().unwrap_or("");
        let prefix = &source[..span.start.min(source.len())];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        let result = match function {
            RuntimeValue::Function(function) => self.call_user(function, arguments, span),
            other => self.call_inner(other, arguments, span),
        };
        result.map_err(|mut error| {
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
                module,
                span,
                line,
                column,
            });
            error
        })
    }
    fn call_inner(
        &mut self,
        function: RuntimeValue<'s>,
        arguments: Cow<'_, [RuntimeValue<'s>]>,
        span: Span,
    ) -> Result<RuntimeValue<'s>> {
        if let RuntimeValue::Host(function) = function {
            if arguments.len() != function.parameters.len()
                || !function
                    .parameters
                    .iter()
                    .zip(arguments.iter())
                    .all(|(ty, value)| ty.accepts_runtime(value))
            {
                return self.error(span, "Host function arguments do not match its signature");
            }
            let arguments: Vec<Value> =
                arguments.into_owned().into_iter().map(Into::into).collect();
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
            let result: RuntimeValue = result.into();
            self.host_value_size(&result, span, 0)?;
            if !function.result.accepts_runtime(&result) {
                return self.error(span, "Host function returned an invalid value");
            }
            return Ok(result);
        }
        if let RuntimeValue::Builtin(builtin) = function {
            for &(index, expected) in builtin.callback_arities() {
                let valid = match arguments.get(index) {
                    Some(RuntimeValue::Function(function)) => {
                        Some(function.parameters.len() == expected)
                    }
                    Some(RuntimeValue::Host(function)) => {
                        Some(function.parameters.len() == expected)
                    }
                    Some(RuntimeValue::Builtin(function)) => {
                        Some(function.arity().contains(&expected))
                    }
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
                return Ok(RuntimeValue::Sequence(cursor.sequence));
            }
            if builtin == Builtin::Collect {
                let [source, RuntimeValue::Number(limit)] = arguments.as_ref() else {
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
                return Ok(RuntimeValue::List(output));
            }
            if matches!(builtin, Builtin::Map | Builtin::Filter)
                && matches!(
                    arguments.first(),
                    Some(RuntimeValue::Range { .. } | RuntimeValue::Sequence(_))
                )
            {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "map/filter require a sequence and callback");
                };
                if !matches!(
                    callback,
                    RuntimeValue::Function(_) | RuntimeValue::Builtin(_) | RuntimeValue::Host(_)
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
                return Ok(RuntimeValue::Sequence(Rc::new(sequence)));
            }
            if matches!(builtin, Builtin::Any | Builtin::All) {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "any/all require a list or sequence and predicate");
                };
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                if !matches!(
                    callback,
                    RuntimeValue::Function(_) | RuntimeValue::Builtin(_) | RuntimeValue::Host(_)
                ) {
                    return self.error(span, "Predicate must be callable");
                }
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let RuntimeValue::Bool(result) = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                    else {
                        return self.error(span, "Predicate must return a boolean");
                    };
                    if result == (builtin == Builtin::Any) {
                        return Ok(RuntimeValue::Bool(result));
                    }
                }
                return Ok(RuntimeValue::Bool(builtin == Builtin::All));
            }
            if builtin == Builtin::Get {
                if arguments.len() != 2 {
                    return self.error(span, "get requires a collection and key");
                }
                let value = match (&arguments[0], &arguments[1]) {
                    (RuntimeValue::Record(fields), RuntimeValue::String(key)) => {
                        fields.get(key).cloned()
                    }
                    (
                        RuntimeValue::List(items) | RuntimeValue::Tuple(items),
                        RuntimeValue::Number(index),
                    ) => {
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
                    Some(value) => RuntimeValue::Variant("Some", vec![value]),
                    None => RuntimeValue::Variant("None", vec![]),
                });
            }
            if builtin == Builtin::Len {
                if arguments.len() != 1 {
                    return self.error(span, "len requires one argument");
                }
                let count = match &arguments[0] {
                    RuntimeValue::List(items) | RuntimeValue::Tuple(items) => items.len(),
                    RuntimeValue::Record(fields) => fields.len(),
                    RuntimeValue::String(text) => {
                        self.charge(text.len(), span)?;
                        text.chars().count()
                    }
                    _ => return self.error(span, "len requires a list, tuple, record or string"),
                };
                return Ok(RuntimeValue::Number(count as f64));
            }
            if builtin == Builtin::Assert {
                if !builtin.arity().contains(&arguments.len()) {
                    return self.error(span, "Incorrect argument count");
                }
                let message = match arguments.get(1) {
                    None => "Assertion failed",
                    Some(RuntimeValue::String(message)) => message.as_str(),
                    _ => return self.error(span, "Assertion message must be a string"),
                };
                return match arguments[0] {
                    RuntimeValue::Bool(true) => Ok(RuntimeValue::Null),
                    RuntimeValue::Bool(false) => self.error(span, message),
                    _ => self.error(span, "Assertion condition must be a boolean"),
                };
            }
            if !builtin.arity().contains(&arguments.len()) {
                return self.error(span, "Incorrect builtin argument count");
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
                return Ok(RuntimeValue::Variant(tag, arguments.into_owned()));
            }

            if builtin == Builtin::Mesh {
                let [RuntimeValue::List(points), RuntimeValue::List(faces)] = arguments.as_ref()
                else {
                    return self.error(span, "mesh requires vertex and triangle lists");
                };
                self.collection_growth(0, points.len(), span)?;
                self.collection_growth(0, faces.len(), span)?;
                let mut vertices = Vec::new();
                for point in points {
                    self.charge(1, span)?;
                    let RuntimeValue::Vector(v) = point else {
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
                    let RuntimeValue::List(indices) = face else {
                        return self.error(span, "Mesh triangle must be a list of three indices");
                    };
                    if indices.len() != 3 {
                        return self.error(span, "Mesh triangle must have three indices");
                    }
                    let mut triangle = [0; 3];
                    for (slot, index) in triangle.iter_mut().zip(indices) {
                        let RuntimeValue::Number(index) = index else {
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
                    .map(|mesh| RuntimeValue::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Transform {
                let [RuntimeValue::Mesh(mesh), RuntimeValue::Matrix(matrix)] = arguments.as_ref()
                else {
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
                    .map(|mesh| RuntimeValue::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::GridMesh {
                let [RuntimeValue::List(xs), RuntimeValue::List(ys), callback] = arguments.as_ref()
                else {
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
                        let RuntimeValue::Vector(point) = point else {
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
                    .map(|mesh| RuntimeValue::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Zip {
                let [RuntimeValue::List(a), RuntimeValue::List(b)] = arguments.as_ref() else {
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
                    values.push(RuntimeValue::Tuple(vec![a.clone(), b.clone()]));
                }
                return Ok(RuntimeValue::List(values));
            }
            if matches!(builtin, Builtin::Range | Builtin::RangeIter) {
                let (start, end, step) = match arguments.as_ref() {
                    [RuntimeValue::Number(start), RuntimeValue::Number(end)] => (*start, *end, 1.0),
                    [
                        RuntimeValue::Number(start),
                        RuntimeValue::Number(end),
                        RuntimeValue::Number(step),
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
                    return Ok(RuntimeValue::Range { start, end, step });
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
                    values.push(RuntimeValue::Number(current));
                    let next = (values.len() as f64).mul_add(step, start);
                    if !next.is_finite() || next == current {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    current = next;
                }
                return Ok(RuntimeValue::List(values));
            }

            if matches!(builtin, Builtin::GroupBy | Builtin::FoldBy) {
                let mut arguments = arguments.into_owned().into_iter();
                let source = arguments.next().unwrap();
                let callback = arguments.next().unwrap();
                if !matches!(
                    callback,
                    RuntimeValue::Function(_) | RuntimeValue::Builtin(_) | RuntimeValue::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let initial = arguments.next().unwrap_or(RuntimeValue::Null);
                let reducer = arguments.next();
                if reducer.as_ref().is_some_and(|f| {
                    !matches!(
                        f,
                        RuntimeValue::Function(_)
                            | RuntimeValue::Builtin(_)
                            | RuntimeValue::Host(_)
                    )
                }) {
                    return self.error(span, "Reducer must be callable");
                }
                let field = if reducer.is_some() { "value" } else { "values" };
                let mut cursor = self.sequence_cursor(source, span)?;
                let mut positions = BTreeMap::<String, usize>::new();
                let mut groups = Vec::<(String, RuntimeValue<'s>)>::new();
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let key = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?;
                    let RuntimeValue::String(key) = key else {
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
                                RuntimeValue::List(Vec::new())
                            },
                        ));
                        position
                    };
                    if let Some(reducer) = &reducer {
                        let accumulator =
                            std::mem::replace(&mut groups[position].1, RuntimeValue::Null);
                        groups[position].1 =
                            self.call(reducer.clone(), Cow::Borrowed(&[accumulator, item]), span)?;
                    } else if let RuntimeValue::List(values) = &mut groups[position].1 {
                        self.collection_growth(values.len(), 1, span)?;
                        values.push(item);
                    }
                }
                let mut output = Vec::new();
                for (key, values) in groups {
                    self.charge(1, span)?;
                    output.push(RuntimeValue::Record(BTreeMap::from([
                        ("key".into(), RuntimeValue::String(key)),
                        (field.into(), values),
                    ])));
                }
                return Ok(RuntimeValue::List(output));
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
                RuntimeValue::Null
            };
            let callback = arguments.next().unwrap();
            if !matches!(
                callback,
                RuntimeValue::Function(_) | RuntimeValue::Builtin(_) | RuntimeValue::Host(_)
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
                accumulator = RuntimeValue::Null;
                match builtin {
                    Builtin::Map => {
                        self.collection_growth(output.len(), 1, span)?;
                        output.push(result);
                    }
                    Builtin::FlatMap => match result {
                        RuntimeValue::List(values) => {
                            self.charge(values.len(), span)?;
                            self.collection_growth(output.len(), values.len(), span)?;
                            output.extend(values);
                        }
                        _ => return self.error(span, "Flat map callback must return a list"),
                    },
                    Builtin::Filter => match result {
                        RuntimeValue::Bool(true) => {
                            self.collection_growth(output.len(), 1, span)?;
                            output.push(item);
                        }
                        RuntimeValue::Bool(false) => {}
                        _ => return self.error(span, "Filter callback must return a boolean"),
                    },
                    Builtin::Fold => accumulator = result,
                    _ => unreachable!(),
                }
            }
            return if builtin == Builtin::Fold {
                Ok(accumulator)
            } else {
                Ok(RuntimeValue::List(output))
            };
        }
        self.error(span, "RuntimeValue is not callable")
    }

    fn call_user(
        &mut self,
        function: Rc<Closure<'s>>,
        arguments: Cow<'_, [RuntimeValue<'s>]>,
        span: Span,
    ) -> Result<RuntimeValue<'s>> {
        if arguments.len() != function.parameters.len() {
            return self.error(span, "Incorrect argument count");
        }
        if function
            .parameter_types
            .iter()
            .zip(arguments.iter())
            .any(|(ty, value)| ty.as_ref().is_some_and(|ty| !ty.accepts_runtime(value)))
        {
            return self.error(span, "Argument does not match its type annotation");
        }
        let mut environment = Environment::child(function.environment.clone());
        if let Some(name) = function.name {
            environment
                .insert(name, RuntimeValue::Function(function.clone()))
                .map_err(|e| self.environment_error(span, e))?;
        }
        let previous_module = self.module;
        let previous_references =
            std::mem::replace(&mut self.current_references, function.references.clone());
        self.module = function.module;
        for (parameter, argument) in function.parameters.iter().zip(arguments.iter()) {
            if let Err(error) = self.bind_pattern(parameter, argument, &mut environment) {
                self.module = previous_module;
                self.current_references = previous_references;
                return Err(error);
            }
        }
        let result = match &function.body {
            FunctionBody::Expression(body) => self.expr(body, &environment),
            FunctionBody::Block(body) => {
                if self.depth >= self.max_depth {
                    self.module = previous_module;
                    self.current_references = previous_references;
                    return self.error(span, "Execution limit exceeded");
                }
                self.depth += 1;
                let result =
                    self.statements(&body.stmts, &mut environment)
                        .map(|(value, returned)| {
                            if returned == Flow::Return {
                                value
                            } else {
                                RuntimeValue::Null
                            }
                        });
                self.depth -= 1;
                result
            }
        };
        self.module = previous_module;
        self.current_references = previous_references;
        drop(environment);
        self.reclaim_cells();
        let value = result?;
        if function
            .result_type
            .as_ref()
            .is_some_and(|ty| !ty.accepts_runtime(&value))
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
