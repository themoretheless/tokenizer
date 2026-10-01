//! Public host representation. Interpreter storage is intentionally separate.
use super::*;
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

impl<'s> From<Value<'s>> for RuntimeValue<'s> {
    fn from(value: Value<'s>) -> Self {
        match value {
            Value::HostObject(value) => Self::HostObject(value),
            Value::Angle(value) => Self::Angle(value),
            Value::Mesh(value) => Self::Mesh(value),
            Value::Quaternion(value) => Self::Quaternion(value),
            Value::String(value) => Self::String(value),
            Value::Matrix(value) => Self::Matrix(value),
            Value::Polygon(value) => Self::Polygon(value),
            Value::Number(value) => Self::Number(value),
            Value::Vector(value) => Self::Vector(value),
            Value::Bool(value) => Self::Bool(value),
            Value::Sequence(value) => Self::Sequence(value),
            Value::Function(value) => Self::Function(value),
            Value::Builtin(value) => Self::Builtin(value),
            Value::Host(value) => Self::Host(value),
            Value::Null => Self::Null,
            Value::Range { start, end, step } => Self::Range { start, end, step },
            Value::List(values) => Self::List(values.into_iter().map(Into::into).collect()),
            Value::Tuple(values) => Self::Tuple(values.into_iter().map(Into::into).collect()),
            Value::Variant(name, values) => {
                Self::Variant(name, values.into_iter().map(Into::into).collect())
            }
            Value::Record(values) => Self::Record(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
            ),
        }
    }
}

impl<'s> From<RuntimeValue<'s>> for Value<'s> {
    fn from(value: RuntimeValue<'s>) -> Self {
        match value {
            RuntimeValue::HostObject(value) => Self::HostObject(value),
            RuntimeValue::Angle(value) => Self::Angle(value),
            RuntimeValue::Mesh(value) => Self::Mesh(value),
            RuntimeValue::Quaternion(value) => Self::Quaternion(value),
            RuntimeValue::String(value) => Self::String(value),
            RuntimeValue::Matrix(value) => Self::Matrix(value),
            RuntimeValue::Polygon(value) => Self::Polygon(value),
            RuntimeValue::Number(value) => Self::Number(value),
            RuntimeValue::Vector(value) => Self::Vector(value),
            RuntimeValue::Bool(value) => Self::Bool(value),
            RuntimeValue::Sequence(value) => Self::Sequence(value),
            RuntimeValue::Function(value) => Self::Function(value),
            RuntimeValue::Builtin(value) => Self::Builtin(value),
            RuntimeValue::Host(value) => Self::Host(value),
            RuntimeValue::Null => Self::Null,
            RuntimeValue::Range { start, end, step } => Self::Range { start, end, step },
            RuntimeValue::List(values) => Self::List(values.into_iter().map(Into::into).collect()),
            RuntimeValue::Tuple(values) => {
                Self::Tuple(values.into_iter().map(Into::into).collect())
            }
            RuntimeValue::Variant(name, values) => {
                Self::Variant(name, values.into_iter().map(Into::into).collect())
            }
            RuntimeValue::Record(values) => Self::Record(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
            ),
        }
    }
}
