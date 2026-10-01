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
mod ast_memory;
mod engine_value;
mod memory;
use engine_value::EngineValue;
mod vm;

#[derive(Clone, Debug, PartialEq)]
enum Binding<'s> {
    Value(EngineValue<'s>),
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
    _allocation: memory::Shared<memory::Reservation>,
    _release_allocation: memory::Shared<memory::Reservation>,
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
        value: EngineValue<'s>,
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
    item_type: Rc<ValueType>,
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
            source: SequenceSource::Host(HostSource {
                factory,
                item_type: Rc::new(item_type),
            }),
            stages: SequenceStages::default(),
            _allocation: None,
        }))
    }
}

/// Repeatable sequence with deferred transformations. Construct through Rush builtins.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence<'s> {
    _allocation: Option<memory::Shared<memory::Reservation>>,
    source: SequenceSource<'s>,
    stages: SequenceStages<'s>,
}

#[derive(Clone, Debug, PartialEq)]
enum SequenceSource<'s> {
    Range { start: f64, end: f64, step: f64 },
    List(memory::Buffer<EngineValue<'s>>),
    Host(HostSource),
}

#[derive(Clone, Debug, PartialEq)]
struct SequenceStage<'s> {
    callback: EngineValue<'s>,
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
    parameter_types: memory::Buffer<Option<memory::Shared<ast_memory::RuntimeType>>>,
    result_type: Option<memory::Shared<ast_memory::RuntimeType>>,
    body: FunctionBody<'s>,
    name: Option<&'s str>,
    environment: memory::Shared<Environment<'s>>,
    references: memory::Buffer<CaptureReference<'s>>,
    _allocation: memory::Shared<memory::Reservation>,
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
            let mut types = Vec::with_capacity(ty.arguments.len());
            for ty in &ty.arguments {
                types.push(Self::annotation(ty)?);
            }
            return Ok(Self::Tuple(types));
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
    _initial_storage: memory::Reservation,
}
impl<'a, 's> ScriptInstance<'a, 's> {
    pub fn initial_value(&self) -> &Value<'s> {
        &self.initial_value
    }
    pub fn get(&self, name: &str) -> Option<Value<'s>> {
        match self.environment.get(name)? {
            Binding::Value(value) => Some(value.export()),
            Binding::Cell(cell) => Some(self.runtime.cells[cell.index].0.export()),
        }
    }
    /// Invoke a named callable with a fresh execution budget and the instance's token.
    pub fn call(
        &mut self,
        name: &str,
        arguments: &[Value<'s>],
        limits: ExecutionLimits,
    ) -> Result<Value<'s>> {
        let function = self.prepare_call(name, limits)?;
        let mut imported = memory::Slots::new(&self.runtime.memory, arguments.len())
            .map_err(|e| self.runtime.environment_error(self.span, e))?;
        for argument in arguments {
            imported
                .push(
                    EngineValue::import(argument, &self.runtime.memory)
                        .map_err(|e| self.runtime.environment_error(self.span, e))?,
                )
                .map_err(|e| self.runtime.environment_error(self.span, e))?;
        }
        let value = self
            .runtime
            .call(function, Cow::Borrowed(&imported), self.span)?;
        Ok(value.export())
    }
    fn prepare_call(&mut self, name: &str, limits: ExecutionLimits) -> Result<EngineValue<'s>> {
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
        let function = match self.environment.get(name) {
            Some(Binding::Value(value)) => value.clone(),
            Some(Binding::Cell(cell)) => self.runtime.cells[cell.index].0.clone(),
            None => {
                return self
                    .runtime
                    .error(self.span, &format!("Unknown function: {name}"));
            }
        };
        Ok(function)
    }
    fn get_engine(&self, name: &str) -> Option<&EngineValue<'s>> {
        match self.environment.get(name)? {
            Binding::Value(v) => Some(v),
            Binding::Cell(cell) => Some(&self.runtime.cells[cell.index].0),
        }
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
        self.instantiate_with_memory_limit(
            limits,
            cancellation,
            inputs,
            functions,
            contextual,
            modules,
            usize::MAX,
        )
    }
    /// Apply the aggregate allocation limit before top-level initialization.
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate_with_memory_limit<'a>(
        &self,
        limits: ExecutionLimits,
        cancellation: &'a CancellationToken,
        inputs: &[(&'s str, Value<'s>)],
        functions: &[Rc<HostFunction>],
        contextual: &[HostRegistration],
        modules: &[(&'s str, &Program<'s>)],
        bytes: usize,
    ) -> Result<ScriptInstance<'a, 's>> {
        let memory = memory::Budget::new(bytes);
        let mut input_refs = memory::Slots::new(&memory, inputs.len())
            .map_err(|e| memory_error(self.parsed.module.span, e))?;
        for (name, value) in inputs {
            input_refs
                .push((*name, value))
                .map_err(|e| memory_error(self.parsed.module.span, e))?;
        }
        self.instantiate_on_budget(
            limits,
            cancellation,
            &input_refs,
            functions,
            contextual,
            modules,
            memory,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn instantiate_on_budget<'a>(
        &self,
        limits: ExecutionLimits,
        cancellation: &'a CancellationToken,
        inputs: &[(&'s str, &Value<'s>)],
        functions: &[Rc<HostFunction>],
        contextual: &[HostRegistration],
        modules: &[(&'s str, &Program<'s>)],
        memory: memory::Budget,
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
        let mut runtime = Runtime {
            module: None,
            modules: memory::Table::new(&memory),
            module_cache: memory::Slots::new(&memory, 0)
                .expect("empty module cache requires no allocation"),
            loading: memory::Slots::new(&memory, 0)
                .expect("empty module stack requires no allocation"),
            module_globals: Environment::new(&memory),
            memory: memory.clone(),
            references: memory::Table::new(&memory),
            current_references: memory::Buffer::from_iter(&memory, self.references.iter().cloned())
                .map_err(|e| memory_error(self.parsed.module.span, e))?,
            cells: memory::Slots::new(&memory, 0)
                .expect("empty cell storage requires no allocation"),
            released_cells: Rc::new(RefCell::new(
                memory::Slots::new(&memory, 0).expect("empty release queue requires no allocation"),
            )),
            free_cells: memory::Slots::new(&memory, 0)
                .expect("empty free cell storage requires no allocation"),
            cell_ids: memory::Slots::new(&memory, 0)
                .expect("empty cell ID storage requires no allocation"),
            cell_id_storage: memory::Slots::new(&memory, 0).expect("empty leases"),
            cell_allocations: 0,
            cell_collection_interval: 64,
            remaining: limits.steps,
            max_depth: limits.max_depth,
            max_collection_items: limits.max_collection_items,
            max_string_bytes: limits.max_string_bytes,
            depth: 0,
            sources: memory::Table::new(&memory),
            _release_allocation: memory::Shared::new(
                &memory,
                memory
                    .reservation(memory::rc_bytes::<RefCell<memory::Slots<usize>>>())
                    .map_err(|e| memory_error(self.parsed.module.span, e))?,
            )
            .map_err(|e| memory_error(self.parsed.module.span, e))?,
            instance_roots: None,
            cancellation,
            contextual: memory::Table::new(&memory),
        };
        runtime
            .sources
            .insert(None, self.parsed.source)
            .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        runtime
            .references
            .insert(None, runtime.current_references.clone())
            .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        for entry in contextual {
            runtime
                .contextual
                .insert(Rc::as_ptr(&entry.function), entry.callback.clone())
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        runtime.check(self.parsed.module.span)?;
        let mut environment = Environment::new(&memory);
        for &(name, builtin) in builtin_catalog() {
            environment
                .insert(name, EngineValue::Builtin(builtin))
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
                .insert(function.name, EngineValue::Host(function.clone()))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        for (name, value) in inputs {
            let value = EngineValue::import(value, &runtime.memory)
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
            runtime.host_value_size(&value, self.parsed.module.span, 0)?;
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
            runtime
                .sources
                .insert(Some(*name), program.parsed.source)
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
            let storage = memory
                .reservation(ast_memory::statements(&program.parsed.module.items))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
            let module = memory::Shared::new(
                &memory,
                ast_memory::StoredModule {
                    module: program.parsed.module.clone(),
                    _storage: storage,
                },
            )
            .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
            if runtime
                .modules
                .insert(*name, module)
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?
                .is_some()
            {
                return runtime.error(self.parsed.module.span, "Duplicate module registration");
            }
            let references = memory::Buffer::from_iter(&memory, program.references.iter().cloned())
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
            runtime
                .references
                .insert(Some(*name), references)
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        let (initial_value, _) = runtime.statements(&self.parsed.module.items, &mut environment)?;
        runtime.instance_roots = Some(
            environment
                .try_clone()
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?,
        );
        let snapshot_bytes = initial_value
            .export_bytes(memory.available_bytes())
            .ok_or_else(|| {
                memory_error(self.parsed.module.span, memory::AllocationError::Capacity)
            })?;
        let snapshot = memory
            .reservation(snapshot_bytes)
            .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        let instance = ScriptInstance {
            runtime,
            environment,
            span: self.parsed.module.span,
            initial_value: initial_value.export(),
            _initial_storage: snapshot,
        };
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
    modules: memory::Table<&'s str, memory::Shared<ast_memory::StoredModule<'s>>>,
    module_cache: memory::Slots<(&'s str, EngineValue<'s>)>,
    loading: memory::Slots<&'s str>,
    module_globals: Environment<'s>,
    memory: memory::Budget,
    references: memory::Table<Option<&'s str>, memory::Buffer<CaptureReference<'s>>>,
    current_references: memory::Buffer<CaptureReference<'s>>,
    cells: memory::Slots<(
        EngineValue<'s>,
        Option<memory::Shared<ast_memory::RuntimeType>>,
    )>,
    released_cells: Rc<RefCell<memory::Slots<usize>>>,
    free_cells: memory::Slots<usize>,
    cell_ids: memory::Slots<Weak<CellId>>,
    cell_id_storage: memory::Slots<Option<memory::Shared<memory::Reservation>>>,
    cell_allocations: usize,
    cell_collection_interval: usize,
    remaining: usize,
    depth: usize,
    max_depth: usize,
    max_collection_items: usize,
    max_string_bytes: usize,
    sources: memory::Table<Option<&'s str>, &'s str>,
    _release_allocation: memory::Shared<memory::Reservation>,
    instance_roots: Option<Environment<'s>>,
    cancellation: &'a CancellationToken,
    contextual: memory::Table<*const HostFunction, ContextualHostCallback>,
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

fn memory_error(span: Span, error: memory::AllocationError) -> RuntimeError {
    RuntimeError {
        stack: Vec::new(),
        location: None,
        module: None,
        span,
        message: format!("Instance memory allocation failed: {error:?}"),
    }
}
