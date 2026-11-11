use super::Vm;
use crate::Value;

impl<'c> Vm<'c> {
    #[inline]
    pub(super) fn read_index(
        obj: &Value<'static>,
        idx: &Value<'static>,
    ) -> Result<Value<'static>, String> {
        match (obj, idx) {
            (Value::List(items) | Value::Tuple(items), _) => {
                if let Some(i) = idx.as_usize_index()
                    && i < items.len()
                {
                    Ok(items[i].clone())
                } else {
                    Err("Index out of bounds".to_string())
                }
            }
            (Value::Vector(components), _) => {
                if let Some(i) = idx.as_usize_index()
                    && i < components.len()
                {
                    Ok(Value::Number(components[i]))
                } else {
                    Err("Index out of bounds".to_string())
                }
            }
            (Value::Record(fields), Value::String(k)) => fields
                .get(k)
                .cloned()
                .ok_or_else(|| format!("Field '{k}' not found")),
            _ => Err("Cannot index value".to_string()),
        }
    }

    #[inline]
    pub(super) fn read_member(
        obj: &Value<'static>,
        field_name: &str,
        ic_cell: &std::cell::Cell<u32>,
    ) -> Result<Value<'static>, String> {
        match obj {
            Value::Record(fields) => {
                let hint = ic_cell.get();
                if let Some((slot, val)) = fields.get_with_hint(field_name, hint) {
                    if slot != hint {
                        ic_cell.set(slot);
                    }
                    Ok(val.clone())
                } else {
                    Err(format!("Field '{field_name}' not found"))
                }
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first() {
                    let hint = ic_cell.get();
                    if let Some((slot, val)) = fields.get_with_hint(field_name, hint) {
                        if slot != hint {
                            ic_cell.set(slot);
                        }
                        Ok(val.clone())
                    } else {
                        Err(format!("Field '{field_name}' not found"))
                    }
                } else {
                    Err("Cannot access member on value".to_string())
                }
            }
            Value::Vector(v) => {
                let Some(swizzle) = crate::catalog::parse_vector_swizzle(field_name) else {
                    return Err(format!("Vector field '{field_name}' not found"));
                };
                if swizzle.max_index() >= v.len() {
                    return Err(format!("Vector field '{field_name}' out of bounds"));
                }
                if swizzle.len == 1 {
                    Ok(Value::Number(v[swizzle.indices[0] as usize]))
                } else {
                    let mut buf = [0.0f64; 4];
                    for (i, slot) in buf.iter_mut().take(swizzle.len as usize).enumerate() {
                        *slot = v[swizzle.indices[i] as usize];
                    }
                    Ok(Value::Vector(crate::InlineVec::from_slice(
                        &buf[..swizzle.len as usize],
                    )))
                }
            }
            _ => Err("Cannot access member on value".to_string()),
        }
    }

    #[inline]
    pub(super) fn mutate_index(
        container: &mut Value<'static>,
        idx: &Value<'static>,
        val: Value<'static>,
    ) -> Option<&'static str> {
        match (container, idx) {
            (Value::List(items), _) => {
                if let Some(i) = idx.as_usize_index() {
                    if i < items.len() {
                        items[i] = val;
                        None
                    } else {
                        Some("Index out of bounds")
                    }
                } else if matches!(idx, Value::Int(_) | Value::Number(_)) {
                    Some("Index out of bounds")
                } else {
                    Some("Index must be an integer")
                }
            }
            (Value::Record(fields), Value::String(k)) => {
                if let Some(slot) = fields.get_mut(k.as_str()) {
                    *slot = val;
                } else {
                    fields.insert(k.clone(), val);
                }
                None
            }
            (Value::Record(_), _) => Some("Record index must be a string"),
            (Value::Vector(components), _) => {
                if let Some(i) = idx.as_usize_index() {
                    if i < components.len() {
                        let Some(num) = val.as_f64() else {
                            return Some("Vector component must be a number");
                        };
                        if !num.is_finite() {
                            return Some("Non-finite vector result");
                        }
                        components[i] = num;
                        None
                    } else {
                        Some("Index out of bounds")
                    }
                } else if matches!(idx, Value::Int(_) | Value::Number(_)) {
                    Some("Index out of bounds")
                } else {
                    Some("Index must be an integer")
                }
            }
            _ => Some("Indexing requires a list, record or vector"),
        }
    }

    #[inline]
    pub(super) fn mutate_member(
        container: &mut Value<'static>,
        field_name: &str,
        val: Value<'static>,
        ic_cell: Option<&std::cell::Cell<u32>>,
    ) -> Option<&'static str> {
        match container {
            Value::Record(fields) => {
                let hint = ic_cell.map_or(u32::MAX, std::cell::Cell::get);
                if let Some((slot_idx, slot)) = fields.get_mut_with_hint(field_name, hint) {
                    if let Some(cell) = ic_cell
                        && slot_idx != hint
                    {
                        cell.set(slot_idx);
                    }
                    *slot = val;
                } else {
                    fields.insert(field_name.to_owned(), val);
                }
                None
            }
            Value::UserData(data) if data.variant.is_none() => {
                if let Some(Value::Record(fields)) = data.values.first_mut() {
                    let hint = ic_cell.map_or(u32::MAX, std::cell::Cell::get);
                    let Some((slot_idx, slot)) = fields.get_mut_with_hint(field_name, hint) else {
                        return Some("Unknown struct field");
                    };
                    if let Some(cell) = ic_cell
                        && slot_idx != hint
                    {
                        cell.set(slot_idx);
                    }
                    *slot = val;
                    None
                } else {
                    Some("Member access requires a struct or record")
                }
            }
            Value::Vector(components) => {
                let Some(swizzle) = crate::catalog::parse_vector_swizzle(field_name) else {
                    return Some("Unknown vector component");
                };
                if swizzle.max_index() >= components.len() {
                    return Some("Vector component out of bounds");
                }
                if swizzle.has_duplicates() {
                    return Some("Vector swizzle assignment cannot repeat components");
                }
                if swizzle.len == 1 {
                    let Some(num) = val.as_f64() else {
                        return Some("Vector component must be a number");
                    };
                    if !num.is_finite() {
                        return Some("Non-finite vector result");
                    }
                    components[swizzle.indices[0] as usize] = num;
                    None
                } else {
                    let Value::Vector(rhs) = val else {
                        return Some("Vector swizzle assignment requires matching vector");
                    };
                    if rhs.len() != swizzle.len as usize {
                        return Some("Vector swizzle assignment requires matching vector");
                    }
                    if rhs.iter().any(|v| !v.is_finite()) {
                        return Some("Non-finite vector result");
                    }
                    for i in 0..swizzle.len as usize {
                        components[swizzle.indices[i] as usize] = rhs[i];
                    }
                    None
                }
            }
            _ => Some("Member access requires a record, struct or vector"),
        }
    }

    pub(super) fn read_path_vm(
        current: &Value<'static>,
        keys: &[Value<'static>],
        member_mask: u32,
        step_idx: usize,
    ) -> Result<Value<'static>, &'static str> {
        let Some((head, rest)) = keys.split_first() else {
            return Ok(current.clone());
        };
        let is_member = (member_mask & (1u32 << step_idx)) != 0;
        if is_member {
            let Value::String(field_name) = head else {
                return Err("Invalid member name");
            };
            match current {
                Value::Record(fields) => {
                    let sub = fields.get(field_name).ok_or("Unknown field")?;
                    Self::read_path_vm(sub, rest, member_mask, step_idx + 1)
                }
                Value::UserData(data) if data.variant.is_none() => {
                    let Some(Value::Record(fields)) = data.values.first() else {
                        return Err("Member access requires a struct or record");
                    };
                    let sub = fields.get(field_name).ok_or("Unknown struct field")?;
                    Self::read_path_vm(sub, rest, member_mask, step_idx + 1)
                }
                Value::Vector(components) => {
                    let Some(swizzle) = crate::catalog::parse_vector_swizzle(field_name.as_str())
                    else {
                        return Err("Unknown vector component");
                    };
                    if swizzle.max_index() >= components.len() {
                        return Err("Vector component out of bounds");
                    }
                    if !rest.is_empty() {
                        return Err("Vector components do not support nested access");
                    }
                    if swizzle.len == 1 {
                        Ok(Value::Number(components[swizzle.indices[0] as usize]))
                    } else {
                        let mut buf = [0.0f64; 4];
                        for (i, slot) in buf.iter_mut().take(swizzle.len as usize).enumerate() {
                            *slot = components[swizzle.indices[i] as usize];
                        }
                        Ok(Value::Vector(crate::InlineVec::from_slice(
                            &buf[..swizzle.len as usize],
                        )))
                    }
                }
                _ => Err("Member access requires a record, struct or vector"),
            }
        } else {
            match (current, head) {
                (Value::List(items) | Value::Tuple(items), _) => {
                    if let Some(i) = head.as_usize_index() {
                        if i < items.len() {
                            Self::read_path_vm(&items[i], rest, member_mask, step_idx + 1)
                        } else {
                            Err("Index out of bounds")
                        }
                    } else if matches!(head, Value::Int(_) | Value::Number(_)) {
                        Err("Index out of bounds")
                    } else {
                        Err("Index must be an integer")
                    }
                }
                (Value::Record(fields), Value::String(k)) => {
                    let sub = fields.get(k).ok_or("Unknown field")?;
                    Self::read_path_vm(sub, rest, member_mask, step_idx + 1)
                }
                (Value::Record(_), _) => Err("Record index must be a string"),
                (Value::Vector(components), _) => {
                    if let Some(i) = head.as_usize_index() {
                        if i < components.len() {
                            if rest.is_empty() {
                                Ok(Value::Number(components[i]))
                            } else {
                                Err("Vector components do not support nested access")
                            }
                        } else {
                            Err("Index out of bounds")
                        }
                    } else if matches!(head, Value::Int(_) | Value::Number(_)) {
                        Err("Index out of bounds")
                    } else {
                        Err("Index must be an integer")
                    }
                }
                _ => Err("Indexing requires a list, record or vector"),
            }
        }
    }

    pub(super) fn mutate_path_vm(
        current: &mut Value<'static>,
        keys: &[Value<'static>],
        member_mask: u32,
        step_idx: usize,
        val: Value<'static>,
    ) -> Option<&'static str> {
        let Some((head, rest)) = keys.split_first() else {
            *current = val;
            return None;
        };
        let is_member = (member_mask & (1u32 << step_idx)) != 0;
        if rest.is_empty() {
            if is_member {
                let Value::String(field_name) = head else {
                    return Some("Invalid member name");
                };
                return Self::mutate_member(current, field_name.as_str(), val, None);
            } else {
                return Self::mutate_index(current, head, val);
            }
        }
        if is_member {
            let Value::String(field_name) = head else {
                return Some("Invalid member name");
            };
            match current {
                Value::Record(fields) => {
                    let Some(sub) = fields.get_mut(field_name) else {
                        return Some("Unknown field");
                    };
                    Self::mutate_path_vm(sub, rest, member_mask, step_idx + 1, val)
                }
                Value::UserData(data) if data.variant.is_none() => {
                    let Some(Value::Record(fields)) = data.values.first_mut() else {
                        return Some("Member access requires a struct or record");
                    };
                    let Some(sub) = fields.get_mut(field_name) else {
                        return Some("Unknown struct field");
                    };
                    Self::mutate_path_vm(sub, rest, member_mask, step_idx + 1, val)
                }
                Value::Vector(_) => Some("Vector components do not support nested access"),
                _ => Some("Member access requires a record, struct or vector"),
            }
        } else {
            match (current, head) {
                (Value::List(items), _) => {
                    if let Some(i) = head.as_usize_index() {
                        if i < items.len() {
                            Self::mutate_path_vm(
                                &mut items[i],
                                rest,
                                member_mask,
                                step_idx + 1,
                                val,
                            )
                        } else {
                            Some("Index out of bounds")
                        }
                    } else if matches!(head, Value::Int(_) | Value::Number(_)) {
                        Some("Index out of bounds")
                    } else {
                        Some("Index must be an integer")
                    }
                }
                (Value::Record(fields), Value::String(k)) => {
                    let Some(sub) = fields.get_mut(k) else {
                        return Some("Unknown field");
                    };
                    Self::mutate_path_vm(sub, rest, member_mask, step_idx + 1, val)
                }
                (Value::Record(_), _) => Some("Record index must be a string"),
                (Value::Vector(_), Value::Int(_) | Value::Number(_)) => {
                    Some("Vector components do not support nested access")
                }
                (Value::Vector(_), _) => Some("Index must be an integer"),
                _ => Some("Indexing requires a list, record or vector"),
            }
        }
    }
}
