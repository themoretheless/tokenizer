//! Shared unary, binary, comparison, and formatting operations across AST, Compiler, and Bytecode VM.
use crate::Value;
use std::fmt::Write as _;

/// Integer operands within the exact f64 range avoid libm fmod.
/// The remainder has the dividend's sign, including zero; other operands use fmod.
#[inline]
pub(crate) fn exact_remainder(a: f64, b: f64) -> f64 {
    const EXACT_INTEGER: f64 = 9_007_199_254_740_991.;
    if a.abs() <= EXACT_INTEGER
        && b.abs() <= EXACT_INTEGER
        && b != 0.
        && a.trunc() == a
        && b.trunc() == b
    {
        ((a as i64 % b as i64) as f64).copysign(a)
    } else {
        a % b
    }
}

/// Truthiness semantics for conditionals and short-circuit boolean expressions.
#[inline]
pub(crate) fn is_truthy(value: &Value<'_>) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::List(l) => !l.is_empty(),
        _ => true,
    }
}

/// Returns true if a value is a function or lazy sequence that cannot be compared for equality.
#[inline]
pub(crate) fn is_non_comparable(value: &Value<'_>) -> bool {
    matches!(
        value,
        Value::BytecodeSequence(_)
            | Value::BytecodeIterator(_)
            | Value::Sequence(_)
            | Value::Function(_)
            | Value::BytecodeFunction(_)
            | Value::Builtin(_)
            | Value::Host(_)
    )
}

#[inline]
pub(crate) fn check_vector_finite(values: Vec<f64>) -> Result<Value<'static>, &'static str> {
    if values.iter().all(|x| x.is_finite()) {
        Ok(Value::Vector(values))
    } else {
        Err("Non-finite vector result")
    }
}

/// Shared unary operator evaluation.
#[inline]
pub(crate) fn eval_unary_value<'s>(
    operator: &str,
    value: Value<'s>,
    strict_bool_not: bool,
) -> Result<Value<'s>, &'static str> {
    match (operator, value) {
        ("-", Value::Number(n)) => {
            let res = -n;
            if res.is_finite() {
                Ok(Value::Number(res))
            } else {
                Err("Non-finite arithmetic result")
            }
        }
        ("+", Value::Number(n)) => {
            if n.is_finite() {
                Ok(Value::Number(n))
            } else {
                Err("Non-finite arithmetic result")
            }
        }
        ("-", Value::Vector(mut values)) => {
            for v in &mut values {
                *v = -*v;
            }
            if values.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(values))
            } else {
                Err("Non-finite vector result")
            }
        }
        ("+", Value::Vector(values)) => {
            if values.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(values))
            } else {
                Err("Non-finite vector result")
            }
        }
        ("!" | "not", Value::Bool(b)) => Ok(Value::Bool(!b)),
        ("!" | "not", other) if !strict_bool_not => Ok(Value::Bool(!is_truthy(&other))),
        _ => Err("Invalid unary operand"),
    }
}

/// Shared binary arithmetic, comparison, vector/matrix/quaternion/angle/collection evaluation.
#[inline]
pub(crate) fn eval_binary_values<'s>(
    operator: &str,
    left: Value<'s>,
    right: Value<'s>,
) -> Result<Value<'s>, &'static str> {
    if matches!(operator, "==" | "!=") {
        if is_non_comparable(&left) || is_non_comparable(&right) {
            return Err("Functions and lazy sequences cannot be compared");
        }
        return Ok(Value::Bool(if operator == "==" {
            left == right
        } else {
            left != right
        }));
    }

    // Fast path for Number op Number
    if let (Value::Number(a), Value::Number(b)) = (&left, &right) {
        let (a, b) = (*a, *b);
        let number = match operator {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => {
                if b == 0.0 {
                    return Err("Division by zero");
                }
                a / b
            }
            "%" => {
                if b == 0.0 {
                    return Err("Division by zero");
                }
                exact_remainder(a, b)
            }
            "**" => a.powf(b),
            "<" => return Ok(Value::Bool(a < b)),
            ">" => return Ok(Value::Bool(a > b)),
            "<=" => return Ok(Value::Bool(a <= b)),
            ">=" => return Ok(Value::Bool(a >= b)),
            _ => return Err("Unsupported binary operator"),
        };
        return if number.is_finite() {
            Ok(Value::Number(number))
        } else {
            Err("Non-finite arithmetic result")
        };
    }

    match (left, right, operator) {
        (Value::String(mut a), Value::String(b), "+") => {
            a.push_str(&b);
            Ok(Value::String(a))
        }
        (Value::List(mut a), Value::List(b), "+") => {
            a.extend(b);
            Ok(Value::List(a))
        }
        (Value::Angle(a), Value::Angle(b), "+") => {
            let r = a + b;
            if r.is_finite() {
                Ok(Value::Angle(r))
            } else {
                Err("Non-finite angle")
            }
        }
        (Value::Angle(a), Value::Angle(b), "-") => {
            let r = a - b;
            if r.is_finite() {
                Ok(Value::Angle(r))
            } else {
                Err("Non-finite angle")
            }
        }
        (Value::Angle(a), Value::Number(b), "*") | (Value::Number(b), Value::Angle(a), "*") => {
            let r = a * b;
            if r.is_finite() {
                Ok(Value::Angle(r))
            } else {
                Err("Non-finite angle")
            }
        }
        (Value::Angle(a), Value::Number(b), "/") => {
            let r = a / b;
            if r.is_finite() {
                Ok(Value::Angle(r))
            } else {
                Err("Non-finite angle")
            }
        }
        (Value::Angle(_), _, _) | (_, Value::Angle(_), _) => Err("Invalid angle operands"),
        (Value::Quaternion(a), Value::Quaternion(b), "*") => {
            Ok(Value::Quaternion(Box::new(a.compose(&b))))
        }
        (Value::Matrix(a), Value::Matrix(b), "*") => a
            .multiply(&b)
            .map(|matrix| Value::Matrix(Box::new(matrix)))
            .ok_or("Matrix multiplication overflow"),
        (Value::Vector(mut a), Value::Vector(b), "+") if a.len() == b.len() => {
            for (x, y) in a.iter_mut().zip(&b) {
                *x += *y;
            }
            if a.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(a))
            } else {
                Err("Non-finite vector result")
            }
        }
        (Value::Vector(mut a), Value::Vector(b), "-") if a.len() == b.len() => {
            for (x, y) in a.iter_mut().zip(&b) {
                *x -= *y;
            }
            if a.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(a))
            } else {
                Err("Non-finite vector result")
            }
        }
        (Value::Vector(mut a), Value::Number(b), "*") => {
            for x in &mut a {
                *x *= b;
            }
            if a.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(a))
            } else {
                Err("Non-finite vector result")
            }
        }
        (Value::Vector(mut a), Value::Number(b), "/") => {
            for x in &mut a {
                *x /= b;
            }
            if a.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(a))
            } else {
                Err("Non-finite vector result")
            }
        }
        (Value::Number(a), Value::Vector(mut b), "*") => {
            for x in &mut b {
                *x *= a;
            }
            if b.iter().all(|x| x.is_finite()) {
                Ok(Value::Vector(b))
            } else {
                Err("Non-finite vector result")
            }
        }
        (Value::Vector(_), _, _) | (_, Value::Vector(_), _) => {
            Err("Invalid vector operands or dimensions")
        }
        _ => Err("Numeric operands required"),
    }
}

pub(crate) fn format_value_for_display(value: &Value<'_>, out: &mut String, depth: usize) {
    if depth > 16 {
        out.push_str("...");
        return;
    }
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) {
                let _ = write!(out, "{:.0}", n);
            } else {
                let _ = write!(out, "{}", n);
            }
        }
        Value::Angle(a) => {
            let _ = write!(out, "{} rad", a);
        }
        Value::String(s) => {
            if depth == 0 {
                out.push_str(s);
            } else {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        _ => out.push(c),
                    }
                }
                out.push('"');
            }
        }
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_for_display(item, out, depth + 1);
            }
            out.push(']');
        }
        Value::Tuple(items) => {
            out.push('(');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_for_display(item, out, depth + 1);
            }
            if items.len() == 1 {
                out.push(',');
            }
            out.push(')');
        }
        Value::Record(fields) => {
            out.push_str("{ ");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(k);
                out.push_str(": ");
                format_value_for_display(v, out, depth + 1);
            }
            out.push_str(" }");
        }
        Value::Vector(comps) => {
            match comps.len() {
                2 => out.push_str("vec2("),
                3 => out.push_str("vec3("),
                4 => out.push_str("vec4("),
                _ => out.push_str("vec("),
            }
            for (i, c) in comps.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "{}", c);
            }
            out.push(')');
        }
        Value::Variant(tag, payload) => {
            out.push_str(tag);
            if !payload.is_empty() {
                out.push('(');
                for (i, item) in payload.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    format_value_for_display(item, out, depth + 1);
                }
                out.push(')');
            }
        }
        Value::UserData(data) => {
            out.push_str(&data.type_name);
            if let Some(var) = &data.variant {
                out.push('.');
                out.push_str(var);
            }
            out.push('(');
            for (i, item) in data.values.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                format_value_for_display(item, out, depth + 1);
            }
            out.push(')');
        }
        Value::Function(f) => {
            if let Some(name) = f.name() {
                let _ = write!(out, "<function {}>", name);
            } else {
                out.push_str("<function>");
            }
        }
        Value::BytecodeFunction(f) => {
            if let Some(name) = f.function.name.as_deref() {
                let _ = write!(out, "<function {}>", name);
            } else {
                out.push_str("<function>");
            }
        }
        Value::BytecodeSequence(_) | Value::Sequence(_) => {
            out.push_str("<sequence>");
        }
        Value::BytecodeIterator(_) => {
            out.push_str("<iterator>");
        }
        Value::Builtin(b) => {
            let _ = write!(out, "<builtin {:?}>", b);
        }
        Value::Host(h) => {
            let _ = write!(out, "<host function {}>", h.name);
        }
        Value::HostObject(o) => {
            let _ = write!(out, "<host object {:?}>", o);
        }
        Value::Range { start, end, step } => {
            let _ = write!(out, "range({}, {}, {})", start, end, step);
        }
        Value::Mesh(_) => out.push_str("<Mesh>"),
        Value::Polygon(_) => out.push_str("<Polygon>"),
        Value::Matrix(_) => out.push_str("<Matrix4>"),
        Value::Quaternion(_) => out.push_str("<Quaternion>"),
    }
}
