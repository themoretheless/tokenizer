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
                .get_engine(name)
                .ok_or_else(|| self.state_error(format!("Unknown state variable: {name}")))?;
            state.insert(
                (*name).into(),
                value.to_state().map_err(|e| self.state_error(e))?,
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
        let mut pending = memory::Slots::new(&self.runtime.memory, state.len())
            .map_err(|e| self.runtime.environment_error(self.span, e))?;
        for (name, value) in state {
            let Some(Binding::Cell(cell)) = self.environment.get(name) else {
                return Err(self.state_error(format!(
                    "State target must be an existing mutable variable: {name}"
                )));
            };
            let index = self.runtime.cell_index(cell, self.span)?;
            let value = EngineValue::import_state(value, &self.runtime.memory, 0)
                .map_err(|e| self.runtime.environment_error(self.span, e))?;
            if self.runtime.cells[index]
                .1
                .as_ref()
                .is_some_and(|ty| !ty.accepts_engine(&value))
            {
                return Err(self.state_error(format!("State violates variable type: {name}")));
            }
            let remaining = self.runtime.remaining;
            self.runtime.remaining = usize::MAX;
            let checked = self.runtime.host_value_size(&value, self.span, 0);
            self.runtime.remaining = remaining;
            checked?;
            pending
                .push((index, value))
                .map_err(|e| self.runtime.environment_error(self.span, e))?;
        }
        // Every replacement has already been allocated and validated. Commit cannot allocate.
        for (index, value) in pending.iter_mut() {
            self.runtime.cells[*index].0 = std::mem::replace(value, EngineValue::Null);
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
    memory: memory::Budget,
    _storage: memory::Reservation,
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
        Self::new_with_memory_limit(source, limits, usize::MAX)
    }
    pub fn new_with_memory_limit(
        source: impl Into<String>,
        limits: ExecutionLimits,
        bytes: usize,
    ) -> Result<Self> {
        Self::with_host_and_memory_limit(
            source,
            limits,
            CancellationToken::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            bytes,
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
        Self::with_host_and_memory_limit(
            source,
            limits,
            cancellation,
            inputs,
            functions,
            contextual,
            modules,
            usize::MAX,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn with_host_and_memory_limit(
        source: impl Into<String>,
        limits: ExecutionLimits,
        cancellation: CancellationToken,
        inputs: Vec<(String, Value<'static>)>,
        functions: Vec<Rc<HostFunction>>,
        contextual: Vec<HostRegistration>,
        modules: Vec<(String, String)>,
        bytes: usize,
    ) -> Result<Self> {
        let source = source.into();
        let memory = memory::Budget::new(bytes);
        let storage_bytes = (std::mem::size_of::<OwnedSource>()
            + std::mem::size_of::<OwnedDependent<'_>>()
            + 64)
            .checked_add(source.capacity())
            .and_then(|n| {
                n.checked_add(inputs.capacity() * std::mem::size_of::<(String, Value<'static>)>())
            })
            .and_then(|n| {
                n.checked_add(functions.capacity() * std::mem::size_of::<Rc<HostFunction>>())
            })
            .and_then(|n| {
                n.checked_add(contextual.capacity() * std::mem::size_of::<HostRegistration>())
            })
            .and_then(|n| {
                n.checked_add(modules.capacity() * std::mem::size_of::<(String, String)>())
            })
            .and_then(|mut n| {
                for (k, v) in &inputs {
                    n = n
                        .checked_add(k.capacity())?
                        .checked_add(public_storage(v, 0)?)?;
                }
                for (name, source) in &modules {
                    n = n
                        .checked_add(name.capacity())?
                        .checked_add(source.capacity())?;
                }
                Some(n)
            })
            .ok_or_else(|| {
                memory_error(
                    Span::new(0, source.len()),
                    memory::AllocationError::Capacity,
                )
            })?;
        let storage = memory
            .reservation(storage_bytes)
            .map_err(|e| memory_error(Span::new(0, source.len()), e))?;
        let owner = OwnedSource {
            source,
            cancellation,
            inputs,
            functions,
            contextual,
            modules,
            memory,
            _storage: storage,
        };
        let cell = ScriptCell::try_new(owner, |owner| {
            let program = Program::compile(&owner.source)?;
            let modules: Vec<_> = owner
                .modules
                .iter()
                .map(|(name, source)| Ok((name.as_str(), Program::compile(source)?)))
                .collect::<Result<_>>()?;
            let modules: Vec<_> = modules.iter().map(|(name, p)| (*name, p)).collect();
            let mut inputs = memory::Slots::new(&owner.memory, owner.inputs.len())
                .map_err(|e| memory_error(program.parsed.module.span, e))?;
            for (name, value) in &owner.inputs {
                inputs
                    .push((name.as_str(), value))
                    .map_err(|e| memory_error(program.parsed.module.span, e))?;
            }
            program.instantiate_on_budget(
                limits,
                &owner.cancellation,
                &inputs,
                &owner.functions,
                &owner.contextual,
                &modules,
                owner.memory.clone(),
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
            let function = instance.prepare_call(name, limits)?;
            let mut imported = memory::Slots::new(&instance.runtime.memory, arguments.len())
                .map_err(|e| instance.runtime.environment_error(instance.span, e))?;
            for value in arguments {
                imported
                    .push(
                        EngineValue::import_state(value, &instance.runtime.memory, 0)
                            .map_err(|e| instance.runtime.environment_error(instance.span, e))?,
                    )
                    .map_err(|e| instance.runtime.environment_error(instance.span, e))?;
            }
            let value = instance
                .runtime
                .call(function, Cow::Borrowed(&imported), instance.span)?;
            value.to_state().map_err(|e| instance.state_error(e))
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
    pub fn peak_memory_usage(&mut self) -> usize {
        self.with_instance(|i| i.peak_memory_usage())
    }
    pub fn memory_usage(&mut self) -> usize {
        self.with_instance(|i| i.memory_usage())
    }
}

impl ScriptInstance<'_, '_> {
    pub fn set_memory_limit(&mut self, bytes: usize) -> Result<()> {
        self.runtime
            .memory
            .set_limit(bytes)
            .map_err(|e| self.runtime.environment_error(self.span, e))?;
        Ok(())
    }
    pub fn memory_usage(&self) -> usize {
        self.runtime.memory.live_bytes()
    }
    pub fn peak_memory_usage(&self) -> usize {
        self.runtime.memory.peak_bytes()
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

fn public_storage(value: &Value<'_>, depth: usize) -> Option<usize> {
    if depth > 64 {
        return None;
    }
    let mut n = 0usize;
    match value {
        Value::String(v) => n = v.capacity(),
        Value::Vector(v) => n = v.capacity().checked_mul(std::mem::size_of::<f64>())?,
        Value::List(v) | Value::Tuple(v) | Value::Variant(_, v) => {
            n = v.capacity().checked_mul(std::mem::size_of::<Value<'_>>())?;
            for v in v {
                n = n.checked_add(public_storage(v, depth + 1)?)?;
            }
        }
        Value::Record(v) => {
            n = v
                .len()
                .checked_mul(12 * std::mem::size_of::<(String, Value<'_>)>() + 256)?;
            for (k, v) in v {
                n = n
                    .checked_add(k.capacity())?
                    .checked_add(public_storage(v, depth + 1)?)?;
            }
        }
        Value::Matrix(_) => n = std::mem::size_of::<crate::Matrix4>(),
        Value::Quaternion(_) => n = std::mem::size_of::<crate::Quaternion>(),
        Value::Polygon(v) => n = v.storage_bytes(),
        // Rc host resources and externally created executable values remain host-owned.
        _ => {}
    }
    Some(n)
}
