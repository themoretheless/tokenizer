use super::{CancellationToken, Result, Runtime, RuntimeError, Value, ValueType, memory};
use std::borrow::Cow;
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

/// Factory for a host-owned source. Each consumption opens a fresh iterator.
/// Keep resource handles in the iterator and release them in its Drop implementation.
pub trait HostSequence: std::fmt::Debug {
    fn open(
        &self,
        cancellation: &CancellationToken,
    ) -> std::result::Result<Box<dyn HostSequenceIterator>, String>;
}

/// One opened source. Values own their data; no reference to a temporary host buffer escapes.
/// Blocking implementations must cooperate with the cancellation token themselves.
pub trait HostSequenceIterator {
    fn next(
        &mut self,
        cancellation: &CancellationToken,
    ) -> std::result::Result<Option<Value<'static>>, String>;
}

#[derive(Clone, Debug)]
pub(super) struct HostSource {
    pub(super) factory: Rc<dyn HostSequence>,
    pub(super) item_type: ValueType,
}

impl PartialEq for HostSource {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.factory, &other.factory) && self.item_type == other.item_type
    }
}

/// Repeatable sequence with deferred transformations. Construct through Rush builtins.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence<'s> {
    pub(super) source: SequenceSource<'s>,
    pub(super) stages: SequenceStages<'s>,
}

impl<'s> Sequence<'s> {
    pub(crate) fn to_bytecode_sequence(&self) -> Option<crate::bytecode::BytecodeSequence> {
        let source = match &self.source {
            SequenceSource::Range { start, end, step } => {
                crate::bytecode::BytecodeSequenceSource::Range {
                    start: *start,
                    end: *end,
                    step: *step,
                }
            }
            SequenceSource::List(buf) => {
                let items: Option<Vec<Value<'static>>> =
                    buf.iter().map(|v| v.to_static()).collect();
                crate::bytecode::BytecodeSequenceSource::List(items?)
            }
            SequenceSource::Host(_) => return None,
        };
        let mut stages = Vec::with_capacity(self.stages.len());
        for stage in self.stages.iter() {
            stages.push(crate::bytecode::BytecodeSequenceStage {
                callback: stage.callback.to_static()?,
                filter: stage.filter,
                span: stage.span,
            });
        }
        Some(crate::bytecode::BytecodeSequence { source, stages })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SequenceSource<'s> {
    Range { start: f64, end: f64, step: f64 },
    List(memory::Buffer<Value<'s>>),
    Host(HostSource),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SequenceStage<'s> {
    pub(super) callback: Value<'s>,
    pub(super) filter: bool,
    pub(super) span: Span,
    pub(super) module: Option<&'s str>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SequenceStages<'s>(
    pub(super) Option<memory::Shared<memory::Slots<SequenceStage<'s>>>>,
);

impl PartialEq for SequenceStages<'_> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<'s> std::ops::Deref for SequenceStages<'s> {
    type Target = [SequenceStage<'s>];

    fn deref(&self) -> &Self::Target {
        self.0.as_deref().map(|slots| &**slots).unwrap_or(&[])
    }
}

impl<'s> SequenceStages<'s> {
    pub(super) fn capacity(&self) -> usize {
        self.0.as_ref().map_or(0, |slots| slots.capacity())
    }

    pub(super) fn append(
        &mut self,
        budget: &memory::Budget,
        stage: SequenceStage<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        let size = self
            .len()
            .checked_add(1)
            .ok_or(memory::AllocationError::Capacity)?;
        let mut slots = memory::Slots::new(budget, size)?;
        for existing in self.iter() {
            slots
                .push(existing.clone())
                .expect("stage capacity reserved");
        }
        slots.push(stage).expect("stage capacity reserved");
        let shared = memory::Shared::new(budget, slots)?;
        self.0 = Some(shared);
        Ok(())
    }
}

pub(super) struct SequenceCursor<'s> {
    pub(super) sequence: Rc<Sequence<'s>>,
    pub(super) index: usize,
    pub(super) previous: Option<f64>,
    pub(super) host: Option<Box<dyn HostSequenceIterator>>,
    pub(super) finished: bool,
}

impl<'s> Runtime<'_, 's> {
    pub(super) fn sequence_cursor(
        &mut self,
        value: Value<'s>,
        span: Span,
    ) -> Result<SequenceCursor<'s>> {
        let sequence = match value {
            Value::Sequence(sequence) => sequence,
            Value::BytecodeSequence(seq) => {
                let source = match &seq.source {
                    crate::bytecode::BytecodeSequenceSource::Range { start, end, step } => {
                        SequenceSource::Range {
                            start: *start,
                            end: *end,
                            step: *step,
                        }
                    }
                    crate::bytecode::BytecodeSequenceSource::List(items) => SequenceSource::List(
                        memory::Buffer::from_iter(&self.memory, items.iter().cloned()).map_err(
                            |e| RuntimeError {
                                stack: Vec::new(),
                                location: None,
                                module: self.module.map(str::to_owned),
                                span,
                                message: format!(
                                    "Runtime sequence source allocation failed: {e:?}"
                                ),
                            },
                        )?,
                    ),
                };
                let mut stages = SequenceStages::default();
                for stage in &seq.stages {
                    stages
                        .append(
                            &self.memory,
                            SequenceStage {
                                callback: stage.callback.clone(),
                                filter: stage.filter,
                                span: stage.span,
                                module: self.module,
                            },
                        )
                        .map_err(|e| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: format!("Runtime sequence stage allocation failed: {e:?}"),
                        })?;
                }
                Rc::new(Sequence { source, stages })
            }
            Value::Range { start, end, step } => Rc::new(Sequence {
                source: SequenceSource::Range { start, end, step },
                stages: SequenceStages::default(),
            }),
            Value::List(items) => Rc::new(Sequence {
                source: SequenceSource::List(
                    memory::Buffer::from_iter(&self.memory, items).map_err(|e| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Runtime sequence source allocation failed: {e:?}"),
                    })?,
                ),
                stages: SequenceStages::default(),
            }),
            _ => return self.error(span, "Expected a list or sequence"),
        };
        Ok(SequenceCursor {
            sequence,
            index: 0,
            previous: None,
            host: None,
            finished: false,
        })
    }

    pub(super) fn sequence_next(
        &mut self,
        cursor: &mut SequenceCursor<'s>,
        span: Span,
    ) -> Result<Option<Value<'s>>> {
        if cursor.finished {
            return Ok(None);
        }
        'candidate: loop {
            self.charge(1, span)?;
            let mut item = match &cursor.sequence.source {
                SequenceSource::Host(source) => {
                    if cursor.host.is_none() {
                        cursor.host =
                            Some(source.factory.open(self.cancellation).map_err(|message| {
                                RuntimeError {
                                    stack: Vec::new(),
                                    location: None,
                                    module: self.module.map(str::to_owned),
                                    span,
                                    message,
                                }
                            })?);
                        self.check(span)?;
                    }
                    let next = cursor
                        .host
                        .as_mut()
                        .unwrap()
                        .next(self.cancellation)
                        .map_err(|message| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message,
                        })?;
                    self.check(span)?;
                    let Some(item) = next else {
                        cursor.finished = true;
                        cursor.host = None;
                        return Ok(None);
                    };
                    self.host_value_size(&item, span, 0)?;
                    if !source.item_type.accepts(&item) {
                        return self
                            .error(span, "Host sequence item does not match its declared type");
                    }
                    item
                }
                SequenceSource::List(items) => match items.get(cursor.index) {
                    Some(item) => item.clone(),
                    None => return Ok(None),
                },
                SequenceSource::Range { start, end, step } => {
                    let current = (cursor.index as f64).mul_add(*step, *start);
                    if cursor.previous == Some(current) || !current.is_finite() {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    if if *step > 0.0 {
                        current >= *end
                    } else {
                        current <= *end
                    } {
                        return Ok(None);
                    }
                    cursor.previous = Some(current);
                    Value::Number(current)
                }
            };
            cursor.index = cursor.index.checked_add(1).ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Sequence index overflow".into(),
            })?;
            for stage in cursor.sequence.stages.iter() {
                let previous_module = self.module;
                self.module = stage.module;
                let result = self
                    .call(
                        stage.callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        stage.span,
                    )
                    .and_then(|value| {
                        if stage.filter && !matches!(value, Value::Bool(_)) {
                            self.error(stage.span, "Filter callback must return a boolean")
                        } else {
                            Ok(value)
                        }
                    });
                self.module = previous_module;
                let result = result?;
                if stage.filter {
                    if result == Value::Bool(false) {
                        continue 'candidate;
                    }
                } else {
                    item = result;
                }
            }
            return Ok(Some(item));
        }
    }
}
