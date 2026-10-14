//! Shared builtin catalog, O(1) name lookup, and pure builtin evaluation for AST and Bytecode VM.
use crate::runtime::ops::{check_vector_finite, format_value_for_display, is_truthy};
use crate::{Builtin, Value};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The runtime registration table, shared with editor name validation.
pub fn builtin_catalog() -> &'static [(&'static str, Builtin)] {
    &[
        ("assert", Builtin::Assert),
        ("json_parse", Builtin::JsonParse),
        ("json_stringify", Builtin::JsonStringify),
        ("trim", Builtin::Trim),
        ("trim_start", Builtin::TrimStart),
        ("trim_end", Builtin::TrimEnd),
        ("split", Builtin::Split),
        ("join", Builtin::Join),
        ("starts_with", Builtin::StartsWith),
        ("ends_with", Builtin::EndsWith),
        ("contains", Builtin::Contains),
        ("replace", Builtin::Replace),
        ("to_lower", Builtin::ToLower),
        ("to_upper", Builtin::ToUpper),
        ("len", Builtin::Len),
        ("get", Builtin::Get),
        ("any", Builtin::Any),
        ("all", Builtin::All),
        ("degrees", Builtin::Degrees),
        ("radians", Builtin::Radians),
        ("Some", Builtin::Some),
        ("None", Builtin::None),
        ("Ok", Builtin::Ok),
        ("Err", Builtin::Err),
        ("random", Builtin::Random),
        ("noise", Builtin::Noise),
        ("transform", Builtin::Transform),
        ("grid_mesh", Builtin::GridMesh),
        ("mesh", Builtin::Mesh),
        ("slerp", Builtin::Slerp),
        ("axis_angle", Builtin::AxisAngle),
        ("rotation_matrix", Builtin::RotationMatrix),
        ("identity", Builtin::Identity),
        ("promote", Builtin::Promote),
        ("arena", Builtin::Arena),
        ("arena_run", Builtin::ArenaRun),
        ("arena_stats", Builtin::ArenaStats),
        ("translation", Builtin::Translation),
        ("scaling", Builtin::Scaling),
        ("rotation_x", Builtin::RotationX),
        ("rotation_y", Builtin::RotationY),
        ("rotation_z", Builtin::RotationZ),
        ("transform_point", Builtin::TransformPoint),
        ("transform_direction", Builtin::TransformDirection),
        ("polygon", Builtin::Polygon),
        ("translate", Builtin::Translate),
        ("rotate", Builtin::Rotate),
        ("zip", Builtin::Zip),
        ("range", Builtin::Range),
        ("range_iter", Builtin::RangeIter),
        ("iter", Builtin::Iter),
        ("collect", Builtin::Collect),
        ("cross", Builtin::Cross),
        ("lerp", Builtin::Lerp),
        ("clamp", Builtin::Clamp),
        ("smoothstep", Builtin::Smoothstep),
        ("vec2", Builtin::Vec2),
        ("vec3", Builtin::Vec3),
        ("vec4", Builtin::Vec4),
        ("dot", Builtin::Dot),
        ("length", Builtin::Length),
        ("normalize", Builtin::Normalize),
        ("sin", Builtin::Sin),
        ("cos", Builtin::Cos),
        ("sqrt", Builtin::Sqrt),
        ("deg", Builtin::Deg),
        ("map", Builtin::Map),
        ("flat_map", Builtin::FlatMap),
        ("filter", Builtin::Filter),
        ("fold", Builtin::Fold),
        ("group_by", Builtin::GroupBy),
        ("fold_by", Builtin::FoldBy),
    ]
}

/// Fast O(1) compiled jump-table lookup for builtin names.
#[inline]
pub fn lookup_builtin(name: &str) -> Option<Builtin> {
    Some(match name {
        "assert" => Builtin::Assert,
        "json_parse" => Builtin::JsonParse,
        "json_stringify" => Builtin::JsonStringify,
        "trim" => Builtin::Trim,
        "trim_start" => Builtin::TrimStart,
        "trim_end" => Builtin::TrimEnd,
        "split" => Builtin::Split,
        "join" => Builtin::Join,
        "starts_with" => Builtin::StartsWith,
        "ends_with" => Builtin::EndsWith,
        "contains" => Builtin::Contains,
        "replace" => Builtin::Replace,
        "to_lower" => Builtin::ToLower,
        "to_upper" => Builtin::ToUpper,
        "len" => Builtin::Len,
        "get" => Builtin::Get,
        "any" => Builtin::Any,
        "all" => Builtin::All,
        "degrees" => Builtin::Degrees,
        "radians" => Builtin::Radians,
        "Some" => Builtin::Some,
        "None" => Builtin::None,
        "Ok" => Builtin::Ok,
        "Err" => Builtin::Err,
        "random" => Builtin::Random,
        "noise" => Builtin::Noise,
        "transform" => Builtin::Transform,
        "grid_mesh" => Builtin::GridMesh,
        "mesh" => Builtin::Mesh,
        "slerp" => Builtin::Slerp,
        "axis_angle" => Builtin::AxisAngle,
        "rotation_matrix" => Builtin::RotationMatrix,
        "identity" => Builtin::Identity,
        "promote" => Builtin::Promote,
        "arena" => Builtin::Arena,
        "arena_run" => Builtin::ArenaRun,
        "arena_stats" => Builtin::ArenaStats,
        "translation" => Builtin::Translation,
        "scaling" => Builtin::Scaling,
        "rotation_x" => Builtin::RotationX,
        "rotation_y" => Builtin::RotationY,
        "rotation_z" => Builtin::RotationZ,
        "transform_point" => Builtin::TransformPoint,
        "transform_direction" => Builtin::TransformDirection,
        "polygon" => Builtin::Polygon,
        "translate" => Builtin::Translate,
        "rotate" => Builtin::Rotate,
        "zip" => Builtin::Zip,
        "range" => Builtin::Range,
        "range_iter" => Builtin::RangeIter,
        "iter" => Builtin::Iter,
        "collect" => Builtin::Collect,
        "cross" => Builtin::Cross,
        "lerp" => Builtin::Lerp,
        "clamp" => Builtin::Clamp,
        "smoothstep" => Builtin::Smoothstep,
        "vec2" => Builtin::Vec2,
        "vec3" => Builtin::Vec3,
        "vec4" => Builtin::Vec4,
        "dot" => Builtin::Dot,
        "length" => Builtin::Length,
        "normalize" => Builtin::Normalize,
        "sin" => Builtin::Sin,
        "cos" => Builtin::Cos,
        "sqrt" => Builtin::Sqrt,
        "deg" => Builtin::Deg,
        "map" => Builtin::Map,
        "flat_map" => Builtin::FlatMap,
        "filter" => Builtin::Filter,
        "fold" => Builtin::Fold,
        "group_by" => Builtin::GroupBy,
        "fold_by" => Builtin::FoldBy,
        _ => return None,
    })
}

/// Evaluate mathematical, vector, matrix, quaternion, angle, and polygon builtins.
pub(crate) fn eval_math_builtin<'s>(
    builtin: Builtin,
    arguments: &[Value<'s>],
) -> Result<Value<'s>, &'static str> {
    if matches!(builtin, Builtin::Vec2 | Builtin::Vec3 | Builtin::Vec4) {
        let dimension = match builtin {
            Builtin::Vec2 => 2,
            Builtin::Vec3 => 3,
            _ => 4,
        };
        if arguments.len() != dimension {
            return Err("Incorrect vector dimension");
        }
        let mut values = Vec::with_capacity(dimension);
        for v in arguments {
            if let Value::Number(n) = v {
                values.push(*n);
            } else {
                return Err("Vector components must be numbers");
            }
        }
        return check_vector_finite(values);
    }

    match (builtin, arguments) {
        (Builtin::Slerp, [Value::Quaternion(a), Value::Quaternion(b), Value::Number(t)]) => {
            return a
                .slerp(b, *t)
                .map(|q| Value::Quaternion(Box::new(q)))
                .ok_or("slerp requires a finite fraction between 0 and 1");
        }
        (
            Builtin::AxisAngle,
            [
                Value::Vector(axis),
                Value::Number(angle) | Value::Angle(angle),
            ],
        ) if axis.len() == 3 => {
            return crate::Quaternion::axis_angle([axis[0], axis[1], axis[2]], *angle)
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
            [Value::Number(angle) | Value::Angle(angle)],
        ) => {
            let mut matrix = crate::Matrix4::identity();
            let (sin, cos) = angle.sin_cos();
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

    if let (Builtin::Degrees | Builtin::Radians, [Value::Number(value)]) = (builtin, arguments) {
        let radians = if builtin == Builtin::Degrees {
            value.to_radians()
        } else {
            *value
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
                Value::Number(angle) | Value::Angle(angle),
            ],
        ) => Some(polygon.rotated(*angle)),
        _ => None,
    };
    if let Some(shape) = shape {
        return shape.map(Value::Polygon);
    }

    let number = match (builtin, arguments) {
        (Builtin::Random, [Value::Number(seed), Value::Number(index)]) => {
            crate::noise::random(*seed, *index)
                .ok_or("random requires nonnegative exact integer seed and index")?
        }
        (Builtin::Noise, [Value::Number(x), Value::Number(seed)]) => crate::noise::noise(*x, *seed)
            .ok_or("noise requires a bounded coordinate and nonnegative exact integer seed")?,
        (Builtin::Cross, [Value::Vector(a), Value::Vector(b)]) if a.len() == 3 && b.len() == 3 => {
            return check_vector_finite(vec![
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]);
        }
        (Builtin::Lerp, [Value::Vector(a), Value::Vector(b), Value::Number(t)])
            if a.len() == b.len() =>
        {
            return check_vector_finite(
                a.iter()
                    .zip(b)
                    .map(|(a, b)| a * (1.0 - t) + b * t)
                    .collect(),
            );
        }
        (Builtin::Lerp, [Value::Number(a), Value::Number(b), Value::Number(t)]) => {
            a * (1.0 - t) + b * t
        }
        (Builtin::Clamp, [Value::Number(x), Value::Number(low), Value::Number(high)])
            if low <= high =>
        {
            x.clamp(*low, *high)
        }
        (Builtin::Smoothstep, [Value::Number(low), Value::Number(high), Value::Number(x)])
            if low < high =>
        {
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
            a.iter().zip(b).map(|(a, b)| a * b).sum()
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
        (Builtin::Cos, [Value::Number(x) | Value::Angle(x)]) => x.cos(),
        (Builtin::Sqrt, [Value::Number(x)]) => x.sqrt(),
        (Builtin::Deg, [Value::Number(x)]) => x.to_radians(),
        _ => return Err("Invalid mathematical arguments"),
    };
    if number.is_finite() {
        Ok(Value::Number(number))
    } else {
        Err("Non-finite mathematical result")
    }
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
            Ok(Value::String(s.to_lowercase()))
        }
        Builtin::ToUpper => {
            let [Value::String(s)] = args else {
                return Err("to_upper requires a string".into());
            };
            Ok(Value::String(s.to_uppercase()))
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
                (Value::List(items), needle) => Ok(Value::Bool(items.contains(needle))),
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
            let mut out = String::new();
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
                Value::String(s) => s.chars().count(),
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
                (Value::List(items) | Value::Tuple(items), Value::Number(index)) => {
                    if !index.is_finite() || index.fract() != 0.0 || *index < 0.0 {
                        return Err("Collection index must be a non-negative integer".into());
                    }
                    if *index >= items.len() as f64 {
                        None
                    } else {
                        items.get(*index as usize).cloned()
                    }
                }
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
            let Some(Value::String(name)) = args.first() else {
                return Err("arena requires a name string".into());
            };
            if name.is_empty() {
                return Err("arena name must not be empty".into());
            }
            let budget = match args.get(1) {
                None | Some(Value::Null) => Value::Null,
                Some(Value::Number(v)) if v.is_finite() && v.fract() == 0.0 && *v >= 0.0 => {
                    Value::Number(*v)
                }
                _ => return Err("arena budget must be a non-negative integer".into()),
            };
            Ok(Value::Record(BTreeMap::from([
                ("name".into(), Value::String(name.clone())),
                ("budget".into(), budget),
            ])))
        }
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
                    let Value::Number(index) = index else {
                        return Err("Mesh index must be an integer".into());
                    };
                    if !index.is_finite()
                        || index.fract() != 0.0
                        || *index < 0.0
                        || *index >= vertices.len() as f64
                    {
                        return Err("Mesh index out of bounds or non-integer".into());
                    }
                    *slot = *index as usize;
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
