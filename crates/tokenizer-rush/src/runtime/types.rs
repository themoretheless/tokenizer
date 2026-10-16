use super::sequence::HostSource;
use super::{
    Builtin, CaptureReference, Environment, Result, Runtime, Sequence, SequenceSource,
    SequenceStages, memory,
};
use crate::{Block, Expr};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use themoretheless_tokenizer_core::Span;

/// Unboxed inline numeric vector (`vec2`, `vec3`, `vec4`) stored directly on the stack
/// without heap allocation.
#[derive(Clone, Copy)]
pub struct InlineVec {
    len: u8,
    data: [f64; 4],
}

impl Default for InlineVec {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl InlineVec {
    #[inline]
    pub const fn new() -> Self {
        Self {
            len: 0,
            data: [0.0; 4],
        }
    }

    #[inline]
    pub const fn vec2(x: f64, y: f64) -> Self {
        Self {
            len: 2,
            data: [x, y, 0.0, 0.0],
        }
    }

    #[inline]
    pub const fn vec3(x: f64, y: f64, z: f64) -> Self {
        Self {
            len: 3,
            data: [x, y, z, 0.0],
        }
    }

    #[inline]
    pub const fn vec4(x: f64, y: f64, z: f64, w: f64) -> Self {
        Self {
            len: 4,
            data: [x, y, z, w],
        }
    }

    #[inline]
    pub fn from_slice(slice: &[f64]) -> Self {
        debug_assert!(slice.len() <= 4, "InlineVec supports up to 4 components");
        let len = slice.len().min(4);
        let mut data = [0.0; 4];
        data[..len].copy_from_slice(&slice[..len]);
        Self {
            len: len as u8,
            data,
        }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn capacity(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn push(&mut self, value: f64) {
        debug_assert!((self.len as usize) < 4, "InlineVec capacity exceeded");
        let idx = self.len as usize;
        if idx < 4 {
            self.data[idx] = value;
            self.len += 1;
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[f64] {
        &self.data[..self.len as usize]
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [f64] {
        &mut self.data[..self.len as usize]
    }
}

impl std::ops::Deref for InlineVec {
    type Target = [f64];

    #[inline]
    fn deref(&self) -> &[f64] {
        self.as_slice()
    }
}

impl std::ops::DerefMut for InlineVec {
    #[inline]
    fn deref_mut(&mut self) -> &mut [f64] {
        self.as_mut_slice()
    }
}

impl std::fmt::Debug for InlineVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl PartialEq for InlineVec {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialEq<[f64]> for InlineVec {
    #[inline]
    fn eq(&self, other: &[f64]) -> bool {
        self.as_slice() == other
    }
}

impl PartialEq<&[f64]> for InlineVec {
    #[inline]
    fn eq(&self, other: &&[f64]) -> bool {
        self.as_slice() == *other
    }
}

impl PartialEq<InlineVec> for [f64] {
    #[inline]
    fn eq(&self, other: &InlineVec) -> bool {
        self == other.as_slice()
    }
}

impl PartialEq<InlineVec> for &[f64] {
    #[inline]
    fn eq(&self, other: &InlineVec) -> bool {
        *self == other.as_slice()
    }
}

impl PartialEq<Vec<f64>> for InlineVec {
    #[inline]
    fn eq(&self, other: &Vec<f64>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> PartialEq<[f64; N]> for InlineVec {
    #[inline]
    fn eq(&self, other: &[f64; N]) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<const N: usize> From<[f64; N]> for InlineVec {
    #[inline]
    fn from(arr: [f64; N]) -> Self {
        Self::from_slice(&arr)
    }
}

impl From<&[f64]> for InlineVec {
    #[inline]
    fn from(slice: &[f64]) -> Self {
        Self::from_slice(slice)
    }
}

impl From<Vec<f64>> for InlineVec {
    #[inline]
    fn from(vec: Vec<f64>) -> Self {
        Self::from_slice(&vec)
    }
}

impl FromIterator<f64> for InlineVec {
    #[inline]
    fn from_iter<T: IntoIterator<Item = f64>>(iter: T) -> Self {
        let mut out = Self::new();
        for item in iter {
            out.push(item);
        }
        out
    }
}

impl IntoIterator for InlineVec {
    type Item = f64;
    type IntoIter = std::iter::Take<std::array::IntoIter<f64, 4>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.data.into_iter().take(self.len as usize)
    }
}

impl<'a> IntoIterator for &'a InlineVec {
    type Item = &'a f64;
    type IntoIter = std::slice::Iter<'a, f64>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a> IntoIterator for &'a mut InlineVec {
    type Item = &'a mut f64;
    type IntoIter = std::slice::IterMut<'a, f64>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}

/// Shared immutable record/struct shape: sorted unique field names.
pub type RecordShape = Rc<[String]>;

/// Dense slotted storage for Rush records and struct payloads.
///
/// Field names are stored in a shared sorted [`RecordShape`] (`Rc<[String]>`),
/// while field values live in a contiguous `Vec<Value<'s>>` indexed by slot position.
#[derive(Clone)]
pub struct RecordMap<'s> {
    shape: RecordShape,
    values: Vec<Value<'s>>,
}

impl<'s> Default for RecordMap<'s> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<'s> RecordMap<'s> {
    #[inline]
    pub fn new() -> Self {
        Self {
            shape: Rc::from([] as [String; 0]),
            values: Vec::new(),
        }
    }

    #[inline]
    pub fn from_shape_and_values(shape: RecordShape, values: Vec<Value<'s>>) -> Self {
        debug_assert_eq!(shape.len(), values.len());
        Self { shape, values }
    }

    pub fn from_unique_entries(mut entries: Vec<(String, Value<'s>)>) -> Self {
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut keys = Vec::with_capacity(entries.len());
        let mut values = Vec::with_capacity(entries.len());
        for (k, v) in entries {
            keys.push(k);
            values.push(v);
        }
        Self {
            shape: Rc::from(keys),
            values,
        }
    }

    #[inline]
    pub fn into_parts(self) -> (RecordShape, Vec<Value<'s>>) {
        (self.shape, self.values)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[inline]
    pub fn shape(&self) -> &RecordShape {
        &self.shape
    }

    #[inline]
    pub fn set_shape(&mut self, shape: RecordShape) {
        debug_assert_eq!(shape.len(), self.values.len());
        self.shape = shape;
    }

    #[inline]
    pub fn keys_slice(&self) -> &[String] {
        &self.shape
    }

    #[inline]
    pub fn values_slice(&self) -> &[Value<'s>] {
        &self.values
    }

    #[inline]
    pub fn values_mut_slice(&mut self) -> &mut [Value<'s>] {
        &mut self.values
    }

    #[inline]
    pub fn slot_of(&self, key: &str) -> Option<usize> {
        let keys = &*self.shape;
        if keys.len() <= 8 {
            keys.iter().position(|k| k.as_str() == key)
        } else {
            keys.binary_search_by(|k| k.as_str().cmp(key)).ok()
        }
    }

    #[inline]
    pub fn contains_key<Q: AsRef<str> + ?Sized>(&self, key: &Q) -> bool {
        self.slot_of(key.as_ref()).is_some()
    }

    #[inline]
    pub fn get<Q: AsRef<str> + ?Sized>(&self, key: &Q) -> Option<&Value<'s>> {
        let slot = self.slot_of(key.as_ref())?;
        self.values.get(slot)
    }

    #[inline]
    pub fn get_mut<Q: AsRef<str> + ?Sized>(&mut self, key: &Q) -> Option<&mut Value<'s>> {
        let slot = self.slot_of(key.as_ref())?;
        self.values.get_mut(slot)
    }

    #[inline]
    pub fn get_slot(&self, slot: usize) -> Option<&Value<'s>> {
        self.values.get(slot)
    }

    #[inline]
    pub fn get_slot_mut(&mut self, slot: usize) -> Option<&mut Value<'s>> {
        self.values.get_mut(slot)
    }

    #[inline]
    pub fn contains_key_with_hint(&self, key: &str, hint: u32) -> Option<u32> {
        if self
            .shape
            .get(hint as usize)
            .is_some_and(|k| k.as_str() == key)
        {
            return Some(hint);
        }
        self.slot_of(key).map(|s| s as u32)
    }

    #[inline]
    pub fn get_with_hint(&self, key: &str, hint: u32) -> Option<(u32, &Value<'s>)> {
        let h = hint as usize;
        if self.shape.get(h).is_some_and(|k| k.as_str() == key) {
            return Some((hint, &self.values[h]));
        }
        let slot = self.slot_of(key)?;
        Some((slot as u32, &self.values[slot]))
    }

    #[inline]
    pub fn get_mut_with_hint(&mut self, key: &str, hint: u32) -> Option<(u32, &mut Value<'s>)> {
        let h = hint as usize;
        if self.shape.get(h).is_some_and(|k| k.as_str() == key) {
            return Some((hint, &mut self.values[h]));
        }
        let slot = self.slot_of(key)?;
        Some((slot as u32, &mut self.values[slot]))
    }

    pub fn insert(&mut self, key: String, value: Value<'s>) -> Option<Value<'s>> {
        match self
            .shape
            .binary_search_by(|k| k.as_str().cmp(key.as_str()))
        {
            Ok(slot) => Some(std::mem::replace(&mut self.values[slot], value)),
            Err(insert_idx) => {
                let mut new_keys = Vec::with_capacity(self.shape.len() + 1);
                new_keys.extend_from_slice(&self.shape[..insert_idx]);
                new_keys.push(key);
                new_keys.extend_from_slice(&self.shape[insert_idx..]);
                self.shape = Rc::from(new_keys);
                self.values.insert(insert_idx, value);
                None
            }
        }
    }

    pub fn remove<Q: AsRef<str> + ?Sized>(&mut self, key: &Q) -> Option<Value<'s>> {
        let slot = self.slot_of(key.as_ref())?;
        let mut new_keys = Vec::with_capacity(self.shape.len().saturating_sub(1));
        new_keys.extend_from_slice(&self.shape[..slot]);
        new_keys.extend_from_slice(&self.shape[slot + 1..]);
        self.shape = Rc::from(new_keys);
        Some(self.values.remove(slot))
    }

    #[inline]
    pub fn keys(&self) -> std::slice::Iter<'_, String> {
        self.shape.iter()
    }

    #[inline]
    pub fn values(&self) -> std::slice::Iter<'_, Value<'s>> {
        self.values.iter()
    }

    #[inline]
    pub fn values_mut(&mut self) -> std::slice::IterMut<'_, Value<'s>> {
        self.values.iter_mut()
    }

    #[inline]
    pub fn iter(
        &self,
    ) -> std::iter::Zip<std::slice::Iter<'_, String>, std::slice::Iter<'_, Value<'s>>> {
        self.shape.iter().zip(self.values.iter())
    }

    #[inline]
    pub fn iter_mut(
        &mut self,
    ) -> std::iter::Zip<std::slice::Iter<'_, String>, std::slice::IterMut<'_, Value<'s>>> {
        self.shape.iter().zip(self.values.iter_mut())
    }
}

impl<'s> std::fmt::Debug for RecordMap<'s> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<'s> PartialEq for RecordMap<'s> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        if self.values.len() != other.values.len() {
            return false;
        }
        (Rc::ptr_eq(&self.shape, &other.shape) || *self.shape == *other.shape)
            && self.values == other.values
    }
}

impl<'s> PartialEq<BTreeMap<String, Value<'s>>> for RecordMap<'s> {
    fn eq(&self, other: &BTreeMap<String, Value<'s>>) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<'s> From<BTreeMap<String, Value<'s>>> for RecordMap<'s> {
    fn from(map: BTreeMap<String, Value<'s>>) -> Self {
        let mut keys = Vec::with_capacity(map.len());
        let mut values = Vec::with_capacity(map.len());
        for (k, v) in map {
            keys.push(k);
            values.push(v);
        }
        Self {
            shape: Rc::from(keys),
            values,
        }
    }
}

impl<'s, const N: usize> From<[(String, Value<'s>); N]> for RecordMap<'s> {
    #[inline]
    fn from(arr: [(String, Value<'s>); N]) -> Self {
        Self::from_iter(arr)
    }
}

impl<'s> FromIterator<(String, Value<'s>)> for RecordMap<'s> {
    fn from_iter<T: IntoIterator<Item = (String, Value<'s>)>>(iter: T) -> Self {
        let mut entries: Vec<(String, Value<'s>)> = iter.into_iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut keys: Vec<String> = Vec::with_capacity(entries.len());
        let mut values: Vec<Value<'s>> = Vec::with_capacity(entries.len());
        for (k, v) in entries {
            if keys.last().is_some_and(|last| last == &k) {
                *values.last_mut().unwrap() = v;
            } else {
                keys.push(k);
                values.push(v);
            }
        }
        Self {
            shape: Rc::from(keys),
            values,
        }
    }
}

impl<'s> IntoIterator for RecordMap<'s> {
    type Item = (String, Value<'s>);
    type IntoIter = std::iter::Zip<std::vec::IntoIter<String>, std::vec::IntoIter<Value<'s>>>;

    fn into_iter(self) -> Self::IntoIter {
        let keys: Vec<String> = self.shape.iter().cloned().collect();
        keys.into_iter().zip(self.values)
    }
}

impl<'a, 's> IntoIterator for &'a RecordMap<'s> {
    type Item = (&'a String, &'a Value<'s>);
    type IntoIter = std::iter::Zip<std::slice::Iter<'a, String>, std::slice::Iter<'a, Value<'s>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, 's> IntoIterator for &'a mut RecordMap<'s> {
    type Item = (&'a String, &'a mut Value<'s>);
    type IntoIter =
        std::iter::Zip<std::slice::Iter<'a, String>, std::slice::IterMut<'a, Value<'s>>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
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
    Record(RecordMap<'s>),
    Matrix(Box<crate::Matrix4>),
    Polygon(crate::Polygon),
    Int(i64),
    Number(f64),
    Vector(InlineVec),
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

impl<'s> Value<'s> {
    /// Create a lazy source without opening a resource. Items are checked against item_type.
    pub fn host_sequence(factory: Rc<dyn super::HostSequence>, item_type: ValueType) -> Self {
        Self::Sequence(Rc::new(Sequence {
            source: SequenceSource::Host(HostSource { factory, item_type }),
            stages: SequenceStages::default(),
        }))
    }

    #[inline]
    pub const fn vec2(x: f64, y: f64) -> Self {
        Self::Vector(InlineVec::vec2(x, y))
    }

    #[inline]
    pub const fn vec3(x: f64, y: f64, z: f64) -> Self {
        Self::Vector(InlineVec::vec3(x, y, z))
    }

    #[inline]
    pub const fn vec4(x: f64, y: f64, z: f64, w: f64) -> Self {
        Self::Vector(InlineVec::vec4(x, y, z, w))
    }

    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    #[inline]
    pub fn as_exact_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Number(n)
                if n.is_finite()
                    && n.fract() == 0.0
                    && *n >= (i64::MIN as f64)
                    && *n < 9_223_372_036_854_775_808.0 =>
            {
                Some(*n as i64)
            }
            _ => None,
        }
    }

    #[inline]
    pub fn as_usize_index(&self) -> Option<usize> {
        match self {
            Value::Int(i) => usize::try_from(*i).ok(),
            Value::Number(n)
                if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= (usize::MAX as f64) =>
            {
                Some(*n as usize)
            }
            _ => None,
        }
    }

    pub fn coerce_for_type(self, ty: &ValueType) -> Self {
        match (ty, self) {
            (ValueType::Int, Value::Number(n))
                if n.is_finite()
                    && n.fract() == 0.0
                    && n >= (i64::MIN as f64)
                    && n < 9_223_372_036_854_775_808.0 =>
            {
                Value::Int(n as i64)
            }
            (ValueType::Number, Value::Int(i)) => Value::Number(i as f64),
            (ValueType::List(elem_ty), Value::List(items)) => Value::List(
                items
                    .into_iter()
                    .map(|v| v.coerce_for_type(elem_ty))
                    .collect(),
            ),
            (ValueType::Tuple(tys), Value::Tuple(items)) if tys.len() == items.len() => {
                Value::Tuple(
                    items
                        .into_iter()
                        .zip(tys)
                        .map(|(v, t)| v.coerce_for_type(t))
                        .collect(),
                )
            }
            (ValueType::Option(inner), Value::Variant("Some", mut items)) if items.len() == 1 => {
                let v = items.pop().unwrap().coerce_for_type(inner);
                Value::Variant("Some", vec![v])
            }
            (ValueType::Result(ok_ty, _), Value::Variant("Ok", mut items)) if items.len() == 1 => {
                let v = items.pop().unwrap().coerce_for_type(ok_ty);
                Value::Variant("Ok", vec![v])
            }
            (ValueType::Result(_, err_ty), Value::Variant("Err", mut items))
                if items.len() == 1 =>
            {
                let v = items.pop().unwrap().coerce_for_type(err_ty);
                Value::Variant("Err", vec![v])
            }
            (_, v) => v,
        }
    }

    /// Try to convert to an owned static value if all inner parts can be static.
    pub fn to_static(&self) -> Option<Value<'static>> {
        match self {
            Value::Null => Some(Value::Null),
            Value::Bool(b) => Some(Value::Bool(*b)),
            Value::Int(i) => Some(Value::Int(*i)),
            Value::Number(n) => Some(Value::Number(*n)),
            Value::Angle(a) => Some(Value::Angle(*a)),
            Value::String(s) => Some(Value::String(s.clone())),
            Value::Vector(v) => Some(Value::Vector(*v)),
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
                let mut static_values = Vec::with_capacity(fields.len());
                for v in fields.values_slice() {
                    static_values.push(v.to_static()?);
                }
                Some(Value::Record(RecordMap::from_shape_and_values(
                    fields.shape().clone(),
                    static_values,
                )))
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

#[derive(Clone, Debug, PartialEq)]
pub struct Closure<'s> {
    pub(super) module: Option<&'s str>,
    pub(super) parameters: Vec<Expr<'s>>,
    pub(super) parameter_types: Vec<Option<ValueType>>,
    pub(super) result_type: Option<ValueType>,
    /// Shared per program point; lambda evaluation no longer deep-clones AST.
    pub(super) body: Rc<FunctionBody<'s>>,
    pub(super) name: Option<&'s str>,
    pub(super) environment: memory::Shared<Environment<'s>>,
    pub(super) references: Rc<Vec<CaptureReference<'s>>>,
    pub(super) siblings: Rc<Vec<Closure<'s>>>,
}

impl<'s> Closure<'s> {
    pub fn name(&self) -> Option<&'s str> {
        self.name
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FunctionBody<'s> {
    Constructor {
        type_name: String,
        variant: Option<String>,
        shape: RecordShape,
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
    pub(super) fn locate(mut self, source: &str) -> Self {
        let prefix = source.get(..self.span.start).unwrap_or("");
        self.location = Some(SourceLocation {
            line: prefix.bytes().filter(|b| *b == b'\n').count() + 1,
            column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        });
        self
    }
}

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
    Int,
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
            "int" | "i64" => Some(Self::Int),
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

    pub(super) fn inferred(value: &Value<'_>) -> Option<Self> {
        Some(match value {
            Value::Int(_) => Self::Int,
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
            (Self::Int, Value::Int(_)) => true,
            (Self::Int, Value::Number(n)) => {
                n.is_finite()
                    && n.fract() == 0.0
                    && *n >= (i64::MIN as f64)
                    && *n < 9_223_372_036_854_775_808.0
            }
            (Self::Number, Value::Number(n)) => n.is_finite(),
            (Self::Number, Value::Int(_)) => true,
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
pub(super) type ContextualHostCallback =
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
    pub(super) callback: ContextualHostCallback,
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

impl<'s> Runtime<'_, 's> {
    pub(super) fn runtime_contract(&self, ty: &ValueType) -> ValueType {
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

    pub(super) fn annotation(&self, ty: &crate::Type<'_>) -> Result<ValueType> {
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

    pub(super) fn equal(
        &mut self,
        left: &Value<'_>,
        right: &Value<'_>,
        span: Span,
    ) -> Result<bool> {
        // Scalar fast path: identical step accounting without a worklist
        // allocation. Compound and mismatched values use the general walk.
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                self.charge(1, span)?;
                return Ok(a == b);
            }
            (Value::Number(a), Value::Number(b)) => {
                self.charge(1, span)?;
                return Ok(a == b);
            }
            (Value::Int(i), Value::Number(f)) | (Value::Number(f), Value::Int(i)) => {
                self.charge(1, span)?;
                return Ok(int_f64_eq(*i, *f));
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
                    if Rc::ptr_eq(a.shape(), b.shape()) {
                        for (k, (va, vb)) in a
                            .keys_slice()
                            .iter()
                            .zip(a.values_slice().iter().zip(b.values_slice()))
                        {
                            self.charge(k.len(), span)?;
                            pending.push((va, vb));
                        }
                    } else {
                        for ((ka, va), (kb, vb)) in a.iter().zip(b) {
                            self.charge(ka.len().max(kb.len()), span)?;
                            if ka != kb {
                                return Ok(false);
                            }
                            pending.push((va, vb));
                        }
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
                (Value::Int(i), Value::Number(f)) | (Value::Number(f), Value::Int(i)) => {
                    if !int_f64_eq(*i, *f) {
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
}

#[inline]
pub(crate) fn is_int_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    (bytes.len() >= 2
        && bytes[0] == b'0'
        && matches!(bytes[1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
        || text.ends_with('i')
        || text.ends_with("i64")
}

pub(crate) fn parse_int_u128(text: &str) -> Option<u128> {
    let body = text
        .strip_suffix("i64")
        .or_else(|| text.strip_suffix('i'))
        .unwrap_or(text);
    let (radix, digits) =
        if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16u128, rest)
        } else if let Some(rest) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            (2u128, rest)
        } else if let Some(rest) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            (8u128, rest)
        } else {
            (10u128, body)
        };
    let mut acc = 0u128;
    let mut has_digit = false;
    for b in digits.bytes() {
        if b == b'_' {
            continue;
        }
        let d = (b as char).to_digit(radix as u32)? as u128;
        has_digit = true;
        acc = acc.checked_mul(radix)?.checked_add(d)?;
    }
    has_digit.then_some(acc)
}

pub(crate) fn parse_int_literal(text: &str) -> Option<i64> {
    i64::try_from(parse_int_u128(text)?).ok()
}

pub(crate) fn parse_neg_int_literal(text: &str) -> Option<i64> {
    let mag = parse_int_u128(text)?;
    if mag == (i64::MAX as u128) + 1 {
        Some(i64::MIN)
    } else {
        i64::try_from(mag).ok()?.checked_neg()
    }
}

pub(crate) fn parse_signed_int_str(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('-') {
        parse_neg_int_literal(rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        parse_int_literal(rest)
    } else {
        parse_int_literal(trimmed)
    }
}

pub(crate) fn parse_f64_literal(text: &str) -> Option<f64> {
    if text.as_bytes().contains(&b'_') {
        let cleaned: String = text.chars().filter(|&c| c != '_').collect();
        cleaned.parse::<f64>().ok().filter(|n| n.is_finite())
    } else {
        text.parse::<f64>().ok().filter(|n| n.is_finite())
    }
}

#[inline]
pub(crate) fn int_f64_eq(i: i64, f: f64) -> bool {
    if !f.is_finite() || f.fract() != 0.0 {
        return false;
    }
    if f < (i64::MIN as f64) || f >= 9_223_372_036_854_775_808.0 {
        return false;
    }
    (f as i128) == (i as i128)
}

#[inline]
pub(crate) fn int_f64_cmp(i: i64, f: f64) -> Option<std::cmp::Ordering> {
    if f.is_nan() {
        return None;
    }
    if f < (i64::MIN as f64) {
        return Some(std::cmp::Ordering::Greater);
    }
    if f >= 9_223_372_036_854_775_808.0 {
        return Some(std::cmp::Ordering::Less);
    }
    let f_trunc = f.trunc() as i128;
    let i128_val = i as i128;
    match i128_val.cmp(&f_trunc) {
        std::cmp::Ordering::Equal => {
            let fract = f.fract();
            if fract > 0.0 {
                Some(std::cmp::Ordering::Less)
            } else if fract < 0.0 {
                Some(std::cmp::Ordering::Greater)
            } else {
                Some(std::cmp::Ordering::Equal)
            }
        }
        ord => Some(ord),
    }
}
