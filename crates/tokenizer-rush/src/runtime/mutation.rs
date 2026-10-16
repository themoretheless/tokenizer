use super::{Binding, Environment, Result, Runtime, RuntimeError, Value, ValueType};
use crate::{Expr, ExprKind};
use std::collections::{BTreeMap, HashMap};
use themoretheless_tokenizer_core::Span;

pub(super) enum AccessStep<'s> {
    Index(Value<'s>, Span),
    Member(&'s str, Span),
}

pub(super) fn path_error(
    module: Option<&str>,
    span: Span,
    message: impl Into<String>,
) -> RuntimeError {
    RuntimeError {
        stack: Vec::new(),
        location: None,
        module: module.map(str::to_owned),
        span,
        message: message.into(),
    }
}

pub(super) fn read_path<'s>(
    module: Option<&str>,
    current: &Value<'s>,
    steps: &[AccessStep<'s>],
) -> Result<Value<'s>> {
    if steps.is_empty() {
        return Ok(current.clone());
    }
    let (head, rest) = steps.split_first().unwrap();
    match head {
        AccessStep::Index(index_val, step_span) => match current {
            Value::List(items) | Value::Tuple(items) => {
                if !matches!(index_val, Value::Number(_) | Value::Int(_)) {
                    return Err(path_error(module, *step_span, "Index must be an integer"));
                }
                let Some(idx) = index_val.as_usize_index().filter(|&i| i < items.len()) else {
                    return Err(path_error(module, *step_span, "Index out of bounds"));
                };
                read_path(module, &items[idx], rest)
            }
            Value::Record(fields) => {
                let Value::String(key) = index_val else {
                    return Err(path_error(
                        module,
                        *step_span,
                        "Record index must be a string",
                    ));
                };
                let sub = fields.get(key).ok_or_else(|| {
                    path_error(module, *step_span, format!("Unknown field: {key}"))
                })?;
                read_path(module, sub, rest)
            }
            Value::Vector(components) => {
                if !matches!(index_val, Value::Number(_) | Value::Int(_)) {
                    return Err(path_error(module, *step_span, "Index must be an integer"));
                }
                let Some(idx) = index_val.as_usize_index().filter(|&i| i < components.len()) else {
                    return Err(path_error(module, *step_span, "Index out of bounds"));
                };
                if rest.is_empty() {
                    Ok(Value::Number(components[idx]))
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Vector components do not support nested access",
                    ))
                }
            }
            _ => Err(path_error(
                module,
                *step_span,
                "Indexing requires a list, record or vector",
            )),
        },
        AccessStep::Member(field_name, step_span) => match current {
            Value::Record(fields) => {
                let sub = fields.get(field_name).ok_or_else(|| {
                    path_error(module, *step_span, format!("Unknown field: {field_name}"))
                })?;
                read_path(module, sub, rest)
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first() {
                    let sub = fields.get(field_name).ok_or_else(|| {
                        path_error(
                            module,
                            *step_span,
                            format!("Unknown struct field: {field_name}"),
                        )
                    })?;
                    read_path(module, sub, rest)
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Member access requires a struct or record",
                    ))
                }
            }
            Value::Vector(components) => {
                let axis = match *field_name {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return Err(path_error(module, *step_span, "Unknown vector component")),
                };
                if axis >= components.len() {
                    return Err(path_error(
                        module,
                        *step_span,
                        "Vector component out of bounds",
                    ));
                }
                if rest.is_empty() {
                    Ok(Value::Number(components[axis]))
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Vector components do not support nested access",
                    ))
                }
            }
            _ => Err(path_error(
                module,
                *step_span,
                "Member access requires a record, struct or vector",
            )),
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn mutate_path<'s>(
    module: Option<&str>,
    struct_fields: &HashMap<String, BTreeMap<String, ValueType>>,
    max_collection_items: usize,
    max_string_bytes: usize,
    current: &mut Value<'s>,
    steps: &[AccessStep<'s>],
    assigned: Value<'s>,
    span: Span,
) -> Result<()> {
    if steps.is_empty() {
        *current = assigned;
        return Ok(());
    }
    let (head, rest) = steps.split_first().unwrap();
    match head {
        AccessStep::Index(index_val, step_span) => match current {
            Value::List(items) => {
                if !matches!(index_val, Value::Number(_) | Value::Int(_)) {
                    return Err(path_error(module, *step_span, "Index must be an integer"));
                }
                let Some(idx) = index_val.as_usize_index().filter(|&i| i < items.len()) else {
                    return Err(path_error(module, *step_span, "Index out of bounds"));
                };
                if rest.is_empty() {
                    items[idx] = assigned;
                    Ok(())
                } else {
                    mutate_path(
                        module,
                        struct_fields,
                        max_collection_items,
                        max_string_bytes,
                        &mut items[idx],
                        rest,
                        assigned,
                        span,
                    )
                }
            }
            Value::Record(fields) => {
                let Value::String(key) = index_val else {
                    return Err(path_error(
                        module,
                        *step_span,
                        "Record index must be a string",
                    ));
                };
                if rest.is_empty() {
                    if let Some(slot) = fields.get_mut(key) {
                        *slot = assigned;
                    } else {
                        if fields.len().saturating_add(1) > max_collection_items {
                            return Err(path_error(
                                module,
                                span,
                                "Collection size exceeds execution limit",
                            ));
                        }
                        fields.insert(key.clone(), assigned);
                    }
                    Ok(())
                } else {
                    let sub = fields.get_mut(key).ok_or_else(|| {
                        path_error(module, *step_span, format!("Unknown field: {key}"))
                    })?;
                    mutate_path(
                        module,
                        struct_fields,
                        max_collection_items,
                        max_string_bytes,
                        sub,
                        rest,
                        assigned,
                        span,
                    )
                }
            }
            Value::Vector(components) => {
                if !matches!(index_val, Value::Number(_) | Value::Int(_)) {
                    return Err(path_error(module, *step_span, "Index must be an integer"));
                }
                let Some(idx) = index_val.as_usize_index().filter(|&i| i < components.len()) else {
                    return Err(path_error(module, *step_span, "Index out of bounds"));
                };
                if rest.is_empty() {
                    let Some(val) = assigned.as_f64() else {
                        return Err(path_error(
                            module,
                            span,
                            "Vector component must be a number",
                        ));
                    };
                    if !val.is_finite() {
                        return Err(path_error(module, span, "Non-finite vector result"));
                    }
                    components[idx] = val;
                    Ok(())
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Vector components do not support nested access",
                    ))
                }
            }
            _ => Err(path_error(
                module,
                *step_span,
                "Indexing requires a list, record or vector",
            )),
        },
        AccessStep::Member(field_name, step_span) => match current {
            Value::Record(fields) => {
                if rest.is_empty() {
                    if let Some(slot) = fields.get_mut(field_name) {
                        *slot = assigned;
                    } else {
                        if field_name.len() > max_string_bytes {
                            return Err(path_error(
                                module,
                                *step_span,
                                "String length exceeds execution limit",
                            ));
                        }
                        if fields.len().saturating_add(1) > max_collection_items {
                            return Err(path_error(
                                module,
                                span,
                                "Collection size exceeds execution limit",
                            ));
                        }
                        fields.insert((*field_name).to_owned(), assigned);
                    }
                    Ok(())
                } else {
                    let sub = fields.get_mut(field_name).ok_or_else(|| {
                        path_error(module, *step_span, format!("Unknown field: {field_name}"))
                    })?;
                    mutate_path(
                        module,
                        struct_fields,
                        max_collection_items,
                        max_string_bytes,
                        sub,
                        rest,
                        assigned,
                        span,
                    )
                }
            }
            Value::UserData(data) if data.variant.is_none() => {
                let super::UserData {
                    type_name, values, ..
                } = &mut **data;
                if let Some(Value::Record(fields)) = values.first_mut() {
                    if rest.is_empty() {
                        let Some(slot) = fields.get_mut(field_name) else {
                            return Err(path_error(
                                module,
                                *step_span,
                                format!("Unknown struct field: {field_name}"),
                            ));
                        };
                        let mut assigned = assigned;
                        if let Some(expected_ty) = struct_fields
                            .get(type_name.as_str())
                            .and_then(|f| f.get(*field_name))
                        {
                            if !expected_ty.accepts(&assigned) {
                                return Err(path_error(
                                    module,
                                    span,
                                    "Assignment violates variable type",
                                ));
                            }
                            assigned = assigned.coerce_for_type(expected_ty);
                        }
                        *slot = assigned;
                        Ok(())
                    } else {
                        let sub = fields.get_mut(field_name).ok_or_else(|| {
                            path_error(
                                module,
                                *step_span,
                                format!("Unknown struct field: {field_name}"),
                            )
                        })?;
                        mutate_path(
                            module,
                            struct_fields,
                            max_collection_items,
                            max_string_bytes,
                            sub,
                            rest,
                            assigned,
                            span,
                        )?;
                        if let Some(expected_ty) = struct_fields
                            .get(type_name.as_str())
                            .and_then(|f| f.get(*field_name))
                            && !expected_ty.accepts(sub)
                        {
                            return Err(path_error(
                                module,
                                span,
                                "Assignment violates variable type",
                            ));
                        }
                        Ok(())
                    }
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Member access requires a struct or record",
                    ))
                }
            }
            Value::Vector(components) => {
                let axis = match *field_name {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return Err(path_error(module, *step_span, "Unknown vector component")),
                };
                if axis >= components.len() {
                    return Err(path_error(
                        module,
                        *step_span,
                        "Vector component out of bounds",
                    ));
                }
                if rest.is_empty() {
                    let Some(val) = assigned.as_f64() else {
                        return Err(path_error(
                            module,
                            span,
                            "Vector component must be a number",
                        ));
                    };
                    if !val.is_finite() {
                        return Err(path_error(module, span, "Non-finite vector result"));
                    }
                    components[axis] = val;
                    Ok(())
                } else {
                    Err(path_error(
                        module,
                        *step_span,
                        "Vector components do not support nested access",
                    ))
                }
            }
            _ => Err(path_error(
                module,
                *step_span,
                "Member access requires a record, struct or vector",
            )),
        },
    }
}

impl<'s> Runtime<'_, 's> {
    pub(super) fn resolve_lvalue_path(
        &mut self,
        expr: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<(usize, Vec<AccessStep<'s>>)> {
        match &expr.kind {
            ExprKind::Name(name) => {
                let Some(Binding::Cell(cell)) = environment.get(name.text) else {
                    return self.error(name.span, "Assignment requires a mutable variable");
                };
                let index = self.cell_index(cell, name.span)?;
                Ok((index, Vec::new()))
            }
            ExprKind::Index { object, index } => {
                let (root, mut steps) = self.resolve_lvalue_path(object, environment)?;
                let index_val = self.expr(index, environment)?;
                steps.push(AccessStep::Index(index_val, index.span));
                Ok((root, steps))
            }
            ExprKind::Member { object, field } => {
                let (root, mut steps) = self.resolve_lvalue_path(object, environment)?;
                steps.push(AccessStep::Member(field.text, field.span));
                Ok((root, steps))
            }
            _ => self.error(expr.span, "Assignment requires a variable, member or index"),
        }
    }
}
