use super::mutation::{mutate_path, read_path};
use super::{
    Binding, Builtin, Closure, Environment, FunctionBody, RecordMap, Result, Runtime, RuntimeError,
    Value, builtins, exact_remainder, format_value_for_display,
};
use crate::{Expr, ExprKind};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

impl<'s> Runtime<'_, 's> {
    pub(super) fn expr(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
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

    pub(super) fn inner(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            // Statement-position await is suspended by the coroutine machine
            // before reaching here; any other position is a stage-A boundary.
            ExprKind::Await(_) => self.error(
                span,
                "await is only supported as a statement of an async task at this stage",
            ),
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
                let mut pairs: Vec<(String, Value<'s>)> = Vec::with_capacity(entries.len());
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
                    if pairs.iter().any(|(k, _)| k == &key) {
                        return self.error(span, "Duplicate record field");
                    }
                    pairs.push((key, self.expr(expression, environment)?));
                }
                Ok(Value::Record(RecordMap::from_unique_entries(pairs)))
            }
            ExprKind::Number(text) => {
                if super::is_int_literal(text) {
                    super::parse_int_literal(text)
                        .map(Value::Int)
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: "Integer literal out of 64-bit signed range".into(),
                        })
                } else {
                    super::parse_f64_literal(text)
                        .map(Value::Number)
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: "Unsupported or non-finite number".into(),
                        })
                }
            }
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
                let (index, steps) = self.resolve_lvalue_path(target, environment)?;
                let mut assigned = if *operator == "=" {
                    self.expr(value, environment)?
                } else {
                    let op = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        "<<=" => "<<",
                        ">>=" => ">>",
                        "&=" => "&",
                        "^=" => "^",
                        _ => return self.error(span, "Unknown assignment operator"),
                    };
                    if steps.is_empty() {
                        self.binary_expression(op, target, value, environment, span)?
                    } else {
                        self.check(span)?;
                        if self.remaining == 0 || self.depth >= self.max_depth {
                            return self.error(span, "Execution limit exceeded");
                        }
                        self.remaining -= 1;
                        self.depth += 1;
                        let result = (|| {
                            let left = read_path(self.module, &self.cells[index].0, &steps)?;
                            let right = self.expr(value, environment)?;
                            self.binary_values(op, left, right, span)
                        })();
                        self.depth -= 1;
                        result?
                    }
                };
                if steps.is_empty() {
                    if let Some(ty) = &self.cells[index].1 {
                        if !ty.accepts(&assigned) {
                            return self.error(span, "Assignment violates variable type");
                        }
                        assigned = assigned.coerce_for_type(ty);
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
                        &self.struct_fields,
                        self.max_collection_items,
                        self.max_string_bytes,
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
            ExprKind::Lambda {
                parameters,
                body,
                is_async,
            } => {
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
                    siblings: Rc::new(Vec::new()),
                    is_async: *is_async,
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
                                    .map(|v| Value::Vector(crate::InlineVec::from(*v)))
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
                let Some(idx) = index.as_usize_index() else {
                    return if matches!(index, Value::Number(_) | Value::Int(_)) {
                        self.error(span, "Invalid collection index")
                    } else {
                        self.error(span, "Index must be an integer")
                    };
                };
                let result = match object {
                    Value::List(values) | Value::Tuple(values) => values.get(idx).cloned(),
                    Value::Vector(values) => values.get(idx).copied().map(Value::Number),
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
                if *operator == "-"
                    && let ExprKind::Number(text) = &value.kind
                    && super::is_int_literal(text)
                {
                    self.check(value.span)?;
                    if self.remaining == 0 || self.depth >= self.max_depth {
                        return self.error(value.span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    return super::parse_neg_int_literal(text)
                        .map(Value::Int)
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: "Integer literal out of 64-bit signed range".into(),
                        });
                }
                match (*operator, self.expr(value, environment)?) {
                    ("-", Value::Int(i)) => match i.checked_neg() {
                        Some(neg) => Ok(Value::Int(neg)),
                        None => self.error(span, "Integer overflow"),
                    },
                    ("+", Value::Int(i)) => Ok(Value::Int(i)),
                    ("~", Value::Int(i)) => Ok(Value::Int(!i)),
                    ("~", Value::Number(n)) => {
                        if let Some(i) = Value::Number(n).as_exact_i64() {
                            Ok(Value::Int(!i))
                        } else {
                            self.error(span, "Bitwise operations require integer operands")
                        }
                    }
                    ("~", _) => self.error(span, "Bitwise operations require integer operands"),
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

    pub(super) fn binary_expression(
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
    pub(super) fn binary_inner(
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
        self.binary_values(operator, left, right, span)
    }

    #[inline(always)]
    pub(super) fn binary_values(
        &mut self,
        operator: &str,
        left: Value<'s>,
        right: Value<'s>,
        span: Span,
    ) -> Result<Value<'s>> {
        if matches!(operator, "==" | "!=") {
            let equal = self.equal(&left, &right, span)?;
            return Ok(Value::Bool(if operator == "==" { equal } else { !equal }));
        }
        if let (Value::Int(a), Value::Int(b)) = (&left, &right) {
            return super::ops::eval_int_binary(operator, *a, *b)
                .map_err(|msg| self.error::<Value<'s>>(span, msg).unwrap_err());
        }
        if matches!(operator, "&" | "^" | "<<" | ">>") {
            if let (Some(a), Some(b)) = (left.as_exact_i64(), right.as_exact_i64()) {
                return super::ops::eval_int_binary(operator, a, b)
                    .map_err(|msg| self.error::<Value<'s>>(span, msg).unwrap_err());
            }
            return self.error(span, "Bitwise operations require integer operands");
        }
        if matches!(
            (&left, &right),
            (Value::Int(_), Value::Number(_)) | (Value::Number(_), Value::Int(_))
        ) {
            return super::ops::eval_binary_values(operator, left, right)
                .map_err(|msg| self.error::<Value<'s>>(span, msg).unwrap_err());
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
                (Value::Angle(a), Value::Int(b), "*") | (Value::Int(b), Value::Angle(a), "*") => {
                    a * (*b as f64)
                }
                (Value::Angle(a), Value::Number(b), "/") => a / b,
                (Value::Angle(a), Value::Int(b), "/") => a / (*b as f64),
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
                (Value::Vector(a), Value::Int(b), "*" | "/") => {
                    let b = *b as f64;
                    a.iter()
                        .map(|a| if operator == "*" { a * b } else { a / b })
                        .collect()
                }
                (Value::Number(a), Value::Vector(b), "*") => b.iter().map(|b| a * b).collect(),
                (Value::Int(a), Value::Vector(b), "*") => {
                    let a = *a as f64;
                    b.iter().map(|b| a * b).collect()
                }
                _ => return self.error(span, "Invalid vector operands or dimensions"),
            };
            return self.vector(components, span);
        }
        self.error(span, "Numeric operands required")
    }

    pub(super) fn vector(&self, values: crate::InlineVec, span: Span) -> Result<Value<'s>> {
        if values.iter().all(|x| x.is_finite()) {
            Ok(Value::Vector(values))
        } else {
            self.error(span, "Non-finite vector result")
        }
    }

    pub(super) fn math(
        &self,
        builtin: Builtin,
        arguments: &[Value<'s>],
        span: Span,
    ) -> Result<Value<'s>> {
        match (builtin, arguments) {
            (Builtin::Polygon, [Value::List(points)]) => {
                self.collection_growth(0, points.len(), span)?;
            }
            (Builtin::Translate, [Value::Polygon(polygon), Value::Vector(offset)])
                if offset.len() == 2 =>
            {
                self.collection_growth(0, polygon.points().len(), span)?;
            }
            (
                Builtin::Rotate,
                [
                    Value::Polygon(polygon),
                    Value::Number(_) | Value::Int(_) | Value::Angle(_),
                ],
            ) => {
                self.collection_growth(0, polygon.points().len(), span)?;
            }
            _ => {}
        }
        match builtins::eval_math_builtin(builtin, arguments) {
            Ok(val) => Ok(val),
            Err(msg) => self.error(span, msg),
        }
    }
}
