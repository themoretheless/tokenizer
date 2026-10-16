use super::shape::{disjoint, lvalue_root};
use super::{Checker, MemberCompletion, NameReference, Shape};
use crate::{Builtin, Expr, ExprKind, InterpolationPart, ValueType};
use std::collections::HashMap;
use themoretheless_tokenizer_core::Span;

impl<'s> Checker<'s> {
    pub(super) fn check_index(&mut self, object: &Expr<'s>, index: &Expr<'s>) {
        if let Some(fields) = self.known_record_fields(object) {
            if self.shape(index).rejects(&ValueType::String) {
                self.error(index.span, "index-type", "Record index must be a string");
            } else if let Some(key) = Self::literal_string(index)
                && !fields.contains(&key)
            {
                self.error(
                    index.span,
                    "unknown-record-field",
                    "Unknown field in record",
                );
            }
            return;
        }
        let length = match self.shape(object) {
            Shape::Typed(ValueType::Vector(size)) => Some(size),
            Shape::Tuple(items) | Shape::List(items) => Some(items.len()),
            Shape::Typed(ValueType::Tuple(items)) => Some(items.len()),
            Shape::Typed(ValueType::List(_)) => None,
            Shape::Typed(_) => {
                self.error(
                    object.span,
                    "index-object",
                    "Value does not support indexing",
                );
                return;
            }
            _ => return,
        };
        if self.shape(index).rejects(&ValueType::Number) {
            self.error(
                index.span,
                "index-type",
                "Collection index must be an integer",
            );
        } else if let Some(number) = Self::literal_number(index)
            && (!number.is_finite()
                || number < 0.0
                || number.fract() != 0.0
                || number >= usize::MAX as f64
                || length.is_some_and(|size| number >= size as f64))
        {
            self.error(
                index.span,
                "index-bounds",
                "Invalid or out-of-bounds collection index",
            );
        }
    }

    pub(super) fn check_math_arguments(&mut self, builtin: Builtin, supplied: &[(Span, Shape)]) {
        let signature: &[ValueType] = match builtin {
            Builtin::Translation | Builtin::Scaling => &[ValueType::Vector(3)],
            Builtin::TransformPoint | Builtin::TransformDirection => {
                &[ValueType::Matrix4, ValueType::Vector(3)]
            }
            Builtin::RotationMatrix => &[ValueType::Quaternion],
            Builtin::Slerp => &[
                ValueType::Quaternion,
                ValueType::Quaternion,
                ValueType::Number,
            ],
            Builtin::Transform => &[ValueType::Mesh, ValueType::Matrix4],
            Builtin::Translate => &[ValueType::Polygon, ValueType::Vector(2)],
            Builtin::Rotate => &[ValueType::Polygon],
            Builtin::AxisAngle => &[ValueType::Vector(3)],
            Builtin::Degrees | Builtin::Radians => &[ValueType::Number],
            _ => &[],
        };
        for ((span, shape), expected) in supplied.iter().zip(signature) {
            if shape.rejects(expected) {
                self.error(
                    *span,
                    "argument-type",
                    "Transformation argument does not match operation",
                );
            }
        }
        let angle_position = match builtin {
            Builtin::RotationX | Builtin::RotationY | Builtin::RotationZ => Some(0),
            Builtin::AxisAngle | Builtin::Rotate => Some(1),
            _ => None,
        };
        if let Some((span, shape)) = angle_position.and_then(|index| supplied.get(index))
            && shape.rejects(&ValueType::Number)
            && shape.rejects(&ValueType::Angle)
        {
            self.error(
                *span,
                "argument-type",
                "Rotation angle must be a number or angle",
            );
        }
        let vector_count = match builtin {
            Builtin::Normalize | Builtin::Length => 1,
            Builtin::Cross | Builtin::Dot => 2,
            _ => 0,
        };
        for (span, shape) in supplied.iter().take(vector_count) {
            let rejected = if builtin == Builtin::Cross {
                shape.rejects(&ValueType::Vector(3))
            } else {
                !matches!(shape, Shape::Unknown | Shape::Typed(ValueType::Vector(_)))
            };
            if rejected {
                self.error(
                    *span,
                    "argument-type",
                    "Mathematical argument requires a compatible vector",
                );
            }
        }
        if builtin == Builtin::Lerp {
            for (span, shape) in supplied.iter().take(2) {
                if !matches!(
                    shape,
                    Shape::Unknown
                        | Shape::Typed(ValueType::Number | ValueType::Int | ValueType::Vector(_))
                ) {
                    self.error(*span, "argument-type", "lerp requires numbers or vectors");
                }
            }
            if let Some((span, shape)) = supplied.get(2)
                && shape.rejects(&ValueType::Number)
            {
                self.error(*span, "argument-type", "lerp factor must be a number");
            }
        }
        if matches!(builtin, Builtin::Dot | Builtin::Lerp)
            && let [(_, left), (span, right), ..] = supplied
        {
            match (left, right) {
                (Shape::Typed(ValueType::Vector(a)), Shape::Typed(ValueType::Vector(b)))
                    if a != b =>
                {
                    self.error(*span, "vector-dimensions", "Vector dimensions must match");
                }
                (
                    Shape::Typed(ValueType::Vector(_)),
                    Shape::Typed(ValueType::Number | ValueType::Int),
                )
                | (
                    Shape::Typed(ValueType::Number | ValueType::Int),
                    Shape::Typed(ValueType::Vector(_)),
                ) if builtin == Builtin::Lerp => {
                    self.error(
                        *span,
                        "argument-type",
                        "lerp endpoints must have matching types",
                    );
                }
                _ => (),
            }
        }
    }

    pub(super) fn call(
        &mut self,
        callee: &Expr<'s>,
        arguments: &[Expr<'s>],
        implicit: Option<(Span, Shape)>,
    ) -> Shape {
        for argument in arguments {
            self.expr(argument);
        }
        if self.calls && self.known_suspending(callee) {
            self.error(
                callee.span,
                "coroutine-call",
                "Suspending functions require spawn_coroutine instead of a synchronous call",
            );
        }
        if self.calls
            && let Some(builtin) = self.known_builtin(callee)
        {
            let supplied = implicit
                .iter()
                .cloned()
                .chain(arguments.iter().map(|arg| (arg.span, self.shape(arg))))
                .collect::<Vec<_>>();
            self.check_math_arguments(builtin, &supplied);
        }
        let result_shape = self.applied_result_shape(
            callee,
            &implicit
                .iter()
                .map(|(_, shape)| shape.clone())
                .chain(arguments.iter().map(|arg| self.shape(arg)))
                .collect::<Vec<_>>(),
        );
        if self.calls
            && let Some((name, None)) = self.user_constructor(callee)
        {
            if self.collect_members
                && let Some(Expr {
                    kind: ExprKind::Map(fields),
                    ..
                }) = arguments.first()
                && let Some(definition) = self.user_types.get(&name)
            {
                for (key, _) in fields {
                    if let ExprKind::Name(field) = &key.kind
                        && let Some(span) = definition.field_spans.get(field.text)
                    {
                        self.references.push(NameReference {
                            usage: field.span,
                            definition: Some(*span),
                            definition_module: definition
                                .module
                                .clone()
                                .filter(|m| self.module.as_ref() != Some(m)),
                        });
                    }
                }
            }

            let supplied = implicit
                .as_ref()
                .map(|(_, shape)| shape.clone())
                .or_else(|| arguments.first().map(|arg| self.shape(arg)));
            if let Some(shape) = supplied {
                if let Shape::Record(fields) = shape {
                    let expected = self.user_types[&name].fields.clone().unwrap_or_default();
                    if fields.keys().ne(expected.keys()) {
                        self.error(
                            callee.span,
                            "struct-fields",
                            "Struct fields must exactly match declaration",
                        );
                    }
                    for (field, ty) in expected {
                        if fields.get(&field).is_some_and(|shape| shape.rejects(&ty)) {
                            self.error(
                                callee.span,
                                "struct-field-type",
                                "Struct field type does not match declaration",
                            );
                        }
                    }
                } else if !matches!(shape, Shape::Unknown) {
                    self.error(
                        callee.span,
                        "struct-fields",
                        "Struct constructor requires a record",
                    );
                }
            }
        }
        if self.calls && self.known_non_callable(callee) {
            self.error(
                callee.span,
                "not-callable",
                "Value cannot be called as a function",
            );
        }
        if self.calls
            && let Some(builtin) = self.known_builtin(callee)
        {
            for &(position, expected) in builtin.callback_arities() {
                if let Some(index) = position.checked_sub(usize::from(implicit.is_some()))
                    && let Some(callback) = arguments.get(index)
                    && self.known_suspending(callback)
                {
                    self.error(
                        callback.span,
                        "coroutine-call",
                        "Synchronous callbacks cannot suspend",
                    );
                }

                if let Some(index) = position.checked_sub(usize::from(implicit.is_some()))
                    && let Some(callback) = arguments.get(index)
                    && self.known_non_callable(callback)
                {
                    self.error(callback.span, "callback-type", "Callback must be callable");
                }
                if let Some(index) = position.checked_sub(usize::from(implicit.is_some()))
                    && let Some(callback) = arguments.get(index)
                    && let Some(arities) = self.known_arity(callback)
                    && !arities.iter().any(|arity| arity.contains(&expected))
                {
                    self.error(
                        callback.span,
                        "callback-argument-count",
                        "Callback argument count does not match operation",
                    );
                }
            }
        }
        if self.calls
            && let Some(arity) = self.known_arity(callee)
            && !arity
                .iter()
                .any(|range| range.contains(&(arguments.len() + usize::from(implicit.is_some()))))
        {
            self.error(
                callee.span,
                "argument-count",
                "Argument count does not match function signature",
            );
        }
        let user_parameters = self.known_user_parameters(callee);
        if self.calls
            && let Some(parameters) = user_parameters
        {
            let supplied = implicit
                .iter()
                .cloned()
                .chain(
                    arguments
                        .iter()
                        .map(|argument| (argument.span, self.shape(argument))),
                )
                .collect::<Vec<_>>();
            for ((span, shape), expected) in supplied.iter().zip(parameters) {
                if let Some(expected) = expected
                    && shape.rejects(&expected)
                {
                    self.error(
                        *span,
                        "argument-type",
                        "Argument type does not match function annotation",
                    );
                }
            }
        }
        let host = self.known_host(callee);
        if self.calls
            && let Some(host) = &host
        {
            let supplied = implicit
                .into_iter()
                .chain(
                    arguments
                        .iter()
                        .map(|argument| (argument.span, self.shape(argument))),
                )
                .collect::<Vec<_>>();
            for ((span, shape), expected) in supplied.iter().zip(&host.parameters) {
                if shape.rejects(expected) {
                    self.error(
                        *span,
                        "argument-type",
                        "Argument type does not match registered host signature",
                    );
                }
            }
        }
        self.expr(callee);
        result_shape
    }

    pub(super) fn condition(&mut self, expression: &Expr<'s>) {
        if self.calls && self.shape(expression).rejects(&ValueType::Bool) {
            self.error(
                expression.span,
                "condition-type",
                "Condition must be a boolean",
            );
        }
        self.expr(expression);
    }

    pub(super) fn expr(&mut self, expr: &Expr<'s>) {
        match &expr.kind {
            ExprKind::Try(value) => {
                if self.calls {
                    let operand = self.shape(value);
                    let result = self.return_types.last().cloned().flatten();
                    match (&operand, result.as_ref()) {
                        (Shape::Typed(ValueType::Option(_)), Some(ValueType::Option(_))) => {}
                        (
                            Shape::Typed(ValueType::Result(_, error)),
                            Some(ValueType::Result(_, expected)),
                        ) if !disjoint(error, expected) => {}
                        (Shape::Unknown, Some(ValueType::Option(_) | ValueType::Result(_, _))) => {}
                        _ => self.error(
                            expr.span,
                            "try-type",
                            "? requires Option or Result compatible with the enclosing function return type",
                        ),
                    }
                }
                self.expr(value);
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                self.condition(condition);
                self.expr(then_value);
                self.expr(else_value);
            }
            ExprKind::Lambda { parameters, body } => {
                self.return_types.push(None);
                self.inferred_returns.push(Vec::new());
                self.scopes.push(HashMap::new());
                self.scope_spans.push(body.span);
                for parameter in parameters {
                    self.pattern(parameter, body.span.start);
                }
                self.expr(body);
                self.return_types.pop();
                self.inferred_returns.pop();
                self.scopes.pop();
                self.scope_spans.pop();
            }
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                if self.calls
                    && *operator == "="
                    && let ExprKind::Name(name) = &target.kind
                    && let Some(expected) = self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                        .and_then(|binding| self.binding_types.get(&binding.1.start))
                    && self.shape(value).rejects(expected)
                {
                    self.error(
                        value.span,
                        "assignment-type",
                        "Assigned value type does not match binding type",
                    );
                }
                if let ExprKind::Name(name) = &target.kind
                    && self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                        .map(|binding| binding.0)
                        == Some(true)
                {
                    self.error(
                        name.span,
                        "immutable-binding",
                        "Cannot assign to an immutable binding",
                    );
                }
                if *operator != "=" {
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
                    let binary = Expr {
                        span: expr.span,
                        kind: ExprKind::Binary {
                            operator: operation,
                            left: target.clone(),
                            right: value.clone(),
                        },
                    };
                    if self.calls
                        && let ExprKind::Name(name) = &target.kind
                        && let Some(expected) = self
                            .scopes
                            .iter()
                            .rev()
                            .find_map(|scope| scope.get(name.text))
                            .and_then(|binding| self.binding_types.get(&binding.1.start))
                        && self.shape(&binary).rejects(expected)
                    {
                        self.error(
                            expr.span,
                            "assignment-type",
                            "Compound assignment changes binding type",
                        );
                    }
                    self.expr(&binary);
                } else {
                    // Plain reassignment revives a moved binding's cell.
                    if let ExprKind::Name(name) = &target.kind {
                        let id = self
                            .scopes
                            .iter()
                            .rev()
                            .find_map(|scope| scope.get(name.text))
                            .map(|binding| binding.1.start);
                        if let Some(id) = id {
                            self.moved.remove(&id);
                        }
                    }
                    self.expr(target);
                    self.expr(value);
                }
                // Compound assignment writes a freshly computed value into an
                // existing outer cell; no region data or cell can flow out.
                if self.region_depth > 0
                    && *operator == "="
                    && let Some(name) = lvalue_root(target)
                    && let Some(binding) = self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                {
                    let target_level = self
                        .binding_regions
                        .get(&binding.1.start)
                        .copied()
                        .unwrap_or(0);
                    if self.region_depth > target_level
                        && !self.is_promote_call(value)
                        && !self.is_move_call(value)
                    {
                        self.check_escape(
                            value,
                            target_level,
                            "Assigned value may capture cells allocated in an inner region; they will be promoted to the target binding's region",
                        );
                    }
                }
            }
            ExprKind::Unary {
                operator: "not" | "!",
                value,
            } => self.condition(value),
            ExprKind::Unary {
                operator: "~",
                value,
            } => {
                let shape = self.shape(value);
                if self.calls && shape.rejects(&ValueType::Int) {
                    self.error(
                        value.span,
                        "unary-operand",
                        "Bitwise not requires an integer",
                    );
                }
                self.expr(value);
            }
            ExprKind::Unary {
                operator: "+" | "-",
                value,
            } => {
                let shape = self.shape(value);
                if self.calls
                    && [
                        ValueType::Number,
                        ValueType::Int,
                        ValueType::Vector(2),
                        ValueType::Vector(3),
                        ValueType::Vector(4),
                    ]
                    .iter()
                    .all(|ty| shape.rejects(ty))
                {
                    self.error(
                        value.span,
                        "unary-operand",
                        "Unary sign requires a number or vector",
                    );
                }
                self.expr(value);
            }
            ExprKind::Unary { value, .. } => self.expr(value),
            ExprKind::Binary {
                left,
                right,
                operator: "and" | "or" | "&&" | "||",
            } => {
                self.condition(left);
                self.condition(right);
            }
            ExprKind::Binary {
                left,
                right,
                operator,
            } => {
                if self.calls && matches!(*operator, "<" | ">" | "<=" | ">=") {
                    for operand in [left, right] {
                        if self.shape(operand).rejects(&ValueType::Number) {
                            self.error(
                                operand.span,
                                "comparison-operand",
                                "Ordered comparison requires a number",
                            );
                        }
                    }
                }
                if self.calls && matches!(*operator, "&" | "^" | "<<" | ">>") {
                    for operand in [left, right] {
                        if self.shape(operand).rejects(&ValueType::Int) {
                            self.error(
                                operand.span,
                                "bitwise-operand",
                                "Bitwise operation requires integers",
                            );
                        }
                    }
                }
                if self.calls && matches!(*operator, "+" | "-" | "*" | "/" | "%" | "**") {
                    let a = self.shape(left);
                    let b = self.shape(right);
                    let specialized = |shape: &Shape| {
                        matches!(
                            shape,
                            Shape::Typed(
                                ValueType::Vector(_)
                                    | ValueType::Matrix4
                                    | ValueType::Quaternion
                                    | ValueType::Angle
                            )
                        )
                    };
                    if !specialized(&a) && !specialized(&b) {
                        let valid = !a.rejects(&ValueType::Number)
                            && !b.rejects(&ValueType::Number)
                            || (*operator == "+"
                                && !a.rejects(&ValueType::String)
                                && !b.rejects(&ValueType::String));
                        if !valid {
                            self.error(
                                expr.span,
                                "arithmetic-operands",
                                "Arithmetic requires numbers or compatible operands",
                            );
                        }
                    }
                    if let (Shape::Typed(a), Shape::Typed(b)) = (&a, &b)
                        && (matches!(a, ValueType::Vector(_)) || matches!(b, ValueType::Vector(_)))
                    {
                        let valid = match (a, b, *operator) {
                            (ValueType::Vector(a), ValueType::Vector(b), "+" | "-") => a == b,
                            (ValueType::Vector(_), ValueType::Number | ValueType::Int, "*" | "/")
                            | (ValueType::Number | ValueType::Int, ValueType::Vector(_), "*") => {
                                true
                            }
                            _ => false,
                        };
                        if !valid {
                            self.error(
                                expr.span,
                                "vector-operands",
                                "Invalid vector operands or dimensions",
                            );
                        }
                    }
                    if let (Shape::Typed(a), Shape::Typed(b)) = (&a, &b)
                        && (matches!(
                            a,
                            ValueType::Matrix4 | ValueType::Quaternion | ValueType::Angle
                        ) || matches!(
                            b,
                            ValueType::Matrix4 | ValueType::Quaternion | ValueType::Angle
                        ))
                        && !matches!(a, ValueType::Vector(_))
                        && !matches!(b, ValueType::Vector(_))
                    {
                        let valid = matches!(
                            (a, b, *operator),
                            (ValueType::Matrix4, ValueType::Matrix4, "*")
                                | (ValueType::Quaternion, ValueType::Quaternion, "*")
                                | (ValueType::Angle, ValueType::Angle, "+" | "-")
                                | (ValueType::Angle, ValueType::Number | ValueType::Int, "*" | "/")
                                | (ValueType::Number | ValueType::Int, ValueType::Angle, "*")
                        );
                        if !valid {
                            self.error(
                                expr.span,
                                "math-operands",
                                "Invalid matrix, quaternion or angle operands",
                            );
                        }
                    }
                }
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Call { callee, arguments } => {
                if matches!(&callee.kind, ExprKind::Name(name) if name.text == "move")
                    && !self.scopes.iter().any(|scope| scope.contains_key("move"))
                {
                    self.move_call(expr.span, arguments);
                } else {
                    self.call(callee, arguments, None);
                }
            }
            ExprKind::Member { object, field } => {
                self.member_reference(object, field);
                let check_member = self.calls && !field.text.is_empty();
                if check_member {
                    let unsupported = match self.shape(object) {
                        Shape::Typed(ty) => !matches!(
                            ty,
                            ValueType::Vector(_) | ValueType::Mesh | ValueType::User(_)
                        ),
                        Shape::List(_) | Shape::Tuple(_) => true,
                        _ => false,
                    };
                    if unsupported {
                        self.error(
                            field.span,
                            "member-object",
                            "Value does not support member access",
                        );
                    }
                }
                if self.collect_members {
                    let object_shape = self.shape(object);
                    let members = if let Some(fields) = self.struct_fields(&object_shape) {
                        fields.into_keys().collect()
                    } else {
                        match object_shape {
                            Shape::Record(fields) => fields.into_keys().collect(),
                            Shape::Typed(ValueType::Vector(size)) => ["x", "y", "z", "w"]
                                .iter()
                                .take(size)
                                .map(|name| (*name).to_owned())
                                .collect(),
                            Shape::Typed(ValueType::Mesh) => {
                                vec!["triangles".into(), "vertices".into()]
                            }
                            _ => Vec::new(),
                        }
                    };
                    self.member_completions.push(MemberCompletion {
                        name_span: field.span,
                        members,
                    });
                }
                if check_member
                    && matches!(self.shape(object), Shape::Typed(ValueType::Mesh))
                    && !matches!(field.text, "vertices" | "triangles")
                {
                    self.error(field.span, "mesh-field", "Unknown mesh field");
                }
                if check_member
                    && let Some(fields) = self.known_record_fields(object)
                    && !fields.iter().any(|name| name == field.text)
                {
                    self.error(
                        field.span,
                        "unknown-record-field",
                        "Unknown field in record",
                    );
                }
                if check_member && let Shape::Typed(ValueType::Vector(size)) = self.shape(object) {
                    let axis = match field.text {
                        "x" => Some(0),
                        "y" => Some(1),
                        "z" => Some(2),
                        "w" => Some(3),
                        _ => None,
                    };
                    if axis.is_none_or(|axis| axis >= size) {
                        self.error(
                            field.span,
                            "vector-component",
                            "Unknown or out-of-bounds vector component",
                        );
                    }
                }
                self.expr(object);
            }
            ExprKind::Index { object, index } => {
                if self.calls {
                    self.check_index(object, index);
                }
                self.expr(object);
                self.expr(index);
            }
            ExprKind::List(values) | ExprKind::Tuple(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    if !matches!(key.kind, ExprKind::Name(_)) {
                        self.expr(key);
                    }
                    self.expr(value);
                }
            }
            ExprKind::Pipeline { input, stages } => {
                let shape = if self.calls {
                    self.shape(input)
                } else {
                    Shape::Unknown
                };
                let mut supplied = (input.span, shape);
                self.expr(input);
                for stage in stages {
                    let result = match &stage.kind {
                        ExprKind::Call { callee, arguments } => {
                            self.call(callee, arguments, Some(supplied))
                        }
                        _ => self.call(stage, &[], Some(supplied)),
                    };
                    supplied = (stage.span, result);
                }
            }
            ExprKind::Match { value, arms } => {
                self.expr(value);
                self.check_exhaustiveness(value, arms);
                let shape = self.shape(value);
                let mut results = Vec::new();
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.scope_spans.push(arm.span);
                    self.match_bindings(&arm.pattern, arm.pattern.span.end);
                    self.match_pattern_types(&arm.pattern, &shape);
                    if let Some(guard) = &arm.guard {
                        self.condition(guard);
                    }
                    self.expr(&arm.value);
                    results.push(self.shape(&arm.value));
                    self.scopes.pop();
                    self.scope_spans.pop();
                }
                let result = if results.iter().all(|shape| Some(shape) == results.first()) {
                    results.first().cloned().unwrap_or(Shape::Unknown)
                } else {
                    Shape::Alternatives(results)
                };
                self.expression_shapes
                    .insert((expr.span.start, expr.span.end), result);
            }
            ExprKind::Name(name) => {
                let definition = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                    .map(|binding| binding.1);
                if let Some(definition) = definition
                    && self.moved.contains_key(&definition.start)
                {
                    self.error(
                        name.span,
                        "moved-value",
                        "Binding was moved out of its cell; assign it again before reading",
                    );
                }
                self.references.push(NameReference {
                    definition_module: None,
                    usage: name.span,
                    definition,
                });
            }
            ExprKind::Interpolate(parts) => {
                for part in parts {
                    if let InterpolationPart::Expr(sub) = part {
                        self.expr(sub);
                    }
                }
            }
            ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Bool(_)
            | ExprKind::Null
            | ExprKind::Error => {}
        }
    }
}
