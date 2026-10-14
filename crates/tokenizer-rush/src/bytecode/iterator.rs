//! Lazy sequence and iterator types for the Bytecode VM.
use crate::bytecode::vm::Vm;
use crate::runtime::ops::is_truthy;
use crate::{RuntimeError, Value};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

/// A repeatable sequence with deferred transformation stages in bytecode VM.
#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeSequence {
    pub source: BytecodeSequenceSource,
    pub stages: Vec<BytecodeSequenceStage>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BytecodeSequenceSource {
    List(Vec<Value<'static>>),
    Range { start: f64, end: f64, step: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeSequenceStage {
    pub callback: Value<'static>,
    pub filter: bool,
    pub span: Span,
}

/// Active cursor iterating over a list, range, or sequence in bytecode VM.
#[derive(Clone, Debug, PartialEq)]
pub enum BytecodeIterator {
    List {
        items: Vec<Value<'static>>,
        index: usize,
    },
    Range {
        start: f64,
        end: f64,
        step: f64,
        index: usize,
        previous: Option<f64>,
    },
    Sequence {
        source_iter: Box<BytecodeIterator>,
        stages: Vec<BytecodeSequenceStage>,
        finished: bool,
    },
}

impl BytecodeIterator {
    pub fn new(value: &Value<'static>, span: Span) -> Result<Self, RuntimeError> {
        match value {
            Value::List(items) => Ok(BytecodeIterator::List {
                items: items.clone(),
                index: 0,
            }),
            Value::Range { start, end, step } => {
                if *step == 0.0 {
                    return Err(RuntimeError {
                        module: None,
                        span,
                        message: "Range step must be nonzero".into(),
                        stack: Vec::new(),
                        location: None,
                    });
                }
                if !start.is_finite() || !end.is_finite() || !step.is_finite() {
                    return Err(RuntimeError {
                        module: None,
                        span,
                        message: "Range arguments must be finite".into(),
                        stack: Vec::new(),
                        location: None,
                    });
                }
                Ok(BytecodeIterator::Range {
                    start: *start,
                    end: *end,
                    step: *step,
                    index: 0,
                    previous: None,
                })
            }
            Value::BytecodeSequence(seq) => {
                let source_iter = match &seq.source {
                    BytecodeSequenceSource::List(items) => BytecodeIterator::List {
                        items: items.clone(),
                        index: 0,
                    },
                    BytecodeSequenceSource::Range { start, end, step } => BytecodeIterator::Range {
                        start: *start,
                        end: *end,
                        step: *step,
                        index: 0,
                        previous: None,
                    },
                };
                Ok(BytecodeIterator::Sequence {
                    source_iter: Box::new(source_iter),
                    stages: seq.stages.clone(),
                    finished: false,
                })
            }
            Value::Sequence(seq) => {
                if let Some(b_seq) = seq.to_bytecode_sequence() {
                    return Self::new(&Value::BytecodeSequence(Rc::new(b_seq)), span);
                }
                Err(RuntimeError {
                    module: None,
                    span,
                    message: "Host sequence not supported in bytecode yet".into(),
                    stack: Vec::new(),
                    location: None,
                })
            }
            _ => Err(RuntimeError {
                module: None,
                span,
                message: "Expected a list or sequence".into(),
                stack: Vec::new(),
                location: None,
            }),
        }
    }

    #[inline]
    pub fn next(
        &mut self,
        vm: &mut Vm,
        span: Span,
    ) -> Result<Option<Value<'static>>, RuntimeError> {
        match self {
            BytecodeIterator::List { items, index } => {
                if *index < items.len() {
                    let item = items[*index].clone();
                    *index += 1;
                    Ok(Some(item))
                } else {
                    Ok(None)
                }
            }
            BytecodeIterator::Range {
                start,
                end,
                step,
                index,
                previous,
            } => {
                let current = (*index as f64).mul_add(*step, *start);
                if *previous == Some(current) || !current.is_finite() {
                    return Err(vm.error(span, "Range cannot advance finitely"));
                }
                if if *step > 0.0 {
                    current >= *end
                } else {
                    current <= *end
                } {
                    return Ok(None);
                }
                *previous = Some(current);
                *index += 1;
                Ok(Some(Value::Number(current)))
            }
            BytecodeIterator::Sequence {
                source_iter,
                stages,
                finished,
            } => {
                if *finished {
                    return Ok(None);
                }
                'candidate: loop {
                    if vm.fuel == 0 {
                        return Err(vm.error(span, "Execution limit exceeded"));
                    }
                    vm.fuel -= 1;

                    let Some(mut item) = source_iter.next(vm, span)? else {
                        *finished = true;
                        return Ok(None);
                    };

                    for stage in stages.iter() {
                        let res = vm.invoke_callable(
                            &stage.callback,
                            std::slice::from_ref(&item),
                            stage.span,
                        )?;
                        if stage.filter {
                            if !is_truthy(&res) {
                                continue 'candidate;
                            }
                        } else {
                            item = res;
                        }
                    }
                    return Ok(Some(item));
                }
            }
        }
    }
}
