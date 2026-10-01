//! Private, shared values. Public Value remains the host boundary representation.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(super) enum EngineValue<'s> {
    HostObject(crate::HostObject),
    Angle(f64),
    Variant(&'static str, memory::Buffer<EngineValue<'s>>),
    Mesh(memory::Shared<EngineMesh>),
    Quaternion(memory::Shared<crate::Quaternion>),
    String(memory::Text),
    Record(memory::Record<EngineValue<'s>>),
    Matrix(memory::Shared<crate::Matrix4>),
    Polygon(memory::Shared<EnginePolygon>),
    Number(f64),
    Vector(memory::Buffer<f64>),
    Bool(bool),
    Null,
    Range { start: f64, end: f64, step: f64 },
    Sequence(Rc<Sequence<'s>>),
    List(memory::Buffer<EngineValue<'s>>),
    Tuple(memory::Buffer<EngineValue<'s>>),
    Function(Rc<Closure<'s>>),
    Builtin(Builtin),
    Host(Rc<HostFunction>),
}
#[derive(Debug, PartialEq)]
pub(super) struct EnginePolygon {
    points: memory::Buffer<[f64; 2]>,
}
impl EnginePolygon {
    pub(super) fn new(
        budget: &memory::Budget,
        points: memory::Slots<[f64; 2]>,
    ) -> std::result::Result<memory::Shared<Self>, &'static str> {
        if points.len() < 3 || points.iter().flatten().any(|v| !v.is_finite()) {
            return Err("Polygon requires at least three finite 2D points");
        }
        let points = memory::Buffer::from_slots(budget, points)
            .map_err(|_| "Instance memory limit exceeded")?;
        memory::Shared::new(budget, Self { points }).map_err(|_| "Instance memory limit exceeded")
    }
    pub(super) fn points(&self) -> &[[f64; 2]] {
        &self.points
    }
}
#[derive(Debug, PartialEq)]
pub(super) struct EngineMesh {
    vertices: memory::Buffer<[f64; 3]>,
    triangles: memory::Buffer<[usize; 3]>,
}
impl EngineMesh {
    pub(super) fn new(
        budget: &memory::Budget,
        vertices: memory::Slots<[f64; 3]>,
        triangles: memory::Slots<[usize; 3]>,
    ) -> std::result::Result<memory::Shared<Self>, &'static str> {
        if vertices.iter().flatten().any(|x| !x.is_finite()) {
            return Err("Mesh vertices must be finite");
        }
        if triangles.iter().any(|t| {
            t.iter().any(|i| *i >= vertices.len()) || t[0] == t[1] || t[1] == t[2] || t[0] == t[2]
        }) {
            return Err("Invalid triangle indices");
        }
        let vertices = memory::Buffer::from_slots(budget, vertices)
            .map_err(|_| "Instance memory limit exceeded")?;
        let triangles = memory::Buffer::from_slots(budget, triangles)
            .map_err(|_| "Instance memory limit exceeded")?;
        memory::Shared::new(
            budget,
            Self {
                vertices,
                triangles,
            },
        )
        .map_err(|_| "Instance memory limit exceeded")
    }
    pub(super) fn vertices(&self) -> &[[f64; 3]] {
        &self.vertices
    }
    pub(super) fn triangles(&self) -> &[[usize; 3]] {
        &self.triangles
    }
}
impl<'s> EngineValue<'s> {
    pub(super) fn import(
        value: &Value<'s>,
        budget: &memory::Budget,
    ) -> std::result::Result<Self, memory::AllocationError> {
        Self::import_depth(value, budget, 0)
    }
    fn import_depth(
        value: &Value<'s>,
        budget: &memory::Budget,
        depth: usize,
    ) -> std::result::Result<Self, memory::AllocationError> {
        if depth > 64 {
            return Err(memory::AllocationError::Depth);
        }
        use Value as V;
        Ok(match value {
            V::HostObject(v) => Self::HostObject(v.clone()),
            V::Angle(v) => Self::Angle(*v),
            V::String(v) => Self::String(memory::Text::from_str(budget, v)?),
            V::Number(v) => Self::Number(*v),
            V::Bool(v) => Self::Bool(*v),
            V::Null => Self::Null,
            V::Vector(v) => Self::Vector(memory::Buffer::from_iter(budget, v.iter().copied())?),
            V::List(v) | V::Tuple(v) | V::Variant(_, v) => {
                let mut values = memory::Slots::new(budget, v.len())?;
                for value in v {
                    values.push(Self::import_depth(value, budget, depth + 1)?)?;
                }
                let values = memory::Buffer::from_slots(budget, values)?;
                match value {
                    V::List(_) => Self::List(values),
                    V::Tuple(_) => Self::Tuple(values),
                    V::Variant(tag, _) => Self::Variant(tag, values),
                    _ => unreachable!(),
                }
            }
            V::Record(v) => {
                let mut fields = memory::Slots::new(budget, v.len())?;
                for (key, value) in v {
                    fields.push((
                        memory::Text::from_str(budget, key)?,
                        Self::import_depth(value, budget, depth + 1)?,
                    ))?;
                }
                Self::Record(memory::Record::from_slots(budget, fields)?)
            }
            V::Range { start, end, step } => Self::Range {
                start: *start,
                end: *end,
                step: *step,
            },
            V::Matrix(v) => Self::Matrix(memory::Shared::new(budget, (**v).clone())?),
            V::Quaternion(v) => Self::Quaternion(memory::Shared::new(budget, (**v).clone())?),
            V::Polygon(v) => {
                let mut points = memory::Slots::new(budget, v.points().len())?;
                points.extend(v.points().iter().copied())?;
                Self::Polygon(
                    EnginePolygon::new(budget, points)
                        .map_err(|_| memory::AllocationError::Limit)?,
                )
            }
            V::Mesh(v) => {
                let mut vertices = memory::Slots::new(budget, v.vertices().len())?;
                vertices.extend(v.vertices().iter().copied())?;
                let mut triangles = memory::Slots::new(budget, v.triangles().len())?;
                triangles.extend(v.triangles().iter().copied())?;
                Self::Mesh(
                    EngineMesh::new(budget, vertices, triangles)
                        .map_err(|_| memory::AllocationError::Limit)?,
                )
            }
            V::Sequence(v) => Self::Sequence(Self::import_sequence(v, budget, depth)?),
            V::Function(v) => Self::Function(Self::import_function(v, budget, depth)?),
            V::Builtin(v) => Self::Builtin(*v),
            V::Host(v) => Self::Host(v.clone()),
        })
    }
    /// Exported buffers belong to the caller, not the script's private heap.
    pub(super) fn export(&self) -> Value<'s> {
        match self {
            Self::HostObject(v) => Value::HostObject(v.clone()),
            Self::Angle(v) => Value::Angle(*v),
            Self::String(v) => Value::String(v.as_str().to_owned()),
            Self::Number(v) => Value::Number(*v),
            Self::Bool(v) => Value::Bool(*v),
            Self::Null => Value::Null,
            Self::Vector(v) => Value::Vector(v.to_vec()),
            Self::List(v) => Value::List(v.iter().map(Self::export).collect()),
            Self::Tuple(v) => Value::Tuple(v.iter().map(Self::export).collect()),
            Self::Variant(tag, v) => Value::Variant(tag, v.iter().map(Self::export).collect()),
            Self::Record(v) => Value::Record(
                v.iter()
                    .map(|(k, v)| (k.as_str().to_owned(), v.export()))
                    .collect(),
            ),
            Self::Matrix(v) => Value::Matrix(Box::new((**v).clone())),
            Self::Quaternion(v) => Value::Quaternion(Box::new((**v).clone())),
            Self::Polygon(v) => {
                Value::Polygon(crate::Polygon::new(v.points().to_vec()).expect("validated polygon"))
            }
            Self::Mesh(v) => Value::Mesh(Rc::new(
                crate::Mesh::new(v.vertices().to_vec(), v.triangles().to_vec())
                    .expect("validated mesh"),
            )),
            Self::Range { start, end, step } => Value::Range {
                start: *start,
                end: *end,
                step: *step,
            },
            Self::Sequence(v) => Value::Sequence(v.clone()),
            Self::Function(v) => Value::Function(v.clone()),
            Self::Builtin(v) => Value::Builtin(*v),
            Self::Host(v) => Value::Host(v.clone()),
        }
    }
}

impl ValueType {
    pub(super) fn accepts_engine(&self, value: &EngineValue<'_>) -> bool {
        match (self, value) {
            (Self::HostObject(name), EngineValue::HostObject(object)) => {
                *name == object.type_name() && object.is_alive()
            }
            (Self::Sequence, EngineValue::Sequence(_) | EngineValue::Range { .. }) => true,
            (Self::Angle, EngineValue::Angle(angle)) => angle.is_finite(),
            (Self::Option(_), EngineValue::Variant("None", values)) => values.is_empty(),
            (Self::Option(ty), EngineValue::Variant("Some", values)) => {
                values.len() == 1 && ty.accepts_engine(&values[0])
            }
            (Self::Result(ty, _), EngineValue::Variant("Ok", values))
            | (Self::Result(_, ty), EngineValue::Variant("Err", values)) => {
                values.len() == 1 && ty.accepts_engine(&values[0])
            }
            (Self::Tuple(types), EngineValue::Tuple(values)) => {
                types.len() == values.len()
                    && types
                        .iter()
                        .zip(values)
                        .all(|(ty, value)| ty.accepts_engine(value))
            }
            (Self::Mesh, EngineValue::Mesh(_)) => true,
            (Self::Quaternion, EngineValue::Quaternion(_)) => true,
            (Self::Matrix4, EngineValue::Matrix(matrix)) => {
                matrix.rows().iter().flatten().all(|n| n.is_finite())
            }
            (Self::String, EngineValue::String(_)) => true,
            (Self::Number, EngineValue::Number(n)) => n.is_finite(),
            (Self::Bool, EngineValue::Bool(_))
            | (Self::Null, EngineValue::Null)
            | (Self::Polygon, EngineValue::Polygon(_)) => true,
            (Self::Vector(size), EngineValue::Vector(values)) => {
                values.len() == *size && values.iter().all(|n| n.is_finite())
            }
            (Self::List(element), EngineValue::List(values)) => {
                values.iter().all(|v| element.accepts_engine(v))
            }
            _ => false,
        }
    }
}

impl EnginePolygon {
    pub(super) fn translated(
        &self,
        budget: &memory::Budget,
        offset: [f64; 2],
    ) -> std::result::Result<memory::Shared<Self>, &'static str> {
        let mut points = memory::Slots::new(budget, self.points.len())
            .map_err(|_| "Instance memory limit exceeded")?;
        points
            .extend(
                self.points
                    .iter()
                    .map(|p| [p[0] + offset[0], p[1] + offset[1]]),
            )
            .map_err(|_| "Instance memory limit exceeded")?;
        Self::new(budget, points)
    }
    pub(super) fn rotated(
        &self,
        budget: &memory::Budget,
        angle: f64,
    ) -> std::result::Result<memory::Shared<Self>, &'static str> {
        let (sin, cos) = angle.sin_cos();
        let mut points = memory::Slots::new(budget, self.points.len())
            .map_err(|_| "Instance memory limit exceeded")?;
        points
            .extend(
                self.points
                    .iter()
                    .map(|p| [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos]),
            )
            .map_err(|_| "Instance memory limit exceeded")?;
        Self::new(budget, points)
    }
}

impl memory::ItemDepth for EngineValue<'_> {
    fn depth(&self) -> usize {
        match self {
            Self::List(v) | Self::Tuple(v) | Self::Variant(_, v) => 1 + v.item_depth(),
            Self::Record(v) => 1 + v.item_depth(),
            _ => 0,
        }
    }
}

impl<'s> EngineValue<'s> {
    pub(super) fn import_state(
        value: &StateValue,
        b: &memory::Budget,
        depth: usize,
    ) -> std::result::Result<Self, memory::AllocationError> {
        if depth > 64 {
            return Err(memory::AllocationError::Depth);
        }
        Ok(match value {
            StateValue::Null => Self::Null,
            StateValue::Bool(v) => Self::Bool(*v),
            StateValue::Number(v) if v.is_finite() => Self::Number(*v),
            StateValue::Number(_) => return Err(memory::AllocationError::Unsupported),
            StateValue::String(v) => Self::String(memory::Text::from_str(b, v)?),
            StateValue::List(v) => {
                let mut items = memory::Slots::new(b, v.len())?;
                for v in v {
                    items.push(Self::import_state(v, b, depth + 1)?)?;
                }
                Self::List(memory::Buffer::from_slots(b, items)?)
            }
            StateValue::Record(v) => {
                let mut fields = memory::Slots::new(b, v.len())?;
                for (k, v) in v {
                    fields.push((
                        memory::Text::from_str(b, k)?,
                        Self::import_state(v, b, depth + 1)?,
                    ))?;
                }
                Self::Record(memory::Record::from_slots(b, fields)?)
            }
        })
    }
    /// Reservation for the stored public initial-result snapshot. BTree nodes
    /// use a conservative full-node bound per entry, including edge storage.
    pub(super) fn export_bytes(&self, limit: usize) -> Option<usize> {
        use std::mem::size_of;
        let items = |v: &[Self]| -> Option<usize> {
            let mut n = v.len().checked_mul(size_of::<Value<'s>>())?;
            if n > limit {
                return None;
            }
            for v in v {
                n = n.checked_add(v.export_bytes(limit - n)?)?;
            }
            Some(n)
        };
        let bytes = match self {
            Self::String(v) => v.len(),
            Self::Vector(v) => v.len().checked_mul(size_of::<f64>())?,
            Self::List(v) | Self::Tuple(v) | Self::Variant(_, v) => items(v)?,
            Self::Record(v) => {
                let mut n = v
                    .len()
                    .checked_mul(12 * size_of::<(String, Value<'s>)>() + 256)?;
                if n > limit {
                    return None;
                }
                for (k, v) in v {
                    n = n.checked_add(k.len())?;
                    if n > limit {
                        return None;
                    }
                    n = n.checked_add(v.export_bytes(limit - n)?)?;
                }
                n
            }
            Self::Matrix(_) => size_of::<crate::Matrix4>(),
            Self::Quaternion(_) => size_of::<crate::Quaternion>(),
            Self::Polygon(v) => v.points().len().checked_mul(size_of::<[f64; 2]>())?,
            Self::Mesh(v) => memory::rc_bytes::<crate::Mesh>()
                .checked_add(v.vertices().len().checked_mul(size_of::<[f64; 3]>())?)?
                .checked_add(v.triangles().len().checked_mul(size_of::<[usize; 3]>())?)?,
            _ => 0,
        };
        (bytes <= limit).then_some(bytes)
    }
}

impl<'s> EngineValue<'s> {
    fn lease(
        b: &memory::Budget,
        bytes: usize,
    ) -> std::result::Result<memory::Shared<memory::Reservation>, memory::AllocationError> {
        memory::Shared::new(b, b.reservation(bytes)?)
    }
    fn import_environment(
        env: &Environment<'s>,
        b: &memory::Budget,
        depth: usize,
    ) -> std::result::Result<memory::Shared<Environment<'s>>, memory::AllocationError> {
        if depth > 64 {
            return Err(memory::AllocationError::Depth);
        }
        let mut result = Environment::new(b);
        result.bindings.reserve(env.bindings.len())?;
        for (name, binding) in env.bindings.iter() {
            let binding = match binding {
                Binding::Value(v) => Binding::Value(Self::copy_engine(v, b, depth + 1)?),
                // An externally retained mutable ID is opaque. cell_index rejects
                // it on use, preserving the existing foreign-capture diagnostic.
                Binding::Cell(id) => Binding::Cell(id.clone()),
            };
            result.bindings.push((*name, binding))?;
        }
        if let Some(parent) = &env.parent {
            result.parent = Some(Self::import_environment(parent, b, depth + 1)?);
        }
        memory::Shared::new(b, result)
    }
    fn import_function(
        f: &Rc<Closure<'s>>,
        b: &memory::Budget,
        depth: usize,
    ) -> std::result::Result<Rc<Closure<'s>>, memory::AllocationError> {
        if b.owns(&f._allocation) {
            return Ok(f.clone());
        }
        if depth > 64 {
            return Err(memory::AllocationError::Depth);
        }
        let bytes = ast_memory::expressions(&f.parameters)
            + match &f.body {
                FunctionBody::Expression(e) => ast_memory::expression(e),
                FunctionBody::Block(v) => ast_memory::block(v),
            }
            + memory::rc_bytes::<Closure<'s>>();
        let allocation = Self::lease(b, bytes)?;
        let mut types = memory::Slots::new(b, f.parameter_types.len())?;
        for ty in f.parameter_types.iter() {
            types.push(ty.as_ref().map(|t| Self::copy_type(t, b)).transpose()?)?;
        }
        Ok(Rc::new(Closure {
            module: f.module,
            parameters: f.parameters.clone(),
            parameter_types: memory::Buffer::from_slots(b, types)?,
            result_type: f
                .result_type
                .as_ref()
                .map(|t| Self::copy_type(t, b))
                .transpose()?,
            body: f.body.clone(),
            name: f.name,
            environment: Self::import_environment(&f.environment, b, depth + 1)?,
            references: memory::Buffer::from_iter(b, f.references.iter().cloned())?,
            _allocation: allocation,
        }))
    }
    fn copy_type(
        t: &ValueType,
        b: &memory::Budget,
    ) -> std::result::Result<memory::Shared<ast_memory::RuntimeType>, memory::AllocationError> {
        fn bytes(t: &ValueType) -> usize {
            match t {
                ValueType::Option(t) | ValueType::List(t) => {
                    std::mem::size_of::<ValueType>() + bytes(t)
                }
                ValueType::Result(a, c) => {
                    2 * std::mem::size_of::<ValueType>() + bytes(a) + bytes(c)
                }
                ValueType::Tuple(v) => {
                    std::mem::size_of_val(v.as_slice()) + v.iter().map(bytes).sum::<usize>()
                }
                _ => 0,
            }
        }
        let storage = b.reservation(bytes(t))?;
        memory::Shared::new(b, ast_memory::RuntimeType::new(t.clone(), storage))
    }
    fn import_sequence(
        v: &Rc<Sequence<'s>>,
        b: &memory::Budget,
        depth: usize,
    ) -> std::result::Result<Rc<Sequence<'s>>, memory::AllocationError> {
        if v._allocation.as_ref().is_some_and(|r| b.owns(r)) {
            return Ok(v.clone());
        }
        if depth > 64 {
            return Err(memory::AllocationError::Depth);
        }
        let allocation = Self::lease(b, memory::rc_bytes::<Sequence<'s>>())?;
        let source = match &v.source {
            SequenceSource::List(items) => {
                let mut copy = memory::Slots::new(b, items.len())?;
                for item in items.iter() {
                    copy.push(Self::copy_engine(item, b, depth + 1)?)?;
                }
                SequenceSource::List(memory::Buffer::from_slots(b, copy)?)
            }
            other => other.clone(),
        };
        let mut stages = SequenceStages::default();
        for stage in v.stages.iter() {
            stages.append(
                b,
                SequenceStage {
                    callback: Self::copy_engine(&stage.callback, b, depth + 1)?,
                    filter: stage.filter,
                    span: stage.span,
                    module: stage.module,
                },
            )?;
        }
        Ok(Rc::new(Sequence {
            source,
            stages,
            _allocation: Some(allocation),
        }))
    }
    fn copy_engine(
        v: &Self,
        b: &memory::Budget,
        depth: usize,
    ) -> std::result::Result<Self, memory::AllocationError> {
        if depth > 64 {
            return Err(memory::AllocationError::Depth);
        }
        Ok(match v {
            Self::Function(f) => Self::Function(Self::import_function(f, b, depth)?),
            Self::Sequence(s) => Self::Sequence(Self::import_sequence(s, b, depth)?),
            Self::String(v) => Self::String(memory::Text::from_str(b, v.as_str())?),
            Self::Vector(v) => Self::Vector(memory::Buffer::from_iter(b, v.iter().copied())?),
            Self::List(values) | Self::Tuple(values) | Self::Variant(_, values) => {
                let mut items = memory::Slots::new(b, values.len())?;
                for v in values.iter() {
                    items.push(Self::copy_engine(v, b, depth + 1)?)?;
                }
                let items = memory::Buffer::from_slots(b, items)?;
                match v {
                    Self::List(_) => Self::List(items),
                    Self::Tuple(_) => Self::Tuple(items),
                    Self::Variant(tag, _) => Self::Variant(tag, items),
                    _ => unreachable!(),
                }
            }
            Self::Record(v) => {
                let mut fields = memory::Slots::new(b, v.len())?;
                for (k, v) in v.iter() {
                    fields.push((
                        memory::Text::from_str(b, k.as_str())?,
                        Self::copy_engine(v, b, depth + 1)?,
                    ))?;
                }
                Self::Record(memory::Record::from_slots(b, fields)?)
            }
            Self::Matrix(v) => Self::Matrix(memory::Shared::new(b, (**v).clone())?),
            Self::Quaternion(v) => Self::Quaternion(memory::Shared::new(b, (**v).clone())?),
            Self::Polygon(v) => {
                let mut points = memory::Slots::new(b, v.points().len())?;
                points.extend(v.points().iter().copied())?;
                Self::Polygon(
                    EnginePolygon::new(b, points).map_err(|_| memory::AllocationError::Limit)?,
                )
            }
            Self::Mesh(v) => {
                let mut vertices = memory::Slots::new(b, v.vertices().len())?;
                vertices.extend(v.vertices().iter().copied())?;
                let mut triangles = memory::Slots::new(b, v.triangles().len())?;
                triangles.extend(v.triangles().iter().copied())?;
                Self::Mesh(
                    EngineMesh::new(b, vertices, triangles)
                        .map_err(|_| memory::AllocationError::Limit)?,
                )
            }
            other => other.clone(),
        })
    }
}

impl EngineValue<'_> {
    pub(super) fn to_state(&self) -> std::result::Result<StateValue, String> {
        Ok(match self {
            Self::Null=>StateValue::Null,Self::Bool(v)=>StateValue::Bool(*v),
            Self::Number(v) if v.is_finite()=>StateValue::Number(*v),
            Self::String(v)=>StateValue::String(v.as_str().to_owned()),
            Self::List(v)=>StateValue::List(v.iter().map(Self::to_state).collect::<std::result::Result<_,_>>()?),
            Self::Record(v)=>StateValue::Record(v.iter().map(|(k,v)|Ok((k.as_str().to_owned(),v.to_state()?))).collect::<std::result::Result<_,String>>()?),
            _=>return Err("State supports only finite numbers, strings, booleans, null, lists and records; restore host objects through engine IDs".into()),
        })
    }
}
