use super::stmt::Flow;
use super::{
    Builtin, CallFrame, Closure, Environment, FunctionBody, RecordMap, RecordShape, RegionStats,
    Result, Runtime, RuntimeError, SequenceStage, StageKind, UserData, Value, ValueType,
    format_value_for_display, json, memory,
};
use crate::{Expr, ExprKind, Name};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

impl<'s> Runtime<'_, 's> {
    pub(super) fn call(
        &mut self,
        function: Value<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        self.check(span)?;
        if self.remaining == 0 {
            return self.error(span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        // Capture only borrowed/copyable labels on the successful path. Stack
        // strings and source coordinates are needed only when a call fails.
        let (name, builtin) = match &function {
            Value::Function(f) => (f.name.unwrap_or("<lambda>").to_owned(), None),
            Value::BytecodeFunction(f) => (
                f.function.name.as_deref().unwrap_or("<lambda>").to_owned(),
                None,
            ),
            Value::Host(f) => (f.name.to_owned(), None),
            Value::Builtin(b) => (String::new(), Some(*b)),
            _ => ("<non-callable>".to_owned(), None),
        };
        let module = self.module;
        let result = match function {
            Value::Function(function) => self.call_user(function, arguments, span),
            Value::BytecodeFunction(closure) => {
                let mut vm = crate::bytecode::Vm::new(self.remaining);
                let static_args: Vec<_> = arguments.iter().filter_map(|a| a.to_static()).collect();
                if static_args.len() != arguments.len() {
                    return self.error(span, "Non-static arguments in bytecode call");
                }
                match vm.run_closure(&closure, &static_args) {
                    Ok(val) => {
                        self.remaining = vm.fuel();
                        Ok(val)
                    }
                    Err(e) => self.error(span, &e.message),
                }
            }
            other => self.call_inner(other, arguments, span),
        };
        result.map_err(|error| {
            let name = builtin.map_or(name, |b| format!("{b:?}"));
            self.call_error(error, name, module, span)
        })
    }

    pub(super) fn call_error(
        &self,
        mut error: RuntimeError,
        name: String,
        module: Option<&'s str>,
        span: Span,
    ) -> RuntimeError {
        let source = self.sources.get(&module).copied().unwrap_or("");
        let prefix = source.get(..span.start).unwrap_or("");
        let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        if error.location.is_none()
            && let Some((_, source)) = self
                .sources
                .iter()
                .find(|(module, _)| module.map(str::to_owned) == error.module)
        {
            error = error.locate(source);
        }
        error.stack.push(CallFrame {
            function: name,
            module: module.map(str::to_owned),
            span,
            line,
            column,
        });
        error
    }

    pub(super) fn call_inner(
        &mut self,
        function: Value<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        if let Value::Host(function) = function {
            if arguments.len() != function.parameters.len()
                || !function
                    .parameters
                    .iter()
                    .zip(arguments.iter())
                    .all(|(ty, value)| ty.accepts(value))
            {
                return self.error(span, "Host function arguments do not match its signature");
            }
            let coerced_args: Cow<'_, [Value<'s>]> = if function
                .parameters
                .iter()
                .zip(arguments.iter())
                .any(|(ty, v)| {
                    matches!(
                        (ty, v),
                        (ValueType::Int, Value::Number(_)) | (ValueType::Number, Value::Int(_))
                    )
                }) {
                Cow::Owned(
                    arguments
                        .iter()
                        .zip(&function.parameters)
                        .map(|(v, ty)| v.clone().coerce_for_type(ty))
                        .collect(),
                )
            } else {
                arguments
            };
            let result = match self.contextual.get(&Rc::as_ptr(&function)) {
                Some(callback) => callback(&coerced_args, self.cancellation),
                None => (function.callback)(&coerced_args, self.cancellation),
            }
            .map_err(|message| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message,
            })?;
            self.check(span)?;
            self.host_value_size(&result, span, 0)?;
            if !function.result.accepts(&result) {
                return self.error(span, "Host function returned an invalid value");
            }
            return Ok(result.coerce_for_type(&function.result));
        }
        if let Value::Builtin(builtin) = function {
            for &(index, expected) in builtin.callback_arities() {
                let valid = match arguments.get(index) {
                    Some(Value::Function(function)) => Some(function.parameters.len() == expected),
                    Some(Value::BytecodeFunction(function)) => {
                        Some(function.function.arity == expected)
                    }
                    Some(Value::Host(function)) => Some(function.parameters.len() == expected),
                    Some(Value::Builtin(function)) => Some(function.arity().contains(&expected)),
                    _ => None,
                };
                if valid == Some(false) {
                    return self.error(span, "Callback argument count does not match operation");
                }
            }
            if builtin == Builtin::Iter {
                let [source] = arguments.as_ref() else {
                    return self.error(span, "iter requires one list or sequence");
                };
                let cursor = self.sequence_cursor(source.clone(), span)?;
                return Ok(Value::Sequence(cursor.sequence));
            }
            if builtin == Builtin::Collect {
                let [source, limit] = arguments.as_ref() else {
                    return self.error(span, "collect requires a sequence and maximum item count");
                };
                if !matches!(limit, Value::Number(_) | Value::Int(_)) {
                    return self.error(span, "collect requires a sequence and maximum item count");
                }
                let Some(limit) = limit.as_usize_index() else {
                    return self.error(
                        span,
                        "Collection limit must be a nonnegative representable integer",
                    );
                };
                if limit > self.max_collection_items {
                    return self.error(span, "Collection item limit exceeded");
                }
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                let mut output = Vec::new();
                for _ in 0..limit {
                    let Some(item) = self.sequence_next(&mut cursor, span)? else {
                        break;
                    };
                    self.charge(1, span)?;
                    output.push(item);
                }
                return Ok(Value::List(output));
            }
            if matches!(builtin, Builtin::Map | Builtin::Filter | Builtin::FlatMap)
                && matches!(
                    arguments.first(),
                    Some(Value::Range { .. } | Value::Sequence(_) | Value::BytecodeSequence(_))
                )
            {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "map/filter/flat_map require a sequence and callback");
                };
                if !matches!(
                    callback,
                    Value::Function(_)
                        | Value::BytecodeFunction(_)
                        | Value::Builtin(_)
                        | Value::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let cursor = self.sequence_cursor(source.clone(), span)?;
                self.charge(cursor.sequence.stages.len() + 1, span)?;
                let mut sequence = (*cursor.sequence).clone();
                sequence
                    .stages
                    .append(
                        &self.memory,
                        SequenceStage {
                            callback: callback.clone(),
                            kind: match builtin {
                                Builtin::Filter => StageKind::Filter,
                                Builtin::FlatMap => StageKind::FlatMap,
                                _ => StageKind::Map,
                            },
                            span,
                            module: self.module,
                        },
                    )
                    .map_err(|e| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Runtime sequence allocation failed: {e:?}"),
                    })?;
                return Ok(Value::Sequence(Rc::new(sequence)));
            }
            if matches!(builtin, Builtin::Any | Builtin::All) {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "any/all require a list or sequence and predicate");
                };
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                if !matches!(
                    callback,
                    Value::Function(_)
                        | Value::BytecodeFunction(_)
                        | Value::Builtin(_)
                        | Value::Host(_)
                ) {
                    return self.error(span, "Predicate must be callable");
                }
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let Value::Bool(result) = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                    else {
                        return self.error(span, "Predicate must return a boolean");
                    };
                    if result == (builtin == Builtin::Any) {
                        return Ok(Value::Bool(result));
                    }
                }
                return Ok(Value::Bool(builtin == Builtin::All));
            }
            if builtin == Builtin::Get {
                if arguments.len() != 2 {
                    return self.error(span, "get requires a collection and key");
                }
                let value = match (&arguments[0], &arguments[1]) {
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
                        _ => {
                            return self
                                .error(span, "Collection index must be a non-negative integer");
                        }
                    },
                    _ => {
                        return self.error(
                            span,
                            "get requires a record and string key, or list/tuple and integer index",
                        );
                    }
                };
                return Ok(match value {
                    Some(value) => Value::Variant("Some", vec![value]),
                    None => Value::Variant("None", vec![]),
                });
            }
            if builtin == Builtin::Len {
                if arguments.len() != 1 {
                    return self.error(span, "len requires one argument");
                }
                let count = match &arguments[0] {
                    Value::List(items) | Value::Tuple(items) => items.len(),
                    Value::Record(fields) => fields.len(),
                    Value::String(text) => {
                        self.charge(text.len(), span)?;
                        text.chars().count()
                    }
                    _ => return self.error(span, "len requires a list, tuple, record or string"),
                };
                return Ok(Value::Number(count as f64));
            }
            if builtin == Builtin::Assert {
                if !builtin.arity().contains(&arguments.len()) {
                    return self.error(span, "Incorrect argument count");
                }
                let message = match arguments.get(1) {
                    None => "Assertion failed",
                    Some(Value::String(message)) => message.as_str(),
                    _ => return self.error(span, "Assertion message must be a string"),
                };
                return match arguments[0] {
                    Value::Bool(true) => Ok(Value::Null),
                    Value::Bool(false) => self.error(span, message),
                    _ => self.error(span, "Assertion condition must be a boolean"),
                };
            }
            if !builtin.arity().contains(&arguments.len()) {
                return self.error(span, "Incorrect builtin argument count");
            }

            if builtin == Builtin::JsonParse {
                if arguments.len() != 1 {
                    return self.error(span, "json_parse requires one argument");
                }
                let Value::String(text) = &arguments[0] else {
                    return self.error(span, "json_parse requires a string");
                };
                self.charge(text.len(), span)?;
                return Ok(match json::parse(text) {
                    Ok(val) => Value::Variant("Ok", vec![val]),
                    Err(err) => Value::Variant("Err", vec![Value::String(err)]),
                });
            }
            if builtin == Builtin::JsonStringify {
                if arguments.len() != 1 {
                    return self.error(span, "json_stringify requires one argument");
                }
                let encoded = json::stringify(&arguments[0]).map_err(|e| {
                    self.call_error(
                        RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: format!("json_stringify failed: {e}"),
                        },
                        "json_stringify".into(),
                        self.module,
                        span,
                    )
                })?;
                self.charge(encoded.len(), span)?;
                return Ok(Value::String(encoded));
            }

            if builtin == Builtin::Trim {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "trim requires a string");
                };
                let res = s.trim().to_string();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::TrimStart {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "trim_start requires a string");
                };
                let res = s.trim_start().to_string();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::TrimEnd {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "trim_end requires a string");
                };
                let res = s.trim_end().to_string();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::ToLower {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "to_lower requires a string");
                };
                let res = s.to_lowercase();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::ToUpper {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "to_upper requires a string");
                };
                let res = s.to_uppercase();
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::StartsWith {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "starts_with requires a string as first argument");
                };
                let Value::String(prefix) = &arguments[1] else {
                    return self.error(span, "starts_with requires a string prefix");
                };
                return Ok(Value::Bool(s.starts_with(prefix.as_str())));
            }
            if builtin == Builtin::EndsWith {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "ends_with requires a string as first argument");
                };
                let Value::String(suffix) = &arguments[1] else {
                    return self.error(span, "ends_with requires a string suffix");
                };
                return Ok(Value::Bool(s.ends_with(suffix.as_str())));
            }
            if builtin == Builtin::Contains {
                match (&arguments[0], &arguments[1]) {
                    (Value::String(s), Value::String(needle)) => {
                        return Ok(Value::Bool(s.contains(needle.as_str())));
                    }
                    (Value::List(items), needle) => {
                        return Ok(Value::Bool(
                            items
                                .iter()
                                .any(|item| super::ops::values_equal(item, needle)),
                        ));
                    }
                    (Value::Record(fields), Value::String(key)) => {
                        return Ok(Value::Bool(fields.contains_key(key)));
                    }
                    _ => {
                        return self.error(
                            span,
                            "contains requires (string, string), (list, value), or (record, string)",
                        );
                    }
                }
            }
            if builtin == Builtin::Replace {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "replace requires a string as first argument");
                };
                let Value::String(from) = &arguments[1] else {
                    return self.error(span, "replace requires a string from-pattern");
                };
                let Value::String(to) = &arguments[2] else {
                    return self.error(span, "replace requires a string to-pattern");
                };
                let res = s.replace(from.as_str(), to.as_str());
                self.string_growth(0, res.len(), span)?;
                return Ok(Value::String(res));
            }
            if builtin == Builtin::Split {
                let Value::String(s) = &arguments[0] else {
                    return self.error(span, "split requires a string as first argument");
                };
                let Value::String(delimiter) = &arguments[1] else {
                    return self.error(span, "split requires a string delimiter");
                };
                let raw_parts: Vec<String> = if delimiter.is_empty() {
                    s.chars().map(|c| c.to_string()).collect()
                } else {
                    s.split(delimiter.as_str())
                        .map(|part| part.to_string())
                        .collect()
                };
                self.collection_growth(0, raw_parts.len(), span)?;
                let mut parts = Vec::with_capacity(raw_parts.len());
                for part in raw_parts {
                    self.string_growth(0, part.len(), span)?;
                    parts.push(Value::String(part));
                }
                return Ok(Value::List(parts));
            }
            if builtin == Builtin::Join {
                let Value::List(items) = &arguments[0] else {
                    return self.error(span, "join requires a list as first argument");
                };
                let Value::String(separator) = &arguments[1] else {
                    return self.error(span, "join requires a string separator");
                };
                let mut out = String::new();
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.string_growth(out.len(), separator.len(), span)?;
                        out.push_str(separator);
                    }
                    let mut piece = String::new();
                    format_value_for_display(item, &mut piece, 0);
                    self.string_growth(out.len(), piece.len(), span)?;
                    out.push_str(&piece);
                }
                return Ok(Value::String(out));
            }

            // Explicit region escape hatch: identity at runtime, the promotion
            // itself is automatic; analysis recognizes the wrapper as intent.
            if builtin == Builtin::Promote {
                let mut arguments = arguments.into_owned();
                return Ok(arguments.swap_remove(0));
            }

            // First-class region handles: `arena(name, budget?)` builds a plain
            // descriptor record, `arena_run` executes a callback inside a fresh
            // frame of that region, `arena_stats` reads its aggregated metrics.
            if builtin == Builtin::Arena {
                let mut arguments = arguments.into_owned().into_iter();
                let Some(Value::String(name)) = arguments.next() else {
                    return self.error(span, "arena requires a name string");
                };
                if name.is_empty() {
                    return self.error(span, "arena name must not be empty");
                }
                let budget = match arguments.next() {
                    None | Some(Value::Null) => Value::Null,
                    Some(v) if v.as_usize_index().is_some() => v,
                    _ => {
                        return self.error(span, "arena budget must be a non-negative integer");
                    }
                };
                self.collection_growth(0, 2, span)?;
                return Ok(Value::Record(RecordMap::from([
                    ("name".into(), Value::String(name)),
                    ("budget".into(), budget),
                ])));
            }
            if matches!(builtin, Builtin::ArenaRun | Builtin::ArenaStats) {
                let mut arguments = arguments.into_owned().into_iter();
                let descriptor = arguments.next().unwrap();
                let Value::Record(fields) = &descriptor else {
                    return self.error(span, "an arena record from arena(...) is required");
                };
                let Some(Value::String(name)) = fields.get("name") else {
                    return self.error(span, "arena record requires a name string");
                };
                if builtin == Builtin::ArenaStats {
                    let stats = self
                        .region_stats
                        .iter()
                        .find(|entry| entry.name.as_deref() == Some(name.as_str()));
                    let metric = |pick: fn(&RegionStats) -> u64| {
                        stats.map_or(0.0, |entry| pick(entry) as f64)
                    };
                    self.collection_growth(0, 7, span)?;
                    return Ok(Value::Record(RecordMap::from([
                        ("name".into(), Value::String(name.clone())),
                        ("entries".into(), Value::Number(metric(|s| s.entries))),
                        ("allocated".into(), Value::Number(metric(|s| s.allocated))),
                        (
                            "reused_slots".into(),
                            Value::Number(metric(|s| s.reused_slots)),
                        ),
                        (
                            "peak_live".into(),
                            Value::Number(stats.map_or(0.0, |s| s.peak_live as f64)),
                        ),
                        ("promoted".into(), Value::Number(metric(|s| s.promoted))),
                        (
                            "suggested_budget".into(),
                            Value::Number(stats.map_or(0.0, |s| s.suggested_budget() as f64)),
                        ),
                    ])));
                }
                let callback = arguments.next().unwrap();
                if !matches!(
                    callback,
                    Value::Function(_) | Value::Builtin(_) | Value::Host(_)
                ) {
                    return self.error(span, "arena_run requires a callable");
                }
                let budget = match fields.get("budget") {
                    None | Some(Value::Null) => None,
                    Some(v) => match v.as_usize_index() {
                        Some(b) => Some(b),
                        None => {
                            return self.error(span, "arena budget must be a non-negative integer");
                        }
                    },
                };
                let region = self.enter_region(budget, Some(name));
                // The callback's own implicit call region would intercept its
                // scratch cells before the arena sees them; the arena frame
                // replaces it for exactly this one call.
                self.skip_call_region = true;
                let result = self.call(callback.clone(), Cow::Borrowed(&[]), span);
                self.skip_call_region = false;
                self.exit_region(region, span)?;
                return result;
            }

            if matches!(
                builtin,
                Builtin::Some | Builtin::None | Builtin::Ok | Builtin::Err
            ) {
                let (tag, count) = match builtin {
                    Builtin::Some => ("Some", 1),
                    Builtin::None => ("None", 0),
                    Builtin::Ok => ("Ok", 1),
                    _ => ("Err", 1),
                };
                if arguments.len() != count {
                    return self.error(span, "Invalid variant argument count");
                }
                return Ok(Value::Variant(tag, arguments.into_owned()));
            }

            if builtin == Builtin::Mesh {
                let [Value::List(points), Value::List(faces)] = arguments.as_ref() else {
                    return self.error(span, "mesh requires vertex and triangle lists");
                };
                self.collection_growth(0, points.len(), span)?;
                self.collection_growth(0, faces.len(), span)?;
                let mut vertices = Vec::new();
                for point in points {
                    self.charge(1, span)?;
                    let Value::Vector(v) = point else {
                        return self.error(span, "Mesh vertex must be vec3");
                    };
                    if v.len() != 3 {
                        return self.error(span, "Mesh vertex must be vec3");
                    }
                    vertices.push([v[0], v[1], v[2]]);
                }
                let mut triangles = Vec::new();
                for face in faces {
                    self.charge(1, span)?;
                    let Value::List(indices) = face else {
                        return self.error(span, "Mesh triangle must be a list of three indices");
                    };
                    if indices.len() != 3 {
                        return self.error(span, "Mesh triangle must have three indices");
                    }
                    let mut triangle = [0; 3];
                    for (slot, index) in triangle.iter_mut().zip(indices) {
                        if !matches!(index, Value::Number(_) | Value::Int(_)) {
                            return self.error(span, "Mesh index must be an integer");
                        }
                        let Some(idx) = index.as_usize_index().filter(|&i| i < vertices.len())
                        else {
                            return self.error(span, "Mesh index out of bounds or non-integer");
                        };
                        *slot = idx;
                    }
                    triangles.push(triangle);
                }
                return crate::Mesh::new(vertices, triangles)
                    .map(|mesh| Value::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Transform {
                let [Value::Mesh(mesh), Value::Matrix(matrix)] = arguments.as_ref() else {
                    return self.error(span, "transform requires a mesh and mat4");
                };
                self.collection_growth(0, mesh.vertices().len(), span)?;
                self.collection_growth(0, mesh.triangles().len(), span)?;
                let mut vertices = Vec::new();
                for vertex in mesh.vertices() {
                    self.charge(1, span)?;
                    let transformed = matrix.apply(vertex, true).ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Mesh transformation overflow".into(),
                    })?;
                    vertices.push([transformed[0], transformed[1], transformed[2]]);
                }
                self.charge(mesh.triangles().len(), span)?;
                return crate::Mesh::new(vertices, mesh.triangles().to_vec())
                    .map(|mesh| Value::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::GridMesh {
                let [Value::List(xs), Value::List(ys), callback] = arguments.as_ref() else {
                    return self.error(
                        span,
                        "grid_mesh requires x values, y values and a point function",
                    );
                };
                if xs.len() < 2 || ys.len() < 2 {
                    return self.error(span, "Grid requires at least two coordinates per axis");
                }
                let count = xs.len().checked_mul(ys.len()).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "Grid size overflow".into(),
                })?;
                self.collection_growth(0, count, span)?;
                let faces = (xs.len() - 1)
                    .checked_mul(ys.len() - 1)
                    .and_then(|n| n.checked_mul(2))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Grid size overflow".into(),
                    })?;
                self.collection_growth(0, faces, span)?;
                self.charge(count, span)?;
                let mut vertices = Vec::new();
                for y in ys {
                    for x in xs {
                        let point = self.call(
                            callback.clone(),
                            Cow::Borrowed(&[x.clone(), y.clone()]),
                            span,
                        )?;
                        let Value::Vector(point) = point else {
                            return self.error(span, "Grid callback must return vec3");
                        };
                        if point.len() != 3 {
                            return self.error(span, "Grid callback must return vec3");
                        }
                        vertices.push([point[0], point[1], point[2]]);
                    }
                }
                let mut triangles = Vec::new();
                for row in 0..ys.len() - 1 {
                    for column in 0..xs.len() - 1 {
                        self.charge(2, span)?;
                        let a = row * xs.len() + column;
                        let b = a + 1;
                        let c = a + xs.len();
                        let d = c + 1;
                        triangles.extend([[a, b, d], [a, d, c]]);
                    }
                }
                return crate::Mesh::new(vertices, triangles)
                    .map(|mesh| Value::Mesh(Rc::new(mesh)))
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Zip {
                let [Value::List(a), Value::List(b)] = arguments.as_ref() else {
                    return self.error(span, "zip requires two lists");
                };
                self.collection_growth(0, a.len().min(b.len()), span)?;
                if !a.is_empty() && !b.is_empty() {
                    self.collection_growth(0, 2, span)?;
                }
                let mut values = Vec::new();
                for (a, b) in a.iter().zip(b) {
                    self.check(span)?;
                    if self.remaining == 0 {
                        return self.error(span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    values.push(Value::Tuple(vec![a.clone(), b.clone()]));
                }
                return Ok(Value::List(values));
            }
            if matches!(builtin, Builtin::Range | Builtin::RangeIter) {
                let (start, end, step) = match arguments.as_ref() {
                    [s, e] if s.as_f64().is_some() && e.as_f64().is_some() => {
                        (s.as_f64().unwrap(), e.as_f64().unwrap(), 1.0)
                    }
                    [s, e, st]
                        if s.as_f64().is_some()
                            && e.as_f64().is_some()
                            && st.as_f64().is_some() =>
                    {
                        (
                            s.as_f64().unwrap(),
                            e.as_f64().unwrap(),
                            st.as_f64().unwrap(),
                        )
                    }
                    _ => return self.error(span, "range expects start, end and optional step"),
                };
                if step == 0.0 {
                    return self.error(span, "Range step must be nonzero");
                }
                if !start.is_finite() || !end.is_finite() || !step.is_finite() {
                    return self.error(span, "Range arguments must be finite");
                }
                if builtin == Builtin::RangeIter {
                    return Ok(Value::Range { start, end, step });
                }
                let mut values = Vec::new();
                let mut current = start;
                while if step > 0.0 {
                    current < end
                } else {
                    current > end
                } {
                    self.check(span)?;
                    if self.remaining == 0 {
                        return self.error(span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    if values.len() >= self.max_collection_items {
                        return self.error(span, "Collection item limit exceeded");
                    }
                    values.push(Value::Number(current));
                    let next = (values.len() as f64).mul_add(step, start);
                    if !next.is_finite() || next == current {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    current = next;
                }
                return Ok(Value::List(values));
            }

            if matches!(builtin, Builtin::GroupBy | Builtin::FoldBy) {
                let mut arguments = arguments.into_owned().into_iter();
                let source = arguments.next().unwrap();
                let callback = arguments.next().unwrap();
                if !matches!(
                    callback,
                    Value::Function(_)
                        | Value::BytecodeFunction(_)
                        | Value::Builtin(_)
                        | Value::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let initial = arguments.next().unwrap_or(Value::Null);
                let reducer = arguments.next();
                if reducer.as_ref().is_some_and(|f| {
                    !matches!(
                        f,
                        Value::Function(_)
                            | Value::BytecodeFunction(_)
                            | Value::Builtin(_)
                            | Value::Host(_)
                    )
                }) {
                    return self.error(span, "Reducer must be callable");
                }
                let field = if reducer.is_some() { "value" } else { "values" };
                let mut cursor = self.sequence_cursor(source, span)?;
                let mut positions = BTreeMap::<String, usize>::new();
                let mut groups = Vec::<(String, Value<'s>)>::new();
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let key = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?;
                    let Value::String(key) = key else {
                        return self.error(span, "Group key must be a string");
                    };
                    self.string_growth(0, key.len(), span)?;
                    let position = if let Some(position) = positions.get(&key) {
                        *position
                    } else {
                        self.collection_growth(groups.len(), 1, span)?;
                        self.collection_growth(0, 2, span)?;
                        self.string_growth(0, field.len(), span)?;
                        let position = groups.len();
                        positions.insert(key.clone(), position);
                        groups.push((
                            key,
                            if reducer.is_some() {
                                initial.clone()
                            } else {
                                Value::List(Vec::new())
                            },
                        ));
                        position
                    };
                    if let Some(reducer) = &reducer {
                        let accumulator = std::mem::replace(&mut groups[position].1, Value::Null);
                        groups[position].1 =
                            self.call(reducer.clone(), Cow::Borrowed(&[accumulator, item]), span)?;
                    } else if let Value::List(values) = &mut groups[position].1 {
                        self.collection_growth(values.len(), 1, span)?;
                        values.push(item);
                    }
                }
                let group_shape: RecordShape = Rc::from([String::from("key"), String::from(field)]);
                let mut output = Vec::with_capacity(groups.len());
                for (key, values) in groups {
                    self.charge(1, span)?;
                    output.push(Value::Record(RecordMap::from_shape_and_values(
                        group_shape.clone(),
                        vec![Value::String(key), values],
                    )));
                }
                return Ok(Value::List(output));
            }
            if !matches!(
                builtin,
                Builtin::Map | Builtin::FlatMap | Builtin::Filter | Builtin::Fold
            ) {
                return self.math(builtin, &arguments, span);
            }
            let expected = if builtin == Builtin::Fold { 3 } else { 2 };
            if arguments.len() != expected {
                return self.error(span, "Incorrect argument count");
            }
            let mut arguments = arguments.into_owned().into_iter();
            let source = arguments.next().unwrap();
            let mut cursor = self.sequence_cursor(source, span)?;
            let mut accumulator = if builtin == Builtin::Fold {
                arguments.next().unwrap()
            } else {
                Value::Null
            };
            let callback = arguments.next().unwrap();
            if !matches!(
                callback,
                Value::Function(_)
                    | Value::BytecodeFunction(_)
                    | Value::Builtin(_)
                    | Value::Host(_)
            ) {
                return self.error(span, "Callback must be callable");
            }
            let mut output = Vec::new();
            while let Some(item) = self.sequence_next(&mut cursor, span)? {
                let result = if builtin == Builtin::Fold {
                    self.call(
                        callback.clone(),
                        Cow::Borrowed(&[accumulator, item.clone()]),
                        span,
                    )?
                } else {
                    self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                };
                accumulator = Value::Null;
                match builtin {
                    Builtin::Map => {
                        self.collection_growth(output.len(), 1, span)?;
                        output.push(result);
                    }
                    Builtin::FlatMap => match result {
                        Value::List(values) => {
                            self.charge(values.len(), span)?;
                            self.collection_growth(output.len(), values.len(), span)?;
                            output.extend(values);
                        }
                        _ => return self.error(span, "Flat map callback must return a list"),
                    },
                    Builtin::Filter => match result {
                        Value::Bool(true) => {
                            self.collection_growth(output.len(), 1, span)?;
                            output.push(item);
                        }
                        Value::Bool(false) => {}
                        _ => return self.error(span, "Filter callback must return a boolean"),
                    },
                    Builtin::Fold => accumulator = result,
                    _ => unreachable!(),
                }
            }
            return if builtin == Builtin::Fold {
                Ok(accumulator)
            } else {
                Ok(Value::List(output))
            };
        }
        self.error(span, "Value is not callable")
    }

    pub(super) fn constructor(
        &self,
        name: &Name<'s>,
        type_name: String,
        variant: Option<String>,
        mut fields: Vec<(String, ValueType)>,
        types: Vec<ValueType>,
    ) -> Result<Value<'s>> {
        fields.sort_by(|(a, _), (b, _)| a.cmp(b));
        let shape: RecordShape =
            Rc::from(fields.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>());
        let count = if variant.is_none() { 1 } else { types.len() };
        Ok(Value::Function(Rc::new(Closure {
            module: self.module,
            name: Some(name.text),
            parameters: (0..count)
                .map(|_| Expr {
                    span: name.span,
                    kind: ExprKind::Name(Name {
                        text: "_",
                        span: name.span,
                    }),
                })
                .collect(),
            parameter_types: if variant.is_none() {
                vec![None]
            } else {
                types.into_iter().map(Some).collect()
            },
            result_type: Some(ValueType::User(type_name.clone())),
            body: Rc::new(FunctionBody::Constructor {
                type_name,
                variant,
                shape,
                fields,
            }),
            environment: memory::Shared::new(&self.memory, Environment::new(&self.memory))
                .map_err(|e| self.environment_error(name.span, e))?,
            references: Rc::new(Vec::new()),
            siblings: Rc::new(Vec::new()),
        })))
    }

    pub(super) fn call_constructor(
        &mut self,
        function: &Closure<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        if let FunctionBody::Constructor {
            type_name,
            variant,
            shape,
            fields,
        } = &*function.body
        {
            let values = if variant.is_none() {
                let mut owned_args = arguments.into_owned();
                let Some(Value::Record(mut supplied)) = owned_args.pop() else {
                    return self.error(span, "Struct constructor requires a record");
                };
                if supplied.len() != fields.len()
                    || (!Rc::ptr_eq(supplied.shape(), shape) && supplied.keys_slice() != &**shape)
                    || supplied
                        .values_slice()
                        .iter()
                        .zip(fields)
                        .any(|(value, (_, ty))| !ty.accepts(value))
                {
                    return self.error(span, "Struct fields do not match declaration");
                }
                for (val, (_, ty)) in supplied.values_mut_slice().iter_mut().zip(fields) {
                    let v = std::mem::replace(val, Value::Null);
                    *val = v.coerce_for_type(ty);
                }
                supplied.set_shape(shape.clone());
                vec![Value::Record(supplied)]
            } else {
                arguments
                    .into_owned()
                    .into_iter()
                    .zip(&function.parameter_types)
                    .map(|(v, ty)| match ty {
                        Some(ty) => v.coerce_for_type(ty),
                        None => v,
                    })
                    .collect()
            };
            self.collection_growth(
                0,
                if variant.is_none() {
                    fields.len()
                } else {
                    values.len()
                },
                span,
            )?;
            for value in values.iter() {
                self.host_value_size(value, span, 0)?;
            }
            return Ok(Value::UserData(Box::new(UserData {
                type_name: type_name.clone(),
                variant: variant.clone(),
                values,
            })));
        }
        unreachable!("constructor body")
    }

    pub(super) fn call_user(
        &mut self,
        function: Rc<Closure<'s>>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        // Consumed upfront so no early return can leak the one-shot flag.
        let skip_call_region = std::mem::take(&mut self.skip_call_region);
        if arguments.len() != function.parameters.len() {
            return self.error(span, "Incorrect argument count");
        }
        if function
            .parameter_types
            .iter()
            .zip(arguments.iter())
            .any(|(ty, value)| ty.as_ref().is_some_and(|ty| !ty.accepts(value)))
        {
            return self.error(span, "Argument does not match its type annotation");
        }
        if matches!(&*function.body, FunctionBody::Constructor { .. }) {
            return self.call_constructor(&function, arguments, span);
        }
        let mut environment = self.take_call_environment(&function.environment);
        for sib in function.siblings.iter() {
            if sib.name == function.name {
                continue;
            }
            if let Some(sib_name) = sib.name {
                let mut bound = sib.clone();
                bound.siblings = function.siblings.clone();
                if let Err(allocation) =
                    environment.insert(sib_name, Value::Function(Rc::new(bound)))
                {
                    let error = self.environment_error(span, allocation);
                    self.recycle_call_environment(environment);
                    return Err(error);
                }
            }
        }
        if let Some(name) = function.name
            && let Err(allocation) = environment.insert(name, Value::Function(function.clone()))
        {
            let error = self.environment_error(span, allocation);
            self.recycle_call_environment(environment);
            return Err(error);
        }
        let previous_module = self.module;
        let previous_references =
            std::mem::replace(&mut self.current_references, function.references.clone());
        self.module = function.module;
        for ((parameter, param_ty), argument) in function
            .parameters
            .iter()
            .zip(&function.parameter_types)
            .zip(arguments.iter())
        {
            let coerced_arg: Cow<'_, Value<'s>> = match param_ty {
                Some(ty) => Cow::Owned(argument.clone().coerce_for_type(ty)),
                None => Cow::Borrowed(argument),
            };
            if let Err(error) = self.bind_pattern(parameter, &coerced_arg, &mut environment) {
                self.module = previous_module;
                self.current_references = previous_references;
                self.recycle_call_environment(environment);
                return Err(error);
            }
        }
        if matches!(&*function.body, FunctionBody::Block(_)) && self.depth >= self.max_depth {
            self.module = previous_module;
            self.current_references = previous_references;
            self.recycle_call_environment(environment);
            return self.error(span, "Execution limit exceeded");
        }
        // Implicit per-call region: temporary cells die in bulk when the call
        // returns, while captured ones promote into the caller's region.
        // Skipped when arena_run already supplied this call's region.
        let call_region = if skip_call_region {
            None
        } else {
            Some(self.enter_region(None, None))
        };
        let result = match &*function.body {
            FunctionBody::Constructor { .. } => unreachable!("constructors return above"),
            FunctionBody::Expression(body) => self.expr(body, &environment),
            FunctionBody::Block(body) => {
                self.depth += 1;
                let result =
                    self.statements(&body.stmts, &mut environment)
                        .map(|(value, returned)| {
                            if returned == Flow::Return {
                                value
                            } else {
                                Value::Null
                            }
                        });
                self.depth -= 1;
                result
            }
        };
        let result = if result.is_err() {
            if let Some(value) = self.early_return.take() {
                Ok(value)
            } else {
                result
            }
        } else {
            result
        };
        self.module = previous_module;
        self.current_references = previous_references;
        self.recycle_call_environment(environment);
        // exit_region reclaims dead cells internally before sweeping; without
        // a call region (arena_run owns it) reclaim explicitly.
        if let Some(call_region) = call_region {
            self.exit_region(call_region, span)?;
        } else {
            self.reclaim_cells();
        }
        let mut value = result?;
        if let Some(ty) = &function.result_type {
            if !ty.accepts(&value) {
                return self.error(span, "Return value does not match its type annotation");
            }
            value = value.coerce_for_type(ty);
        }
        Ok(value)
    }
}
