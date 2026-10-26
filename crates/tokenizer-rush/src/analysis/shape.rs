use super::Checker;
use crate::{Builtin, Expr, ExprKind, HostFunction, Name, ValueType};
use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    Constructor(String, Option<String>),
    Function(Vec<Option<ValueType>>, Box<Shape>, bool),
    Alternatives(Vec<Shape>),
    Unknown,
    Typed(ValueType),
    List(Vec<Shape>),
    Tuple(Vec<Shape>),
    Record(BTreeMap<String, Shape>),
    // Functions and records with unknown keys have no current ValueType contract.
    Unsupported,
}

impl Shape {
    /// Whether a value of this shape can carry captured cells — closures or
    /// containers whose payload is not statically known to be scalar. Pure
    /// scalar copies never move a cell out of its region.
    pub(super) fn can_carry_cells(&self) -> bool {
        fn type_carries(ty: &ValueType) -> bool {
            match ty {
                ValueType::Function(..)
                | ValueType::User(_)
                | ValueType::HostObject(_)
                | ValueType::Sequence => true,
                ValueType::Option(inner) => type_carries(inner),
                ValueType::Result(ok, err) => type_carries(ok) || type_carries(err),
                ValueType::List(element) => type_carries(element),
                ValueType::Tuple(items) => items.iter().any(type_carries),
                _ => false,
            }
        }
        match self {
            Self::Function(..) | Self::Constructor(..) | Self::Unknown | Self::Unsupported => true,
            Self::Alternatives(items) | Self::List(items) | Self::Tuple(items) => {
                items.iter().any(Self::can_carry_cells)
            }
            Self::Record(fields) => fields.values().any(Self::can_carry_cells),
            Self::Typed(ty) => type_carries(ty),
        }
    }

    pub(super) fn has_contract(&self) -> bool {
        match self {
            Self::Typed(_) => true,
            Self::List(items) | Self::Tuple(items) => items.iter().all(Self::has_contract),
            Self::Record(fields) => fields.values().all(Self::has_contract),
            _ => false,
        }
    }

    pub(super) fn member_paths(&self, prefix: &str, output: &mut BTreeMap<String, Vec<String>>) {
        match self {
            Self::Record(fields) => {
                if !prefix.is_empty() {
                    output.insert(prefix.to_owned(), fields.keys().cloned().collect());
                }
                for (field, shape) in fields {
                    let path = if prefix.is_empty() {
                        field.clone()
                    } else {
                        format!("{prefix}.{field}")
                    };
                    shape.member_paths(&path, output);
                }
            }
            Self::Typed(ValueType::Mesh) if !prefix.is_empty() => {
                output.insert(
                    prefix.to_owned(),
                    vec!["triangles".into(), "vertices".into()],
                );
            }
            Self::Typed(ValueType::Vector(size)) if !prefix.is_empty() => {
                output.insert(
                    prefix.to_owned(),
                    ["x", "y", "z", "w"]
                        .iter()
                        .take(*size)
                        .map(|name| (*name).to_owned())
                        .collect(),
                );
            }
            _ => {}
        }
    }

    pub(super) fn rejects(&self, expected: &ValueType) -> bool {
        match self {
            Self::Alternatives(shapes) => shapes.iter().any(|shape| shape.rejects(expected)),
            Self::Unknown => false,
            Self::Function(parameters, result, suspends) => {
                if let ValueType::Function(expected_parameters, expected_result) = expected {
                    *suspends
                        || parameters.len() != expected_parameters.len()
                        || parameters
                            .iter()
                            .zip(expected_parameters)
                            .any(|(actual, expected)| actual.as_ref() != Some(expected))
                        || result.rejects(expected_result)
                        || matches!(result.as_ref(), Shape::Unknown)
                } else {
                    true
                }
            }
            Self::Unsupported | Self::Record(_) | Self::Constructor(..) => true,
            Self::Typed(actual) => disjoint(actual, expected),
            Self::List(items) => match expected {
                ValueType::List(element) => items.iter().any(|item| item.rejects(element)),
                _ => true,
            },
            Self::Tuple(items) => match expected {
                ValueType::Tuple(types) => {
                    items.len() != types.len()
                        || items.iter().zip(types).any(|(item, ty)| item.rejects(ty))
                }
                _ => true,
            },
        }
    }
}

// True only when the two runtime contracts accept no common value.
pub(super) fn disjoint(actual: &ValueType, expected: &ValueType) -> bool {
    match (actual, expected) {
        (ValueType::Int, ValueType::Number) | (ValueType::Number, ValueType::Int) => false,
        (ValueType::List(_), ValueType::List(_)) | (ValueType::Option(_), ValueType::Option(_)) => {
            false
        }
        (ValueType::Tuple(a), ValueType::Tuple(b)) => {
            a.len() != b.len() || a.iter().zip(b).any(|(a, b)| disjoint(a, b))
        }
        (ValueType::Result(ok_a, err_a), ValueType::Result(ok_b, err_b)) => {
            disjoint(ok_a, ok_b) && disjoint(err_a, err_b)
        }
        _ => actual != expected,
    }
}

pub(super) fn lvalue_root<'a, 's>(expr: &'a Expr<'s>) -> Option<&'a Name<'s>> {
    match &expr.kind {
        ExprKind::Name(name) => Some(name),
        ExprKind::Index { object, .. } | ExprKind::Member { object, .. } => lvalue_root(object),
        _ => None,
    }
}

impl<'s> Checker<'s> {
    pub(super) fn literal_number(expression: &Expr<'_>) -> Option<f64> {
        match &expression.kind {
            ExprKind::Number(text) => {
                if crate::runtime::is_int_literal(text) {
                    crate::runtime::parse_int_literal(text).map(|i| i as f64)
                } else {
                    crate::runtime::parse_f64_literal(text)
                }
            }
            ExprKind::Unary {
                operator: "+",
                value,
            } => Self::literal_number(value),
            ExprKind::Unary {
                operator: "-",
                value,
            } => Self::literal_number(value).map(|n| -n),
            _ => None,
        }
    }

    pub(super) fn literal_string(expression: &Expr<'_>) -> Option<String> {
        let ExprKind::String(text) = &expression.kind else {
            return None;
        };
        crate::string_literal::characters(text)
            .collect::<Result<String, _>>()
            .ok()
    }

    pub(super) fn user_constructor(
        &self,
        expression: &Expr<'s>,
    ) -> Option<(String, Option<String>)> {
        if let Shape::Constructor(name, variant) = self.shape(expression) {
            Some((name, variant))
        } else {
            None
        }
    }

    pub(super) fn struct_fields(&self, shape: &Shape) -> Option<BTreeMap<String, Shape>> {
        let Shape::Typed(ValueType::User(name)) = shape else {
            return None;
        };
        Some(
            self.user_types
                .get(name)?
                .fields
                .as_ref()?
                .iter()
                .map(|(name, ty)| (name.clone(), Shape::Typed(ty.clone())))
                .collect(),
        )
    }

    pub(super) fn known_builtin(&self, expression: &Expr<'s>) -> Option<Builtin> {
        match &expression.kind {
            ExprKind::Name(name) => {
                if let Some(binding) = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                {
                    return self.builtin_aliases.get(&binding.1.start).copied();
                }
                crate::runtime::builtins::lookup_builtin(name.text)
            }
            ExprKind::If {
                then_value,
                else_value,
                ..
            } => {
                let left = self.known_builtin(then_value)?;
                (Some(left) == self.known_builtin(else_value)).then_some(left)
            }
            _ => None,
        }
    }

    pub(super) fn known_arity(&self, expression: &Expr<'s>) -> Option<Vec<RangeInclusive<usize>>> {
        if let Shape::Typed(ValueType::Function(parameters, _)) = self.shape(expression) {
            let n = parameters.len();
            return Some(std::iter::once(n..=n).collect());
        }
        if let Shape::Function(parameters, _, _) = self.shape(expression) {
            let n = parameters.len();
            return Some(std::iter::once(n..=n).collect());
        }
        if let Some((name, variant)) = self.user_constructor(expression) {
            let count = variant
                .as_ref()
                .map_or(1, |variant| self.user_types[&name].variants[variant].len());
            return Some(std::iter::once(count..=count).collect());
        }
        match &expression.kind {
            ExprKind::Lambda { parameters, .. } => {
                Some(std::iter::once(parameters.len()..=parameters.len()).collect())
            }
            ExprKind::Name(name) => {
                if let Some(binding) = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                {
                    return binding.2.clone();
                }
                crate::builtin_catalog()
                    .iter()
                    .find(|(builtin, _)| *builtin == name.text)
                    .map(|(_, builtin)| vec![builtin.arity()])
                    .or_else(|| {
                        self.host_functions.get(name.text).map(|function| {
                            std::iter::once(function.parameters.len()..=function.parameters.len())
                                .collect()
                        })
                    })
            }
            ExprKind::If {
                then_value,
                else_value,
                ..
            } => {
                let mut alternatives = self.known_arity(then_value)?;
                for arity in self.known_arity(else_value)? {
                    if !alternatives.contains(&arity) {
                        alternatives.push(arity);
                    }
                }
                Some(alternatives)
            }
            _ => None,
        }
    }

    pub(super) fn known_host(&self, expression: &Expr<'s>) -> Option<Rc<HostFunction>> {
        if self.host_functions.is_empty() {
            return None;
        }
        let ExprKind::Name(name) = &expression.kind else {
            return None;
        };
        if let Some(binding) = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name.text))
        {
            return binding.3.clone();
        }
        if crate::builtin_catalog()
            .iter()
            .any(|(builtin, _)| *builtin == name.text)
        {
            return None;
        }
        self.host_functions.get(name.text).cloned()
    }

    pub(super) fn known_record_fields(&self, expression: &Expr<'s>) -> Option<Vec<String>> {
        let shape = self.shape(expression);
        if let Some(fields) = self.struct_fields(&shape) {
            return Some(fields.into_keys().collect());
        }
        match shape {
            Shape::Record(fields) => Some(fields.into_keys().collect()),
            _ => None,
        }
    }

    pub(super) fn known_user_parameters(
        &self,
        expression: &Expr<'s>,
    ) -> Option<Vec<Option<ValueType>>> {
        if let Shape::Typed(ValueType::Function(parameters, _)) = self.shape(expression) {
            return Some(parameters.into_iter().map(Some).collect());
        }
        if let Shape::Function(parameters, _, _) = self.shape(expression) {
            return Some(parameters);
        }
        if let Some((name, variant)) = self.user_constructor(expression) {
            return Some(variant.map_or_else(
                || vec![None],
                |variant| {
                    self.user_types[&name].variants[&variant]
                        .iter()
                        .cloned()
                        .map(Some)
                        .collect()
                },
            ));
        }
        let ExprKind::Name(name) = &expression.kind else {
            return None;
        };
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name.text))
            .and_then(|binding| self.function_parameters.get(&binding.1.start))
            .cloned()
    }

    pub(super) fn call_result_shape(&self, callee: &Expr<'s>) -> Shape {
        if let Shape::Typed(ValueType::Function(_, result)) = self.shape(callee) {
            return Shape::Typed(*result);
        }
        if let Shape::Function(_, result, _) = self.shape(callee) {
            return *result;
        }
        if let Some((name, _)) = self.user_constructor(callee) {
            return Shape::Typed(ValueType::User(name));
        }
        if let Some(builtin) = self.known_builtin(callee) {
            let size = match builtin {
                Builtin::Vec2 => Some(2),
                Builtin::Vec3
                | Builtin::Cross
                | Builtin::TransformPoint
                | Builtin::TransformDirection => Some(3),
                Builtin::Vec4 => Some(4),
                _ => None,
            };
            if let Some(size) = size {
                return Shape::Typed(ValueType::Vector(size));
            }
            let ty = match builtin {
                Builtin::Identity
                | Builtin::Translation
                | Builtin::Scaling
                | Builtin::RotationX
                | Builtin::RotationY
                | Builtin::RotationZ
                | Builtin::RotationMatrix => Some(ValueType::Matrix4),
                Builtin::AxisAngle | Builtin::Slerp => Some(ValueType::Quaternion),
                Builtin::Degrees | Builtin::Radians => Some(ValueType::Angle),
                Builtin::Mesh | Builtin::GridMesh | Builtin::Transform => Some(ValueType::Mesh),
                Builtin::Polygon | Builtin::Translate | Builtin::Rotate => Some(ValueType::Polygon),
                Builtin::Trim
                | Builtin::TrimStart
                | Builtin::TrimEnd
                | Builtin::Join
                | Builtin::Replace
                | Builtin::ToLower
                | Builtin::ToUpper => Some(ValueType::String),
                Builtin::StartsWith | Builtin::EndsWith | Builtin::Contains => {
                    Some(ValueType::Bool)
                }
                Builtin::Split => Some(ValueType::List(Box::new(ValueType::String))),
                Builtin::Int
                | Builtin::BitAnd
                | Builtin::BitOr
                | Builtin::BitXor
                | Builtin::BitNot
                | Builtin::Shl
                | Builtin::Shr => Some(ValueType::Int),
                Builtin::Float => Some(ValueType::Number),
                _ => None,
            };
            if let Some(ty) = ty {
                return Shape::Typed(ty);
            }
        }
        if let Some(function) = self.known_host(callee) {
            return Shape::Typed(function.result.clone());
        }
        if let ExprKind::Name(name) = &callee.kind
            && let Some(binding) = self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name.text))
        {
            return self
                .function_shapes
                .get(&binding.1.start)
                .cloned()
                .unwrap_or_else(|| {
                    self.function_results
                        .get(&binding.1.start)
                        .map_or(Shape::Unknown, |ty| Shape::Typed(ty.clone()))
                });
        }
        if let ExprKind::Name(name) = &callee.kind
            && !self
                .scopes
                .iter()
                .rev()
                .any(|scope| scope.contains_key(name.text))
        {
            let dimensions = match name.text {
                "vec2" => Some(2),
                "vec3" => Some(3),
                "vec4" => Some(4),
                _ => None,
            };
            if let Some(dimensions) = dimensions {
                return Shape::Typed(ValueType::Vector(dimensions));
            }
        }
        Shape::Unknown
    }

    pub(super) fn applied_result_shape(&self, callee: &Expr<'s>, supplied: &[Shape]) -> Shape {
        match (self.known_builtin(callee), supplied) {
            (Some(Builtin::Normalize), [shape @ Shape::Typed(ValueType::Vector(_))]) => {
                shape.clone()
            }
            (
                Some(Builtin::Lerp),
                [
                    left @ Shape::Typed(ValueType::Vector(_) | ValueType::Number | ValueType::Int),
                    right,
                    _,
                ],
            ) if left == right => {
                if matches!(left, Shape::Typed(ValueType::Int)) {
                    Shape::Typed(ValueType::Number)
                } else {
                    left.clone()
                }
            }
            _ => self.call_result_shape(callee),
        }
    }

    pub(super) fn shape(&self, expression: &Expr<'s>) -> Shape {
        if let Some(shape) = self
            .expression_shapes
            .get(&(expression.span.start, expression.span.end))
        {
            return shape.clone();
        }
        match &expression.kind {
            ExprKind::Await(inner) => match &inner.kind {
                // `await make()` yields the async function's annotated result;
                // a bare handle or a plain value has no further contract.
                ExprKind::Call { callee, arguments } => self.applied_result_shape(
                    callee,
                    &arguments
                        .iter()
                        .map(|arg| self.shape(arg))
                        .collect::<Vec<_>>(),
                ),
                _ => self.shape(inner),
            },
            ExprKind::Try(value) => match self.shape(value) {
                Shape::Typed(ValueType::Option(ty) | ValueType::Result(ty, _)) => Shape::Typed(*ty),
                _ => Shape::Unknown,
            },
            ExprKind::Name(name) => self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name.text))
                .map_or(Shape::Unknown, |binding| self.binding_shape(binding)),
            ExprKind::Number(text) => {
                if crate::runtime::is_int_literal(text) {
                    Shape::Typed(ValueType::Int)
                } else {
                    Shape::Typed(ValueType::Number)
                }
            }
            ExprKind::String(_) | ExprKind::Interpolate(_) => Shape::Typed(ValueType::String),
            ExprKind::Bool(_) => Shape::Typed(ValueType::Bool),
            ExprKind::Null => Shape::Typed(ValueType::Null),
            ExprKind::List(items) => {
                Shape::List(items.iter().map(|item| self.shape(item)).collect())
            }
            ExprKind::Tuple(items) => {
                Shape::Tuple(items.iter().map(|item| self.shape(item)).collect())
            }
            ExprKind::Map(entries) => {
                let mut fields = BTreeMap::new();
                for (key, value) in entries {
                    let ExprKind::Name(name) = &key.kind else {
                        return Shape::Unsupported;
                    };
                    fields.insert(name.text.to_owned(), self.shape(value));
                }
                Shape::Record(fields)
            }
            ExprKind::Lambda { .. } => Shape::Unsupported,
            ExprKind::Call { callee, arguments } => self.applied_result_shape(
                callee,
                &arguments
                    .iter()
                    .map(|arg| self.shape(arg))
                    .collect::<Vec<_>>(),
            ),
            ExprKind::Index { object, index } => {
                let position = Self::literal_number(index)
                    .filter(|n| {
                        n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < usize::MAX as f64
                    })
                    .map(|n| n as usize);
                match self.shape(object) {
                    Shape::Record(mut fields) => Self::literal_string(index)
                        .and_then(|key| fields.remove(&key))
                        .unwrap_or(Shape::Unknown),
                    Shape::Typed(ValueType::Vector(_)) => Shape::Typed(ValueType::Number),
                    Shape::Typed(ValueType::List(element)) => Shape::Typed(*element),
                    Shape::Typed(ValueType::Tuple(items)) => position
                        .and_then(|i| items.get(i).cloned())
                        .map_or(Shape::Unknown, Shape::Typed),
                    Shape::Tuple(items) | Shape::List(items) => position
                        .and_then(|i| items.get(i).cloned())
                        .unwrap_or(Shape::Unknown),
                    _ => Shape::Unknown,
                }
            }
            ExprKind::Pipeline { input, stages } => {
                let mut result = self.shape(input);
                for stage in stages {
                    let mut supplied = vec![result];
                    let callee = if let ExprKind::Call { callee, arguments } = &stage.kind {
                        supplied.extend(arguments.iter().map(|arg| self.shape(arg)));
                        callee.as_ref()
                    } else {
                        stage
                    };
                    result = self.applied_result_shape(callee, &supplied);
                }
                result
            }
            ExprKind::Member { object, field } => {
                if let Some(fields) = self.struct_fields(&self.shape(object)) {
                    return fields.get(field.text).cloned().unwrap_or(Shape::Unknown);
                }
                if let Shape::Record(fields) = self.shape(object) {
                    return fields.get(field.text).cloned().unwrap_or(Shape::Unknown);
                }
                if let Shape::Typed(ValueType::Mesh) = self.shape(object) {
                    return match field.text {
                        "vertices" => Shape::Typed(ValueType::List(Box::new(ValueType::Vector(3)))),
                        "triangles" => Shape::Typed(ValueType::List(Box::new(ValueType::List(
                            Box::new(ValueType::Number),
                        )))),
                        _ => Shape::Unknown,
                    };
                }
                let axis = match field.text {
                    "x" => Some(0),
                    "y" => Some(1),
                    "z" => Some(2),
                    "w" => Some(3),
                    _ => None,
                };
                if let (Shape::Typed(ValueType::Vector(size)), Some(axis)) =
                    (self.shape(object), axis)
                    && axis < size
                {
                    Shape::Typed(ValueType::Number)
                } else {
                    Shape::Unknown
                }
            }
            ExprKind::Unary {
                operator: "not" | "!",
                ..
            } => Shape::Typed(ValueType::Bool),
            ExprKind::Unary { operator: "~", .. } => Shape::Typed(ValueType::Int),
            ExprKind::Unary {
                operator: "+" | "-",
                value,
            } => {
                let shape = self.shape(value);
                match shape {
                    Shape::Typed(ValueType::Int | ValueType::Number | ValueType::Vector(_)) => {
                        shape
                    }
                    _ => Shape::Unknown,
                }
            }
            ExprKind::Binary {
                operator: "==" | "!=" | "<" | ">" | "<=" | ">=" | "and" | "or" | "&&" | "||",
                ..
            } => Shape::Typed(ValueType::Bool),
            ExprKind::Binary {
                operator: "&" | "^" | "<<" | ">>",
                ..
            } => Shape::Typed(ValueType::Int),
            ExprKind::Binary {
                left,
                right,
                operator,
            } => {
                let left = self.shape(left);
                let right = self.shape(right);
                match (&left, &right, *operator) {
                    (Shape::Typed(ValueType::Matrix4), Shape::Typed(ValueType::Matrix4), "*")
                    | (
                        Shape::Typed(ValueType::Quaternion),
                        Shape::Typed(ValueType::Quaternion),
                        "*",
                    )
                    | (Shape::Typed(ValueType::Angle), Shape::Typed(ValueType::Angle), "+" | "-")
                    | (
                        Shape::Typed(ValueType::Angle),
                        Shape::Typed(ValueType::Number | ValueType::Int),
                        "*" | "/",
                    ) => return left,
                    (
                        Shape::Typed(ValueType::Number | ValueType::Int),
                        Shape::Typed(ValueType::Angle),
                        "*",
                    ) => {
                        return right;
                    }
                    (
                        Shape::Typed(ValueType::Vector(a)),
                        Shape::Typed(ValueType::Vector(b)),
                        "+" | "-",
                    ) if a == b => return left,
                    (
                        Shape::Typed(ValueType::Vector(_)),
                        Shape::Typed(ValueType::Number | ValueType::Int),
                        "*" | "/",
                    ) => return left,
                    (
                        Shape::Typed(ValueType::Number | ValueType::Int),
                        Shape::Typed(ValueType::Vector(_)),
                        "*",
                    ) => {
                        return right;
                    }
                    (Shape::Typed(ValueType::Int), Shape::Typed(ValueType::Int), _) => {
                        return Shape::Typed(ValueType::Int);
                    }
                    (
                        Shape::Typed(ValueType::Number | ValueType::Int),
                        Shape::Typed(ValueType::Number | ValueType::Int),
                        _,
                    ) => {
                        return Shape::Typed(ValueType::Number);
                    }
                    _ => {}
                }
                if right == left && *operator == "+" && left == Shape::Typed(ValueType::String) {
                    left
                } else {
                    Shape::Unknown
                }
            }
            ExprKind::If {
                then_value,
                else_value,
                ..
            } => {
                let left = self.shape(then_value);
                if left == self.shape(else_value) {
                    left
                } else {
                    Shape::Alternatives(vec![left, self.shape(else_value)])
                }
            }
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                if *operator == "=" {
                    self.shape(value)
                } else {
                    let operation = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        "<<=" => "<<",
                        ">>=" => ">>",
                        "&=" => "&",
                        "^=" => "^",
                        _ => "",
                    };
                    self.shape(&Expr {
                        span: expression.span,
                        kind: ExprKind::Binary {
                            operator: operation,
                            left: target.clone(),
                            right: value.clone(),
                        },
                    })
                }
            }
            _ => Shape::Unknown,
        }
    }

    pub(super) fn known_non_callable(&self, value: &Expr<'s>) -> bool {
        fn data(shape: &Shape) -> bool {
            match shape {
                Shape::Alternatives(shapes) => shapes.iter().any(data),
                Shape::Typed(ValueType::Function(..)) => false,
                Shape::Typed(_) | Shape::List(_) | Shape::Tuple(_) | Shape::Record(_) => true,
                _ => false,
            }
        }
        matches!(value.kind, ExprKind::Map(_)) || data(&self.shape(value))
    }
}
