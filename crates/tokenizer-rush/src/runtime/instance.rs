//! Script ownership and portable, explicitly selected save data.
use super::*;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum StateValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    List(Vec<StateValue>),
    Record(BTreeMap<String, StateValue>),
}
pub type ScriptState = BTreeMap<String, StateValue>;
impl StateValue {
    pub fn from_value(value: &Value<'_>) -> std::result::Result<Self, String> {
        Self::convert(value, 0)
    }
    fn convert(value: &Value<'_>, depth: usize) -> std::result::Result<Self, String> {
        if depth > 64 {
            return Err("State nesting limit exceeded".into());
        }
        Ok(match value {
            Value::Null => Self::Null,
            Value::Bool(v) => Self::Bool(*v),
            Value::Number(v) if v.is_finite() => Self::Number(*v),
            Value::String(v) => Self::String(v.clone()),
            Value::List(values) => Self::List(values.iter().map(|v| Self::convert(v, depth + 1)).collect::<std::result::Result<_, _>>()?),
            Value::Record(fields) => Self::Record(fields.iter().map(|(k,v)| Ok((k.clone(), Self::convert(v, depth + 1)?))).collect::<std::result::Result<_, String>>()?),
            _ => return Err("State supports only finite numbers, strings, booleans, null, lists and records; restore host objects through engine IDs".into()),
        })
    }
    pub fn to_value(&self) -> std::result::Result<Value<'static>, String> {
        fn convert(v: &StateValue, depth: usize) -> std::result::Result<Value<'static>, String> {
            if depth > 64 {
                return Err("State nesting limit exceeded".into());
            }
            Ok(match v {
                StateValue::Null => Value::Null,
                StateValue::Bool(v) => Value::Bool(*v),
                StateValue::Number(v) if v.is_finite() => Value::Number(*v),
                StateValue::Number(_) => return Err("Non-finite state number".into()),
                StateValue::String(v) => Value::String(v.clone()),
                StateValue::List(v) => Value::List(
                    v.iter()
                        .map(|v| convert(v, depth + 1))
                        .collect::<std::result::Result<_, _>>()?,
                ),
                StateValue::Record(v) => Value::Record(
                    v.iter()
                        .map(|(k, v)| Ok((k.clone(), convert(v, depth + 1)?)))
                        .collect::<std::result::Result<_, String>>()?,
                ),
            })
        }
        convert(self, 0)
    }
}
impl ScriptInstance<'_, '_> {
    pub fn export_state(&self, names: &[&str]) -> Result<ScriptState> {
        let mut state = ScriptState::new();
        for name in names {
            let value = self
                .get(name)
                .ok_or_else(|| self.state_error(format!("Unknown state variable: {name}")))?;
            state.insert(
                (*name).into(),
                StateValue::from_value(&value).map_err(|e| self.state_error(e))?,
            );
        }
        Ok(state)
    }
    fn state_error(&self, message: String) -> RuntimeError {
        RuntimeError {
            module: None,
            span: self.span,
            message,
            stack: Vec::new(),
            location: None,
        }
    }
    /// Validate the entire save before updating any mutable binding.
    pub fn restore_state(&mut self, state: &ScriptState) -> Result<()> {
        let mut pending = Vec::new();
        for (name, value) in state {
            let Some(Binding::Cell(cell)) = self.environment.get(name) else {
                return Err(self.state_error(format!(
                    "State target must be an existing mutable variable: {name}"
                )));
            };
            let index = self.runtime.cell_index(cell, self.span)?;
            let value = value.to_value().map_err(|e| self.state_error(e))?;
            if self.runtime.cells[index]
                .1
                .as_ref()
                .is_some_and(|ty| !ty.accepts(&value))
            {
                return Err(self.state_error(format!("State violates variable type: {name}")));
            }
            let remaining = self.runtime.remaining;
            self.runtime.remaining = usize::MAX;
            let checked = self.runtime.host_value_size(&value, self.span, 0);
            self.runtime.remaining = remaining;
            checked?;
            pending.push((index, value));
        }
        let previous: Vec<_> = pending
            .into_iter()
            .map(|(index, value)| {
                (
                    index,
                    std::mem::replace(&mut self.runtime.cells[index].0, value),
                )
            })
            .collect();
        if let Err(error) = self.enforce_memory_limit() {
            for (index, value) in previous {
                self.runtime.cells[index].0 = value;
            }
            return Err(error);
        }
        Ok(())
    }
}

struct OwnedSource {
    source: String,
    cancellation: CancellationToken,
    inputs: Vec<(String, Value<'static>)>,
    functions: Vec<Rc<HostFunction>>,
    contextual: Vec<HostRegistration>,
    modules: Vec<(String, String)>,
}
type OwnedDependent<'a> = ScriptInstance<'a, 'a>;
self_cell::self_cell! {
    struct ScriptCell {
        owner: OwnedSource,
        #[not_covariant]
        dependent: OwnedDependent,
    }
}
/// Movable instance which owns all script/module text and its cancellation token.
/// Borrowed callable values may be used inside `with_instance`, but cannot escape.
/// ```compile_fail
/// use themoretheless_tokenizer_rush::{OwnedScriptInstance, ExecutionLimits};
/// let limits = ExecutionLimits::new(10_000);
/// let mut script = OwnedScriptInstance::new("fn make() { return () => 1 }", limits).unwrap();
/// let escaped = script.with_instance(|instance| instance.call("make", &[], limits).unwrap());
/// drop(script);
/// println!("{escaped:?}");
/// ```
pub struct OwnedScriptInstance {
    cell: ScriptCell,
}
impl OwnedScriptInstance {
    pub fn new(source: impl Into<String>, limits: ExecutionLimits) -> Result<Self> {
        Self::with_host(
            source,
            limits,
            CancellationToken::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn with_host(
        source: impl Into<String>,
        limits: ExecutionLimits,
        cancellation: CancellationToken,
        inputs: Vec<(String, Value<'static>)>,
        functions: Vec<Rc<HostFunction>>,
        contextual: Vec<HostRegistration>,
        modules: Vec<(String, String)>,
    ) -> Result<Self> {
        let owner = OwnedSource {
            source: source.into(),
            cancellation,
            inputs,
            functions,
            contextual,
            modules,
        };
        let cell = ScriptCell::try_new(owner, |owner| {
            let program = Program::compile(&owner.source)?;
            let modules: Vec<_> = owner
                .modules
                .iter()
                .map(|(name, source)| Ok((name.as_str(), Program::compile(source)?)))
                .collect::<Result<_>>()?;
            let modules: Vec<_> = modules.iter().map(|(name, p)| (*name, p)).collect();
            let inputs: Vec<_> = owner
                .inputs
                .iter()
                .map(|(name, value)| (name.as_str(), value.clone()))
                .collect();
            program.instantiate(
                limits,
                &owner.cancellation,
                &inputs,
                &owner.functions,
                &owner.contextual,
                &modules,
            )
        })?;
        Ok(Self { cell })
    }
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cell.borrow_owner().cancellation.clone()
    }
    pub fn source(&self) -> &str {
        &self.cell.borrow_owner().source
    }
    pub fn with_instance<R>(
        &mut self,
        f: impl for<'a> FnOnce(&mut ScriptInstance<'a, 'a>) -> R,
    ) -> R {
        self.cell.with_dependent_mut(|_, instance| {
            let result = f(instance);
            instance.runtime.reclaim_cells();
            result
        })
    }
    /// Data-only result with no lifetime; use `with_instance` for callable/host values.
    pub fn call(
        &mut self,
        name: &str,
        arguments: &[StateValue],
        limits: ExecutionLimits,
    ) -> Result<StateValue> {
        self.with_instance(|instance| {
            let arguments = arguments
                .iter()
                .map(StateValue::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| instance.state_error(e))?;
            let value = instance.call(name, &arguments, limits)?;
            StateValue::from_value(&value).map_err(|e| instance.state_error(e))
        })
    }
    pub fn export_state(&mut self, names: &[&str]) -> Result<ScriptState> {
        self.with_instance(|i| i.export_state(names))
    }
    pub fn restore_state(&mut self, state: &ScriptState) -> Result<()> {
        self.with_instance(|i| i.restore_state(state))
    }
    pub fn set_memory_limit(&mut self, bytes: usize) -> Result<()> {
        self.with_instance(|i| i.set_memory_limit(bytes))
    }
    pub fn memory_usage(&mut self) -> usize {
        self.with_instance(|i| i.memory_usage())
    }
}

/// Logical retained-data accounting. Rc payloads are counted once, cycles terminate.
/// This is not allocator/RSS accounting; host-owned allocations are opaque.
#[derive(Default)]
struct Usage {
    bytes: usize,
    seen: std::collections::HashSet<(u8, usize)>,
}
impl Usage {
    fn add(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_add(bytes);
    }
    fn environment(&mut self, environment: &Environment<'_>) {
        self.add(
            environment
                .bindings
                .capacity()
                .saturating_mul(std::mem::size_of::<(&str, Binding<'_>)>()),
        );
        for (_, binding) in environment.bindings.iter() {
            if let Binding::Value(value) = binding {
                self.value(value);
            }
        }
        if let Some(parent) = &environment.parent
            && self
                .seen
                .insert((1, memory::Shared::as_ptr(parent) as usize))
        {
            self.environment(parent);
        }
    }
    fn value(&mut self, value: &Value<'_>) {
        match value {
            Value::String(text) => self.add(text.capacity()),
            Value::Vector(v) => self.add(v.capacity().saturating_mul(8)),
            Value::List(v) | Value::Tuple(v) | Value::Variant(_, v) => {
                self.add(
                    v.capacity()
                        .saturating_mul(std::mem::size_of::<Value<'_>>()),
                );
                for item in v {
                    self.value(item);
                }
            }
            Value::Record(v) => {
                self.add(v.len().saturating_mul(
                    std::mem::size_of::<(String, Value<'_>)>() + 3 * std::mem::size_of::<usize>(),
                ));
                for (key, value) in v {
                    self.add(key.capacity());
                    self.value(value);
                }
            }
            Value::Function(f) if self.seen.insert((2, Rc::as_ptr(f) as usize)) => {
                self.add(std::mem::size_of::<Closure<'_>>());
                if self
                    .seen
                    .insert((1, memory::Shared::as_ptr(&f.environment) as usize))
                {
                    self.environment(&f.environment);
                }
            }
            Value::Sequence(sequence) if self.seen.insert((3, Rc::as_ptr(sequence) as usize)) => {
                self.add(
                    std::mem::size_of::<Sequence<'_>>()
                        + sequence.stages.capacity() * std::mem::size_of::<SequenceStage<'_>>(),
                );
                if let SequenceSource::List(list) = &sequence.source
                    && self.seen.insert((4, Rc::as_ptr(list) as usize))
                {
                    self.add(list.capacity() * std::mem::size_of::<Value<'_>>());
                    for v in list.iter() {
                        self.value(v);
                    }
                }
                for stage in &sequence.stages {
                    self.value(&stage.callback);
                }
            }
            Value::Mesh(mesh) if self.seen.insert((5, Rc::as_ptr(mesh) as usize)) => {
                self.add(
                    std::mem::size_of_val(mesh.vertices())
                        + std::mem::size_of_val(mesh.triangles()),
                );
            }
            Value::Polygon(p) => self.add(std::mem::size_of_val(p.points())),
            Value::Matrix(_) => self.add(std::mem::size_of::<crate::Matrix4>()),
            Value::Quaternion(_) => self.add(std::mem::size_of::<crate::Quaternion>()),
            _ => {}
        }
    }
}
impl Runtime<'_, '_> {
    fn retained_usage(&self) -> Usage {
        let mut usage = Usage::default();
        usage.add(
            self.cells
                .len()
                .saturating_mul(std::mem::size_of::<(Value<'_>, Option<ValueType>)>()),
        );
        for (value, _) in self.cells.iter() {
            usage.value(value);
        }
        usage.environment(&self.module_globals);
        if let Some(environment) = &self.instance_roots {
            usage.environment(environment);
        }
        for (_, value) in self.module_cache.iter() {
            usage.value(value);
        }
        usage
    }
    pub(super) fn enforce_retained_limit(
        &self,
        span: Span,
        additional: Option<&Value<'_>>,
    ) -> Result<()> {
        if self.memory_limit == usize::MAX {
            return Ok(());
        }
        let mut usage = self.retained_usage();
        if let Some(value) = additional {
            usage.value(value);
        }
        if usage.bytes > self.memory_limit {
            self.error(span, "Instance retained-data limit exceeded")
        } else {
            Ok(())
        }
    }
}
impl ScriptInstance<'_, '_> {
    /// Budget for retained script data across calls. Not a hard heap/RSS ceiling:
    /// allocations in host contexts, ASTs and transient evaluation are excluded.
    pub fn set_memory_limit(&mut self, bytes: usize) -> Result<()> {
        let previous = self.runtime.memory_limit;
        self.runtime.memory_limit = bytes;
        if let Err(error) = self.enforce_memory_limit() {
            self.runtime.memory_limit = previous;
            return Err(error);
        }
        Ok(())
    }
    pub fn memory_usage(&self) -> usize {
        self.runtime.retained_usage().bytes
    }
    pub(super) fn enforce_memory_limit(&self) -> Result<()> {
        if self.memory_usage() > self.runtime.memory_limit {
            self.runtime
                .error(self.span, "Instance retained-data limit exceeded")
        } else {
            Ok(())
        }
    }
}

impl ScriptInstance<'_, '_> {
    pub(super) fn initialize_usage(&mut self) {
        let mut usage = Usage::default();
        usage.value(&self.initial_value);
        self.runtime.initial_data_bytes = usage.bytes;
    }
}

fn owned_value(value: Value<'_>) -> std::result::Result<Value<'static>, String> {
    Ok(match value {
        Value::HostObject(v) => Value::HostObject(v),
        Value::Angle(v) => Value::Angle(v),
        Value::Mesh(v) => Value::Mesh(v),
        Value::Quaternion(v) => Value::Quaternion(v),
        Value::String(v) => Value::String(v),
        Value::Record(v) => Value::Record(
            v.into_iter()
                .map(|(k, v)| Ok((k, owned_value(v)?)))
                .collect::<std::result::Result<_, String>>()?,
        ),
        Value::Matrix(v) => Value::Matrix(v),
        Value::Polygon(v) => Value::Polygon(v),
        Value::Number(v) => Value::Number(v),
        Value::Vector(v) => Value::Vector(v),
        Value::Bool(v) => Value::Bool(v),
        Value::Null => Value::Null,
        Value::Range { start, end, step } => Value::Range { start, end, step },
        Value::List(v) => Value::List(
            v.into_iter()
                .map(owned_value)
                .collect::<std::result::Result<_, _>>()?,
        ),
        Value::Tuple(v) => Value::Tuple(
            v.into_iter()
                .map(owned_value)
                .collect::<std::result::Result<_, _>>()?,
        ),
        Value::Variant(name, v) => Value::Variant(
            name,
            v.into_iter()
                .map(owned_value)
                .collect::<std::result::Result<_, _>>()?,
        ),
        Value::Host(v) => Value::Host(v),
        Value::Builtin(v) => Value::Builtin(v),
        Value::Function(_) | Value::Sequence(_) => {
            return Err("Borrowed callable or lazy sequence result requires with_instance".into());
        }
    })
}
impl OwnedScriptInstance {
    /// Arbitrary owned value arguments, including host object handles and event records.
    pub fn call_values(
        &mut self,
        name: &str,
        arguments: &[Value<'static>],
        limits: ExecutionLimits,
    ) -> Result<Value<'static>> {
        self.with_instance(|instance| {
            owned_value(instance.call(name, arguments, limits)?)
                .map_err(|e| instance.state_error(e))
        })
    }
}

impl Runtime<'_, '_> {
    pub(super) fn enforce_cell_limit(&self, value: &Value<'_>, span: Span) -> Result<()> {
        if self.memory_limit == usize::MAX {
            return Ok(());
        }
        let mut usage = self.retained_usage();
        usage.value(value);
        if self.free_cells.is_empty() {
            usage.add(std::mem::size_of::<(Value<'_>, Option<ValueType>)>());
        }
        if usage.bytes > self.memory_limit {
            self.error(span, "Instance retained-data limit exceeded")
        } else {
            Ok(())
        }
    }
}
