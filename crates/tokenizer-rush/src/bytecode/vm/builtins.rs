use super::Vm;
use crate::bytecode::iterator::{
    BytecodeIterator, BytecodeSequence, BytecodeSequenceSource, BytecodeSequenceStage,
    BytecodeStageKind,
};
use crate::runtime::builtins::eval_pure_builtin;
use crate::runtime::memory::Cover;
use crate::runtime::ops::is_truthy;
use crate::{Builtin, RecordMap, RuntimeError, Value};
use std::collections::BTreeMap;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

#[inline(always)]
pub(super) const fn is_higher_order_builtin(b: Builtin) -> bool {
    matches!(
        b,
        Builtin::Map
            | Builtin::Filter
            | Builtin::Fold
            | Builtin::Range
            | Builtin::RangeIter
            | Builtin::Iter
            | Builtin::Collect
            | Builtin::FlatMap
            | Builtin::Any
            | Builtin::All
            | Builtin::Zip
            | Builtin::GroupBy
            | Builtin::FoldBy
    )
}

impl<'c> Vm<'c> {
    pub fn invoke_callable(
        &mut self,
        callable: &Value<'static>,
        args: &[Value<'static>],
        span: Span,
    ) -> Result<Value<'static>, RuntimeError> {
        match callable {
            Value::BytecodeFunction(closure) => self.run_closure(closure, args),
            Value::Builtin(b) => self.call_builtin(*b, args, span),
            _ => Err(self.error(span, "Value is not callable")),
        }
    }

    pub(super) fn call_builtin(
        &mut self,
        builtin: Builtin,
        args: &[Value<'static>],
        span: Span,
    ) -> Result<Value<'static>, RuntimeError> {
        match builtin {
            Builtin::Map => {
                if args.len() != 2 {
                    return Err(self.error(span, "map requires 2 arguments"));
                }
                let (source, callback) = (&args[0], &args[1]);
                match source {
                    Value::Range { start, end, step } => {
                        let seq = BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: vec![BytecodeSequenceStage {
                                callback: callback.clone(),
                                kind: BytecodeStageKind::Map,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            kind: BytecodeStageKind::Map,
                            span,
                        });
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: seq.source.clone(),
                            stages,
                        })))
                    }
                    Value::List(items) => {
                        let mut output = Vec::with_capacity(items.len());
                        let mut cover = Cover::new(&self.memory);
                        for item in items {
                            let res =
                                self.invoke_callable(callback, std::slice::from_ref(item), span)?;
                            output.push(res);
                            cover
                                .cover_vec(&output)
                                .map_err(|e| self.cover_error(span, e))?;
                        }
                        Ok(Value::List(output))
                    }
                    _ => Err(self.error(span, "map requires a list or sequence and a callback")),
                }
            }
            Builtin::FlatMap => {
                if args.len() != 2 {
                    return Err(self.error(span, "flat_map requires a list and a callback"));
                }
                let (source, callback) = (&args[0], &args[1]);
                match source {
                    Value::Range { start, end, step } => {
                        let seq = BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: vec![BytecodeSequenceStage {
                                callback: callback.clone(),
                                kind: BytecodeStageKind::FlatMap,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            kind: BytecodeStageKind::FlatMap,
                            span,
                        });
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: seq.source.clone(),
                            stages,
                        })))
                    }
                    Value::List(items) => {
                        let mut output = Vec::new();
                        let mut cover = Cover::new(&self.memory);
                        for item in items {
                            let Value::List(values) =
                                self.invoke_callable(callback, std::slice::from_ref(item), span)?
                            else {
                                return Err(
                                    self.error(span, "flat_map callback must return a list")
                                );
                            };
                            output.extend(values);
                            cover
                                .cover_vec(&output)
                                .map_err(|e| self.cover_error(span, e))?;
                        }
                        Ok(Value::List(output))
                    }
                    _ => {
                        Err(self.error(span, "flat_map requires a list or sequence and a callback"))
                    }
                }
            }
            Builtin::Filter => {
                if args.len() != 2 {
                    return Err(self.error(span, "filter requires 2 arguments"));
                }
                let (source, callback) = (&args[0], &args[1]);
                match source {
                    Value::Range { start, end, step } => {
                        let seq = BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: vec![BytecodeSequenceStage {
                                callback: callback.clone(),
                                kind: BytecodeStageKind::Filter,
                                span,
                            }],
                        };
                        Ok(Value::BytecodeSequence(Rc::new(seq)))
                    }
                    Value::BytecodeSequence(seq) => {
                        let mut stages = seq.stages.clone();
                        stages.push(BytecodeSequenceStage {
                            callback: callback.clone(),
                            kind: BytecodeStageKind::Filter,
                            span,
                        });
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: seq.source.clone(),
                            stages,
                        })))
                    }
                    Value::List(items) => {
                        let mut output = Vec::new();
                        let mut cover = Cover::new(&self.memory);
                        for item in items {
                            let res =
                                self.invoke_callable(callback, std::slice::from_ref(item), span)?;
                            if is_truthy(&res) {
                                output.push(item.clone());
                                cover
                                    .cover_vec(&output)
                                    .map_err(|e| self.cover_error(span, e))?;
                            }
                        }
                        Ok(Value::List(output))
                    }
                    _ => Err(self.error(span, "filter requires a list or sequence and a callback")),
                }
            }
            Builtin::Fold => {
                if args.len() != 3 {
                    return Err(self.error(span, "fold requires 3 arguments"));
                }
                let (source, initial, callback) = (&args[0], &args[1], &args[2]);
                match source {
                    Value::List(items) => {
                        let mut acc = initial.clone();
                        for item in items {
                            acc = self.invoke_callable(callback, &[acc, item.clone()], span)?;
                        }
                        Ok(acc)
                    }
                    Value::Range { .. } | Value::BytecodeSequence(_) => {
                        let mut iter = BytecodeIterator::new(source, span)?;
                        let mut acc = initial.clone();
                        while let Some(item) = iter.next(self, span)? {
                            acc = self.invoke_callable(callback, &[acc, item], span)?;
                        }
                        Ok(acc)
                    }
                    _ => Err(self.error(
                        span,
                        "fold requires a list, an initial value and a callback",
                    )),
                }
            }
            Builtin::Any | Builtin::All => {
                if args.len() != 2 {
                    return Err(self.error(span, "any and all require a list and a callback"));
                }
                let (Value::List(items), callback) = (&args[0], &args[1]) else {
                    return Err(self.error(span, "any and all require a list and a callback"));
                };
                for item in items {
                    let Value::Bool(matched) =
                        self.invoke_callable(callback, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "Predicate callback must return bool"));
                    };
                    if builtin == Builtin::Any && matched {
                        return Ok(Value::Bool(true));
                    }
                    if builtin == Builtin::All && !matched {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(builtin == Builtin::All))
            }
            Builtin::Zip => {
                let [Value::List(left), Value::List(right)] = args else {
                    return Err(self.error(span, "zip requires two lists"));
                };
                let count = left.len().min(right.len());
                let mut items = Vec::with_capacity(count);
                let mut cover = Cover::new(&self.memory);
                for (l, r) in left
                    .iter()
                    .take(count)
                    .cloned()
                    .zip(right.iter().take(count).cloned())
                {
                    items.push(Value::Tuple(vec![l, r]));
                    cover
                        .cover_vec(&items)
                        .map_err(|e| self.cover_error(span, e))?;
                }
                Ok(Value::List(items))
            }
            Builtin::GroupBy => {
                let [Value::List(items), key_fn] = args else {
                    return Err(
                        self.error(span, "group_by requires a list and a key selector callback")
                    );
                };
                let mut groups: BTreeMap<String, (Vec<Value<'static>>, Cover)> = BTreeMap::new();
                for item in items {
                    let Value::String(key) =
                        self.invoke_callable(key_fn, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "group_by key selector must return string"));
                    };
                    let (values, cover) = groups
                        .entry(key)
                        .or_insert_with(|| (Vec::new(), Cover::new(&self.memory)));
                    values.push(item.clone());
                    cover
                        .cover_vec(values)
                        .map_err(|e| self.cover_error(span, e))?;
                }
                Ok(Value::Record(
                    groups
                        .into_iter()
                        .map(|(k, (v, _))| (k, Value::List(v)))
                        .collect(),
                ))
            }
            Builtin::FoldBy => {
                let [Value::List(items), key_fn, initial, fold_fn] = args else {
                    return Err(self.error(
                        span,
                        "fold_by requires a list, key selector, initial value and fold callback",
                    ));
                };
                let mut groups: BTreeMap<String, Value<'static>> = BTreeMap::new();
                for item in items {
                    let Value::String(key) =
                        self.invoke_callable(key_fn, std::slice::from_ref(item), span)?
                    else {
                        return Err(self.error(span, "fold_by key selector must return string"));
                    };
                    let current = groups.remove(&key).unwrap_or_else(|| initial.clone());
                    let next = self.invoke_callable(fold_fn, &[current, item.clone()], span)?;
                    groups.insert(key, next);
                }
                Ok(Value::Record(RecordMap::from(groups)))
            }
            Builtin::Range | Builtin::RangeIter => {
                let (start, end, step) = match args {
                    [a, b] => {
                        let (Some(start), Some(end)) = (a.as_f64(), b.as_f64()) else {
                            return Err(
                                self.error(span, "range expects start, end and optional step")
                            );
                        };
                        (start, end, 1.0)
                    }
                    [a, b, c] => {
                        let (Some(start), Some(end), Some(step)) =
                            (a.as_f64(), b.as_f64(), c.as_f64())
                        else {
                            return Err(
                                self.error(span, "range expects start, end and optional step")
                            );
                        };
                        (start, end, step)
                    }
                    _ => return Err(self.error(span, "range expects start, end and optional step")),
                };
                if step == 0.0 {
                    return Err(self.error(span, "Range step must be nonzero"));
                }
                if !start.is_finite() || !end.is_finite() || !step.is_finite() {
                    return Err(self.error(span, "Range arguments must be finite"));
                }
                if builtin == Builtin::RangeIter {
                    return Ok(Value::Range { start, end, step });
                }
                let est = ((end - start) / step).ceil();
                let cap = if est.is_finite() && est > 0.0 {
                    (est as usize).min(self.fuel.saturating_add(1)).min(65_536)
                } else {
                    0
                };
                let mut values = Vec::with_capacity(cap);
                let mut cover = Cover::new(&self.memory);
                let mut current = start;
                while if step > 0.0 {
                    current < end
                } else {
                    current > end
                } {
                    if self.fuel == 0 {
                        return Err(self.error(span, "Execution limit exceeded"));
                    }
                    self.fuel -= 1;
                    values.push(Value::Number(current));
                    cover
                        .cover_vec(&values)
                        .map_err(|e| self.cover_error(span, e))?;
                    let next = (values.len() as f64).mul_add(step, start);
                    if !next.is_finite() || next == current {
                        return Err(self.error(span, "Range cannot advance finitely"));
                    }
                    current = next;
                }
                Ok(Value::List(values))
            }
            Builtin::Iter => {
                let Some(source) = args.first() else {
                    return Err(self.error(span, "iter requires one list or sequence"));
                };
                match source {
                    Value::List(items) => Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                        source: BytecodeSequenceSource::List(items.clone()),
                        stages: Vec::new(),
                    }))),
                    Value::Range { start, end, step } => {
                        Ok(Value::BytecodeSequence(Rc::new(BytecodeSequence {
                            source: BytecodeSequenceSource::Range {
                                start: *start,
                                end: *end,
                                step: *step,
                            },
                            stages: Vec::new(),
                        })))
                    }
                    Value::BytecodeSequence(seq) => Ok(Value::BytecodeSequence(seq.clone())),
                    _ => Err(self.error(span, "iter requires one list or sequence")),
                }
            }
            Builtin::Collect => {
                if args.len() != 2 {
                    return Err(
                        self.error(span, "collect requires a sequence and maximum item count")
                    );
                }
                let source = &args[0];
                let Some(max_count) = args[1].as_usize_index() else {
                    if matches!(args[1], Value::Int(_) | Value::Number(_)) {
                        return Err(self.error(
                            span,
                            "Collection limit must be a nonnegative representable integer",
                        ));
                    }
                    return Err(
                        self.error(span, "collect requires a sequence and maximum item count")
                    );
                };
                let mut iter = BytecodeIterator::new(source, span)?;
                let mut output = Vec::new();
                let mut cover = Cover::new(&self.memory);
                for _ in 0..max_count {
                    match iter.next(self, span)? {
                        Some(item) => {
                            output.push(item);
                            cover
                                .cover_vec(&output)
                                .map_err(|e| self.cover_error(span, e))?;
                        }
                        None => break,
                    }
                }
                Ok(Value::List(output))
            }
            other => eval_pure_builtin(other, args).map_err(|msg| self.error(span, msg)),
        }
    }
}
