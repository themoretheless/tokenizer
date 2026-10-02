//! Cooperative statement continuations. Scheduling and wake requests belong to the host.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CoroutineId(u64);

#[derive(Clone, Debug, PartialEq)]
pub enum CoroutineState<'s> {
    Yielded(Value<'s>),
    Complete(Value<'s>),
}

pub(super) enum Frame<'s> {
    Block {
        statements: Rc<Vec<Stmt<'s>>>,
        next: usize,
        environment: Environment<'s>,
    },
    While {
        condition: Expr<'s>,
        body: Rc<Vec<Stmt<'s>>>,
        environment: Environment<'s>,
    },
    For {
        binding: Name<'s>,
        body: Rc<Vec<Stmt<'s>>>,
        cursor: SequenceCursor<'s>,
        environment: Environment<'s>,
    },
}
impl<'s> Frame<'s> {
    pub(super) fn environment(&self) -> &Environment<'s> {
        match self {
            Self::Block { environment, .. }
            | Self::While { environment, .. }
            | Self::For { environment, .. } => environment,
        }
    }
    pub(super) fn sequence(&self) -> Option<&Rc<Sequence<'s>>> {
        if let Self::For { cursor, .. } = self {
            Some(&cursor.sequence)
        } else {
            None
        }
    }
}
pub(super) struct Coroutine<'s> {
    pub(super) function: Rc<Closure<'s>>,
    pub(super) frames: Vec<Frame<'s>>,
}
impl<'s> Coroutine<'s> {
    fn push(&mut self, frame: Frame<'s>, runtime: &Runtime<'_, 's>, span: Span) -> Result<()> {
        if self.frames.len() >= runtime.max_depth {
            return runtime.error(span, "Coroutine frame limit exceeded");
        }
        self.frames.push(frame);
        Ok(())
    }
    fn loop_control(&mut self, leave: bool, runtime: &Runtime<'_, 's>, span: Span) -> Result<()> {
        while let Some(frame) = self.frames.last() {
            if matches!(frame, Frame::While { .. } | Frame::For { .. }) {
                if leave {
                    self.frames.pop();
                }
                return Ok(());
            }
            self.frames.pop();
        }
        runtime.error(span, "Coroutine loop control has no enclosing loop")
    }
    fn advance(&mut self, runtime: &mut Runtime<'_, 's>, span: Span) -> Result<CoroutineState<'s>> {
        loop {
            runtime.charge(1, span)?;
            let Some(frame) = self.frames.last_mut() else {
                return Ok(CoroutineState::Complete(Value::Null));
            };
            match frame {
                Frame::While {
                    condition,
                    body,
                    environment,
                } => {
                    let value = runtime.expr(condition, environment)?;
                    let Value::Bool(enter) = value else {
                        return runtime.error(condition.span, "Condition must be boolean");
                    };
                    if enter {
                        let frame = Frame::Block {
                            statements: body.clone(),
                            next: 0,
                            environment: environment
                                .try_clone()
                                .map_err(|e| runtime.environment_error(span, e))?,
                        };
                        self.push(frame, runtime, span)?;
                    } else {
                        self.frames.pop();
                    }
                    continue;
                }
                Frame::For {
                    binding,
                    body,
                    cursor,
                    environment,
                } => {
                    if let Some(value) = runtime.sequence_next(cursor, span)? {
                        let mut environment = environment
                            .try_clone()
                            .map_err(|e| runtime.environment_error(span, e))?;
                        environment
                            .insert(binding.text, value)
                            .map_err(|e| runtime.environment_error(span, e))?;
                        let frame = Frame::Block {
                            statements: body.clone(),
                            next: 0,
                            environment,
                        };
                        self.push(frame, runtime, span)?;
                    } else {
                        self.frames.pop();
                    }
                    continue;
                }
                Frame::Block {
                    statements,
                    next,
                    environment,
                } => {
                    // Keep the code alive independently of the frame stack. Executing a
                    // statement can push/pop frames; only the Rc is copied, not its AST.
                    let code = statements.clone();
                    let Some(statement) = code.get(*next) else {
                        self.frames.pop();
                        runtime.reclaim_cells();
                        continue;
                    };
                    *next += 1;
                    match &statement.kind {
                        StmtKind::Yield(expression) => {
                            let value = runtime.expr(expression, environment)?;
                            return Ok(CoroutineState::Yielded(value));
                        }
                        StmtKind::If {
                            condition,
                            then_block,
                            else_block,
                        } => {
                            let Value::Bool(condition) = runtime.expr(condition, environment)?
                            else {
                                return runtime.error(statement.span, "Condition must be boolean");
                            };
                            if let Some(block) = if condition {
                                Some(then_block)
                            } else {
                                else_block.as_ref()
                            } {
                                let frame = Frame::Block {
                                    statements: block.stmts.clone(),
                                    next: 0,
                                    environment: environment
                                        .try_clone()
                                        .map_err(|e| runtime.environment_error(span, e))?,
                                };
                                self.push(frame, runtime, statement.span)?;
                            }
                        }
                        StmtKind::While { condition, body } => {
                            let frame = Frame::While {
                                condition: condition.clone(),
                                body: body.stmts.clone(),
                                environment: environment
                                    .try_clone()
                                    .map_err(|e| runtime.environment_error(span, e))?,
                            };
                            self.push(frame, runtime, statement.span)?;
                        }
                        StmtKind::For {
                            binding,
                            iterable,
                            body,
                        } => {
                            let value = runtime.expr(iterable, environment)?;
                            let cursor = runtime.sequence_cursor(value, iterable.span)?;
                            let frame = Frame::For {
                                binding: binding.clone(),
                                body: body.stmts.clone(),
                                cursor,
                                environment: environment
                                    .try_clone()
                                    .map_err(|e| runtime.environment_error(span, e))?,
                            };
                            self.push(frame, runtime, statement.span)?;
                        }
                        StmtKind::Break => self.loop_control(true, runtime, statement.span)?,
                        StmtKind::Continue => self.loop_control(false, runtime, statement.span)?,
                        _ => {
                            let (value, flow) =
                                runtime.statements(std::slice::from_ref(statement), environment)?;
                            if flow == Flow::Return {
                                return Ok(CoroutineState::Complete(value));
                            }
                        }
                    }
                }
            }
        }
    }
}
impl<'a, 's> ScriptInstance<'a, 's> {
    fn coroutine_limits(&mut self, limits: ExecutionLimits) -> Result<()> {
        if limits.max_depth > 64 {
            return self
                .runtime
                .error(self.span, "Maximum evaluation depth cannot exceed 64");
        }
        self.runtime.remaining = limits.steps;
        self.runtime.max_depth = limits.max_depth;
        self.runtime.max_collection_items = limits.max_collection_items;
        self.runtime.max_string_bytes = limits.max_string_bytes;
        if self.runtime.cancellation.is_cancelled() {
            self.runtime.coroutines.clear();
            self.runtime.reclaim_cells();
        }
        self.runtime.check(self.span)
    }
    /// Register a named function without running its body. Resume explicitly to execute it.
    pub fn spawn_coroutine(
        &mut self,
        name: &str,
        arguments: &[Value<'s>],
        limits: ExecutionLimits,
    ) -> Result<CoroutineId> {
        self.coroutine_limits(limits)?;
        let Some(Value::Function(function)) = self.get(name) else {
            return self
                .runtime
                .error(self.span, "Coroutine requires a Rush function");
        };
        let FunctionBody::Block(body) = &function.body else {
            return self
                .runtime
                .error(self.span, "Coroutine requires a named function body");
        };
        if arguments.len() != function.parameters.len() {
            return self.runtime.error(self.span, "Incorrect argument count");
        }
        if function
            .parameter_types
            .iter()
            .zip(arguments)
            .any(|(ty, value)| ty.as_ref().is_some_and(|ty| !ty.accepts(value)))
        {
            return self
                .runtime
                .error(self.span, "Argument does not match its type annotation");
        }
        let mut environment = Environment::child(function.environment.clone());
        if let Some(name) = function.name {
            environment
                .insert(name, Value::Function(function.clone()))
                .map_err(|e| self.runtime.environment_error(self.span, e))?;
        }
        for (parameter, value) in function.parameters.iter().zip(arguments) {
            self.runtime.host_value_size(value, self.span, 0)?;
            self.runtime
                .bind_pattern(parameter, value, &mut environment)?;
        }
        let frames = vec![Frame::Block {
            statements: body.stmts.clone(),
            next: 0,
            environment,
        }];
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = CoroutineId(
            NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| RuntimeError {
                    module: None,
                    span: self.span,
                    message: "Coroutine IDs exhausted".into(),
                    stack: Vec::new(),
                    location: None,
                })?,
        );
        self.runtime
            .coroutines
            .insert(id, Coroutine { function, frames });
        if let Err(error) = self.enforce_memory_limit() {
            self.runtime.coroutines.remove(&id);
            self.runtime.reclaim_cells();
            return Err(error);
        }
        Ok(id)
    }
    /// Run until the next yield or return with fresh limits. Errors terminate the task.
    pub fn resume_coroutine(
        &mut self,
        id: CoroutineId,
        limits: ExecutionLimits,
    ) -> Result<CoroutineState<'s>> {
        self.coroutine_limits(limits)?;
        let Some(mut task) = self.runtime.coroutines.remove(&id) else {
            return self.runtime.error(self.span, "Coroutine is not active");
        };
        let previous_module = self.runtime.module;
        let previous_references = std::mem::replace(
            &mut self.runtime.current_references,
            task.function.references.clone(),
        );
        self.runtime.module = task.function.module;
        let result = task.advance(&mut self.runtime, self.span);
        let result = if result.is_err() {
            if let Some(value) = self.runtime.early_return.take() {
                Ok(CoroutineState::Complete(value))
            } else {
                result
            }
        } else {
            result
        };
        let result = match result {
            Ok(CoroutineState::Complete(value))
                if task
                    .function
                    .result_type
                    .as_ref()
                    .is_some_and(|ty| !ty.accepts(&value)) =>
            {
                self.runtime
                    .error(self.span, "Return value does not match its type annotation")
            }
            result => result,
        };
        let function_span = match &task.function.body {
            FunctionBody::Block(body) => body.span,
            _ => self.span,
        };
        let result = result.map_err(|error| {
            self.runtime.call_error(
                error,
                task.function.name.unwrap_or("<lambda>").to_owned(),
                task.function.module,
                function_span,
            )
        });
        self.runtime.module = previous_module;
        self.runtime.current_references = previous_references;
        if matches!(result, Ok(CoroutineState::Yielded(_))) {
            self.runtime.coroutines.insert(id, task);
            if let Err(error) = self.enforce_memory_limit() {
                self.runtime.coroutines.remove(&id);
                self.runtime.reclaim_cells();
                return Err(error);
            }
        } else {
            drop(task);
        }
        self.runtime.reclaim_cells();
        result
    }
    /// Cancel one task immediately, dropping its locals and open iterators. Mutations persist.
    pub fn cancel_coroutine(&mut self, id: CoroutineId) -> bool {
        let removed = self.runtime.coroutines.remove(&id).is_some();
        self.runtime.reclaim_cells();
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_and_nested_frames_share_program_statement_storage() {
        let program = Program::compile("fn work() { while true { if true { yield 1 } } }").unwrap();
        let cancellation = CancellationToken::default();
        let limits = ExecutionLimits::new(1000);
        let mut script = program
            .instantiate(limits, &cancellation, &[], &[], &[], &[])
            .unwrap();
        let StmtKind::Function { body, .. } = &program.parsed.module.items[0].kind else {
            panic!()
        };
        let StmtKind::While {
            body: loop_body, ..
        } = &body.stmts[0].kind
        else {
            panic!()
        };
        let StmtKind::If { then_block, .. } = &loop_body.stmts[0].kind else {
            panic!()
        };
        for _ in 0..16 {
            let id = script.spawn_coroutine("work", &[], limits).unwrap();
            let Frame::Block { statements, .. } = &script.runtime.coroutines[&id].frames[0] else {
                panic!()
            };
            assert!(Rc::ptr_eq(statements, &body.stmts));
            for _ in 0..2 {
                script.resume_coroutine(id, limits).unwrap();
                let frames = &script.runtime.coroutines[&id].frames;
                let Frame::Block { statements, .. } = frames.last().unwrap() else {
                    panic!()
                };
                assert!(Rc::ptr_eq(statements, &then_block.stmts));
                let Frame::While { body, .. } = &frames[1] else {
                    panic!()
                };
                assert!(Rc::ptr_eq(body, &loop_body.stmts));
            }
        }
    }
}
