//! Shared builtin catalog, O(1) name lookup, and pure builtin evaluation for AST and Bytecode VM.
use crate::runtime::ops::{check_vector_finite, format_value_for_display, is_truthy};
use crate::{RecordMap, Value};
use std::rc::Rc;

pub use crate::catalog::{Builtin, builtin_catalog, lookup_builtin};

/// Evaluate mathematical, vector, matrix, quaternion, angle, and polygon builtins.
pub(crate) fn eval_math_builtin<'s>(
    builtin: Builtin,
    arguments: &[Value<'s>],
) -> Result<Value<'s>, &'static str> {
    match (builtin, arguments) {
        (Builtin::Int, [arg]) => {
            return match arg {
                Value::Int(i) => Ok(Value::Int(*i)),
                Value::Number(n)
                    if n.is_finite()
                        && *n >= (i64::MIN as f64)
                        && *n < 9_223_372_036_854_775_808.0 =>
                {
                    Ok(Value::Int(*n as i64))
                }
                Value::Number(_) => Err("Cannot convert non-finite or out-of-range number to int"),
                Value::Bool(b) => Ok(Value::Int(i64::from(*b))),
                Value::String(s) => crate::runtime::parse_signed_int_str(s)
                    .map(Value::Int)
                    .ok_or("Invalid integer string"),
                _ => Err("int requires an int, number, bool or string"),
            };
        }
        (Builtin::Float, [arg]) => {
            return match arg {
                Value::Number(n) => {
                    if n.is_finite() {
                        Ok(Value::Number(*n))
                    } else {
                        Err("Non-finite mathematical result")
                    }
                }
                Value::Int(i) => Ok(Value::Number(*i as f64)),
                Value::Bool(b) => Ok(Value::Number(if *b { 1.0 } else { 0.0 })),
                Value::String(s) => crate::runtime::parse_f64_literal(s.trim())
                    .map(Value::Number)
                    .ok_or("Invalid float string"),
                _ => Err("float requires an int, number, bool or string"),
            };
        }
        (Builtin::BitNot, [arg]) => {
            let Some(a) = arg.as_exact_i64() else {
                return Err("Bitwise operations require integer operands");
            };
            return Ok(Value::Int(!a));
        }
        (
            Builtin::BitAnd | Builtin::BitOr | Builtin::BitXor | Builtin::Shl | Builtin::Shr,
            [left, right],
        ) => {
            let (Some(a), Some(b)) = (left.as_exact_i64(), right.as_exact_i64()) else {
                return Err("Bitwise operations require integer operands");
            };
            return match builtin {
                Builtin::BitAnd => Ok(Value::Int(a & b)),
                Builtin::BitOr => Ok(Value::Int(a | b)),
                Builtin::BitXor => Ok(Value::Int(a ^ b)),
                Builtin::Shl => u32::try_from(b)
                    .ok()
                    .and_then(|s| a.checked_shl(s))
                    .map(Value::Int)
                    .ok_or("Invalid shift count"),
                Builtin::Shr => u32::try_from(b)
                    .ok()
                    .and_then(|s| a.checked_shr(s))
                    .map(Value::Int)
                    .ok_or("Invalid shift count"),
                _ => unreachable!(),
            };
        }
        _ => {}
    }

    if matches!(builtin, Builtin::Vec2 | Builtin::Vec3 | Builtin::Vec4) {
        let dimension = match builtin {
            Builtin::Vec2 => 2,
            Builtin::Vec3 => 3,
            _ => 4,
        };
        if arguments.len() != dimension {
            return Err("Incorrect vector dimension");
        }
        let mut comps = crate::InlineVec::new();
        for arg in arguments {
            let Some(v) = arg.as_f64() else {
                return Err("Vector components must be numbers");
            };
            if !v.is_finite() {
                return Err("Non-finite vector result");
            }
            comps.push(v);
        }
        return Ok(Value::Vector(comps));
    }

    match (builtin, arguments) {
        (Builtin::Slerp, [Value::Quaternion(a), Value::Quaternion(b), t])
            if t.as_f64().is_some() =>
        {
            return a
                .slerp(b, t.as_f64().unwrap())
                .map(|q| Value::Quaternion(Box::new(q)))
                .ok_or("slerp requires a finite fraction between 0 and 1");
        }
        (Builtin::AxisAngle, [Value::Vector(axis), angle]) if axis.len() == 3 => {
            let Some(rad) = (match angle {
                Value::Angle(a) => Some(*a),
                other => other.as_f64(),
            }) else {
                return Err("Invalid mathematical arguments");
            };
            return crate::Quaternion::axis_angle([axis[0], axis[1], axis[2]], rad)
                .map(|q| Value::Quaternion(Box::new(q)))
                .ok_or("Rotation requires a finite nonzero axis and angle");
        }
        (Builtin::RotationMatrix, [Value::Quaternion(q)]) => {
            return Ok(Value::Matrix(Box::new(q.matrix())));
        }
        (Builtin::Identity, []) => {
            return Ok(Value::Matrix(Box::new(crate::Matrix4::identity())));
        }
        (Builtin::Translation | Builtin::Scaling, [Value::Vector(v)]) if v.len() == 3 => {
            let mut matrix = crate::Matrix4::identity();
            for (axis, value) in v.iter().enumerate() {
                if builtin == Builtin::Translation {
                    matrix.0[axis][3] = *value;
                } else {
                    matrix.0[axis][axis] = *value;
                }
            }
            return Ok(Value::Matrix(Box::new(matrix)));
        }
        (
            Builtin::RotationX | Builtin::RotationY | Builtin::RotationZ,
            [angle @ (Value::Number(_) | Value::Int(_) | Value::Angle(_))],
        ) => {
            let rad = match angle {
                Value::Angle(a) => *a,
                other => other.as_f64().unwrap(),
            };
            let mut matrix = crate::Matrix4::identity();
            let (sin, cos) = rad.sin_cos();
            let (a, b) = match builtin {
                Builtin::RotationX => (1, 2),
                Builtin::RotationY => (2, 0),
                _ => (0, 1),
            };
            matrix.0[a][a] = cos;
            matrix.0[a][b] = -sin;
            matrix.0[b][a] = sin;
            matrix.0[b][b] = cos;
            return Ok(Value::Matrix(Box::new(matrix)));
        }
        (
            Builtin::TransformPoint | Builtin::TransformDirection,
            [Value::Matrix(matrix), Value::Vector(v)],
        ) => {
            return matrix
                .apply(v, builtin == Builtin::TransformPoint)
                .map(Value::Vector)
                .ok_or("Invalid matrix transformation");
        }
        _ => {}
    }

    if let (Builtin::Degrees | Builtin::Radians, [val]) = (builtin, arguments)
        && let Some(value) = val.as_f64()
    {
        let radians = if builtin == Builtin::Degrees {
            value.to_radians()
        } else {
            value
        };
        if !radians.is_finite() {
            return Err("Non-finite angle");
        }
        return Ok(Value::Angle(radians));
    }

    let shape = match (builtin, arguments) {
        (Builtin::Polygon, [Value::List(points)]) => {
            let points = points
                .iter()
                .map(|p| match p {
                    Value::Vector(v) if v.len() == 2 => Some([v[0], v[1]]),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();
            Some(match points {
                Some(points) => crate::Polygon::new(points),
                None => Err("Polygon points must be vec2 values"),
            })
        }
        (Builtin::Translate, [Value::Polygon(polygon), Value::Vector(offset)])
            if offset.len() == 2 =>
        {
            Some(polygon.translated([offset[0], offset[1]]))
        }
        (
            Builtin::Rotate,
            [
                Value::Polygon(polygon),
                angle @ (Value::Number(_) | Value::Int(_) | Value::Angle(_)),
            ],
        ) => {
            let rad = match angle {
                Value::Angle(a) => *a,
                other => other.as_f64().unwrap(),
            };
            Some(polygon.rotated(rad))
        }
        _ => None,
    };
    if let Some(shape) = shape {
        return shape.map(Value::Polygon);
    }

    if let (Builtin::Clamp, [Value::Int(x), Value::Int(low), Value::Int(high)]) =
        (builtin, arguments)
        && low <= high
    {
        return Ok(Value::Int((*x).clamp(*low, *high)));
    }

    let number = match (builtin, arguments) {
        (Builtin::Random, [seed, index]) if seed.as_f64().is_some() && index.as_f64().is_some() => {
            crate::noise::random(seed.as_f64().unwrap(), index.as_f64().unwrap())
                .ok_or("random requires nonnegative exact integer seed and index")?
        }
        (Builtin::Noise, [x, seed]) if x.as_f64().is_some() && seed.as_f64().is_some() => {
            crate::noise::noise(x.as_f64().unwrap(), seed.as_f64().unwrap())
                .ok_or("noise requires a bounded coordinate and nonnegative exact integer seed")?
        }
        (Builtin::Cross, [Value::Vector(a), Value::Vector(b)]) if a.len() == 3 && b.len() == 3 => {
            let x = a[1] * b[2] - a[2] * b[1];
            let y = a[2] * b[0] - a[0] * b[2];
            let z = a[0] * b[1] - a[1] * b[0];
            return if x.is_finite() && y.is_finite() && z.is_finite() {
                Ok(Value::Vector(crate::InlineVec::vec3(x, y, z)))
            } else {
                Err("Non-finite vector result")
            };
        }
        (Builtin::Lerp, [Value::Vector(a), Value::Vector(b), t])
            if a.len() == b.len() && t.as_f64().is_some() =>
        {
            let t = t.as_f64().unwrap();
            return check_vector_finite(
                a.iter()
                    .zip(b)
                    .map(|(a, b)| a * (1.0 - t) + b * t)
                    .collect(),
            );
        }
        (Builtin::Lerp, [a, b, t])
            if a.as_f64().is_some() && b.as_f64().is_some() && t.as_f64().is_some() =>
        {
            let (a, b, t) = (
                a.as_f64().unwrap(),
                b.as_f64().unwrap(),
                t.as_f64().unwrap(),
            );
            a * (1.0 - t) + b * t
        }
        (Builtin::Clamp, [x, low, high])
            if x.as_f64().is_some()
                && low.as_f64().is_some()
                && high.as_f64().is_some()
                && low.as_f64().unwrap() <= high.as_f64().unwrap() =>
        {
            x.as_f64()
                .unwrap()
                .clamp(low.as_f64().unwrap(), high.as_f64().unwrap())
        }
        (Builtin::Smoothstep, [low, high, x])
            if low.as_f64().is_some()
                && high.as_f64().is_some()
                && x.as_f64().is_some()
                && low.as_f64().unwrap() < high.as_f64().unwrap() =>
        {
            let (low, high, x) = (
                low.as_f64().unwrap(),
                high.as_f64().unwrap(),
                x.as_f64().unwrap(),
            );
            let t = if x <= low {
                0.0
            } else if x >= high {
                1.0
            } else {
                let width = high - low;
                if width.is_finite() {
                    (x - low) / width
                } else {
                    (x * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)
                }
            };
            t * t * (3.0 - 2.0 * t)
        }
        (Builtin::Dot, [Value::Vector(a), Value::Vector(b)]) if a.len() == b.len() => {
            match (a.as_slice(), b.as_slice()) {
                ([a0, a1], [b0, b1]) => a0 * b0 + a1 * b1,
                ([a0, a1, a2], [b0, b1, b2]) => a0 * b0 + a1 * b1 + a2 * b2,
                ([a0, a1, a2, a3], [b0, b1, b2, b3]) => a0 * b0 + a1 * b1 + a2 * b2 + a3 * b3,
                _ => a.iter().zip(b).map(|(a, b)| a * b).sum(),
            }
        }
        (Builtin::Length | Builtin::Normalize, [Value::Vector(v)]) => {
            if builtin == Builtin::Normalize {
                let scale = v.iter().fold(0.0_f64, |scale, x| scale.max(x.abs()));
                if scale == 0.0 || v.iter().any(|x| !x.is_finite()) {
                    return Err("Cannot normalize this vector");
                }
                let length = v.iter().fold(0.0_f64, |length, x| length.hypot(x / scale));
                return check_vector_finite(v.iter().map(|x| (x / scale) / length).collect());
            }
            v.iter().fold(0.0_f64, |length, x| length.hypot(*x))
        }
        (Builtin::Sin, [Value::Number(x) | Value::Angle(x)]) => x.sin(),
        (Builtin::Sin, [Value::Int(x)]) => (*x as f64).sin(),
        (Builtin::Cos, [Value::Number(x) | Value::Angle(x)]) => x.cos(),
        (Builtin::Cos, [Value::Int(x)]) => (*x as f64).cos(),
        (Builtin::Sqrt, [Value::Number(x)]) => x.sqrt(),
        (Builtin::Sqrt, [Value::Int(x)]) => (*x as f64).sqrt(),
        (Builtin::Deg, [Value::Number(x)]) => x.to_radians(),
        (Builtin::Deg, [Value::Int(x)]) => (*x as f64).to_radians(),
        _ => return Err("Invalid mathematical arguments"),
    };
    if number.is_finite() {
        Ok(Value::Number(number))
    } else {
        Err("Non-finite mathematical result")
    }
}

/// Validate `arena(name, budget?, persistent?, strict?)` arguments and build its descriptor record.
pub(crate) fn build_arena_descriptor<'s>(
    args: &[Value<'s>],
) -> Result<RecordMap<'s>, &'static str> {
    let Some(Value::String(name)) = args.first() else {
        return Err("arena requires a name string");
    };
    if name.is_empty() {
        return Err("arena name must not be empty");
    }
    let budget = match args.get(1) {
        None | Some(Value::Null) => Value::Null,
        Some(v) if v.as_usize_index().is_some() => v.clone(),
        _ => return Err("arena budget must be a non-negative integer"),
    };
    let persistent = match args.get(2) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err("arena persistent flag must be a boolean"),
    };
    let strict = match args.get(3) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err("arena strict flag must be a boolean"),
    };
    Ok(RecordMap::from([
        ("name".into(), Value::String(name.clone())),
        ("budget".into(), budget),
        ("persistent".into(), Value::Bool(persistent)),
        ("strict".into(), Value::Bool(strict)),
    ]))
}

/// Evaluate pure, callback-free builtins shared across Bytecode VM and AST runtime.
pub(crate) fn eval_pure_builtin<'s>(
    builtin: Builtin,
    args: &[Value<'s>],
) -> Result<Value<'s>, String> {
    match builtin {
        Builtin::Trim => {
            let [Value::String(s)] = args else {
                return Err("trim requires a string".into());
            };
            Ok(Value::String(s.trim().to_string()))
        }
        Builtin::TrimStart => {
            let [Value::String(s)] = args else {
                return Err("trim_start requires a string".into());
            };
            Ok(Value::String(s.trim_start().to_string()))
        }
        Builtin::TrimEnd => {
            let [Value::String(s)] = args else {
                return Err("trim_end requires a string".into());
            };
            Ok(Value::String(s.trim_end().to_string()))
        }
        Builtin::ToLower => {
            let [Value::String(s)] = args else {
                return Err("to_lower requires a string".into());
            };
            let lowered = if s.is_ascii() {
                s.to_ascii_lowercase()
            } else {
                s.to_lowercase()
            };
            Ok(Value::String(lowered))
        }
        Builtin::ToUpper => {
            let [Value::String(s)] = args else {
                return Err("to_upper requires a string".into());
            };
            let uppered = if s.is_ascii() {
                s.to_ascii_uppercase()
            } else {
                s.to_uppercase()
            };
            Ok(Value::String(uppered))
        }
        Builtin::StartsWith => {
            let [Value::String(s), Value::String(prefix)] = args else {
                return Err("starts_with requires string arguments".into());
            };
            Ok(Value::Bool(s.starts_with(prefix.as_str())))
        }
        Builtin::EndsWith => {
            let [Value::String(s), Value::String(suffix)] = args else {
                return Err("ends_with requires string arguments".into());
            };
            Ok(Value::Bool(s.ends_with(suffix.as_str())))
        }
        Builtin::Contains => {
            if args.len() != 2 {
                return Err("contains requires 2 arguments".into());
            }
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(needle)) => {
                    Ok(Value::Bool(s.contains(needle.as_str())))
                }
                (Value::List(items), needle) => {
                    Ok(Value::Bool(items.iter().any(|item| {
                        crate::runtime::ops::values_equal(item, needle)
                    })))
                }
                (Value::Record(fields), Value::String(k)) => {
                    Ok(Value::Bool(fields.contains_key(k)))
                }
                _ => Err(
                    "contains requires (string, string), (list, value), or (record, string)".into(),
                ),
            }
        }
        Builtin::Replace => {
            let [Value::String(s), Value::String(from), Value::String(to)] = args else {
                return Err("replace requires string arguments".into());
            };
            Ok(Value::String(s.replace(from.as_str(), to.as_str())))
        }
        Builtin::Split => {
            let [Value::String(s), Value::String(del)] = args else {
                return Err("split requires string arguments".into());
            };
            let parts: Vec<Value<'s>> = if del.is_empty() {
                s.chars().map(|c| Value::String(c.to_string())).collect()
            } else {
                s.split(del.as_str())
                    .map(|p| Value::String(p.to_string()))
                    .collect()
            };
            Ok(Value::List(parts))
        }
        Builtin::Join => {
            let [Value::List(items), Value::String(sep)] = args else {
                return Err("join requires a list and a separator".into());
            };
            let mut out = String::with_capacity(items.len() * (4 + sep.len()));
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(sep);
                }
                format_value_for_display(item, &mut out, 0);
            }
            Ok(Value::String(out))
        }
        Builtin::Len => {
            let [arg] = args else {
                return Err("len requires one argument".into());
            };
            let count = match arg {
                Value::String(s) => {
                    if s.is_ascii() {
                        s.len()
                    } else {
                        s.chars().count()
                    }
                }
                Value::List(l) | Value::Tuple(l) => l.len(),
                Value::Record(r) => r.len(),
                Value::Vector(v) => v.len(),
                _ => return Err("len requires a list, tuple, record or string".into()),
            };
            Ok(Value::Number(count as f64))
        }
        Builtin::Get => {
            if args.len() != 2 {
                return Err("get requires a collection and key".into());
            }
            let value = match (&args[0], &args[1]) {
                (Value::Record(fields), Value::String(key)) => fields.get(key).cloned(),
                (
                    Value::List(items) | Value::Tuple(items),
                    index @ (Value::Number(_) | Value::Int(_)),
                ) => match index {
                    Value::Int(i) if *i >= 0 => items.get(*i as usize).cloned(),
                    Value::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => {
                        usize::try_from(*n as u128)
                            .ok()
                            .and_then(|idx| items.get(idx).cloned())
                    }
                    _ => return Err("Collection index must be a non-negative integer".into()),
                },
                _ => {
                    return Err(
                        "get requires a record and string key, or list/tuple and integer index"
                            .into(),
                    );
                }
            };
            Ok(match value {
                Some(v) => Value::Variant("Some", vec![v]),
                None => Value::Variant("None", Vec::new()),
            })
        }
        Builtin::Assert => {
            if !builtin.arity().contains(&args.len()) {
                return Err("Incorrect argument count".into());
            }
            let msg = match args.get(1) {
                None => "Assertion failed",
                Some(Value::String(s)) => s.as_str(),
                _ => return Err("Assertion message must be a string".into()),
            };
            if !is_truthy(&args[0]) {
                return Err(msg.into());
            }
            Ok(Value::Null)
        }
        Builtin::JsonParse => {
            let [Value::String(text)] = args else {
                return Err("json_parse requires a string".into());
            };
            Ok(match crate::json_parse(text) {
                Ok(v) => Value::Variant("Ok", vec![v]),
                Err(e) => Value::Variant("Err", vec![Value::String(e)]),
            })
        }
        Builtin::JsonStringify => {
            let [arg] = args else {
                return Err("json_stringify requires one argument".into());
            };
            crate::json_stringify(arg)
                .map(Value::String)
                .map_err(|e| format!("json_stringify failed: {e}"))
        }
        Builtin::Promote => {
            let [val] = args else {
                return Err("promote requires one argument".into());
            };
            Ok(val.clone())
        }
        Builtin::Arena => {
            if !builtin.arity().contains(&args.len()) {
                return Err("Incorrect builtin argument count".into());
            }
            build_arena_descriptor(args)
                .map(Value::Record)
                .map_err(Into::into)
        }
        Builtin::ArenaRun | Builtin::ArenaStats | Builtin::ArenaReset => {
            Err("Arena runtime operations require the tree-walk runtime".into())
        }
        Builtin::TaskAll | Builtin::TaskRace => {
            Err("Task concurrency operations require CoroutineScheduler".into())
        }
        Builtin::GridMesh => Err("grid_mesh requires the tree-walk runtime".into()),
        Builtin::Some | Builtin::None | Builtin::Ok | Builtin::Err => {
            let (tag, count) = match builtin {
                Builtin::Some => ("Some", 1),
                Builtin::None => ("None", 0),
                Builtin::Ok => ("Ok", 1),
                _ => ("Err", 1),
            };
            if args.len() != count {
                return Err("Invalid variant argument count".into());
            }
            Ok(Value::Variant(tag, args.to_vec()))
        }
        Builtin::Mesh => {
            let [Value::List(points), Value::List(faces)] = args else {
                return Err("mesh requires vertex and triangle lists".into());
            };
            let mut vertices = Vec::with_capacity(points.len());
            for point in points {
                let Value::Vector(v) = point else {
                    return Err("Mesh vertex must be vec3".into());
                };
                if v.len() != 3 {
                    return Err("Mesh vertex must be vec3".into());
                }
                vertices.push([v[0], v[1], v[2]]);
            }
            let mut triangles = Vec::with_capacity(faces.len());
            for face in faces {
                let Value::List(indices) = face else {
                    return Err("Mesh triangle must be a list of three indices".into());
                };
                if indices.len() != 3 {
                    return Err("Mesh triangle must have three indices".into());
                }
                let mut triangle = [0; 3];
                for (slot, index) in triangle.iter_mut().zip(indices) {
                    if !matches!(index, Value::Number(_) | Value::Int(_)) {
                        return Err("Mesh index must be an integer".into());
                    }
                    let Some(idx) = index.as_usize_index().filter(|&i| i < vertices.len()) else {
                        return Err("Mesh index out of bounds or non-integer".into());
                    };
                    *slot = idx;
                }
                triangles.push(triangle);
            }
            crate::Mesh::new(vertices, triangles)
                .map(|m| Value::Mesh(Rc::new(m)))
                .map_err(Into::into)
        }
        Builtin::Transform => {
            let [Value::Mesh(mesh), Value::Matrix(matrix)] = args else {
                return Err("transform requires a mesh and mat4".into());
            };
            let mut vertices = Vec::with_capacity(mesh.vertices().len());
            for vertex in mesh.vertices() {
                let transformed = matrix
                    .apply(vertex, true)
                    .ok_or_else(|| "Mesh transformation overflow".to_string())?;
                vertices.push([transformed[0], transformed[1], transformed[2]]);
            }
            crate::Mesh::new(vertices, mesh.triangles().to_vec())
                .map(|m| Value::Mesh(Rc::new(m)))
                .map_err(Into::into)
        }
        _ => eval_math_builtin(builtin, args).map_err(Into::into),
    }
}
