//! Deterministic host-driven time/event scheduling. Each ready task advances once per poll.
use super::*;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeRequest {
    After(Duration),
    Event(String),
}
#[derive(Clone, Debug, PartialEq)]
pub enum ScheduledState<'s> {
    Waiting(WakeRequest),
    Complete(Value<'s>),
    Failed(RuntimeError),
}
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledStep<'s> {
    pub id: CoroutineId,
    pub state: ScheduledState<'s>,
}
enum Waiting {
    Ready(Duration),
    Timer(Duration),
    Event(String),
}
struct ScheduledTask {
    order: u64,
    waiting: Waiting,
}
/// Borrows one instance; time advances only through `poll`, events through `emit`.
/// Events are broadcast to current subscribers, are not buffered, and carry no payload.
/// Drop cancels every owned task, closing its iterators and removing subscriptions.
pub struct CoroutineScheduler<'i, 'a, 's> {
    instance: &'i mut ScriptInstance<'a, 's>,
    tasks: HashMap<CoroutineId, ScheduledTask>,
    now: Duration,
    order: u64,
}
impl<'i, 'a, 's> CoroutineScheduler<'i, 'a, 's> {
    pub fn new(instance: &'i mut ScriptInstance<'a, 's>) -> Self {
        Self {
            instance,
            tasks: HashMap::new(),
            now: Duration::ZERO,
            order: 0,
        }
    }
    /// Access handlers/state on the same instance, for example to deliver an event payload.
    pub fn instance_mut(&mut self) -> &mut ScriptInstance<'a, 's> {
        self.instance
    }
    pub fn len(&self) -> usize {
        self.tasks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
    pub fn subscriptions(&self, event: &str) -> usize {
        self.tasks
            .values()
            .filter(|task| matches!(&task.waiting,Waiting::Event(name) if name == event))
            .count()
    }
    pub fn next_deadline(&self) -> Option<Duration> {
        self.tasks
            .values()
            .filter_map(|task| match task.waiting {
                Waiting::Ready(at) | Waiting::Timer(at) => Some(at),
                Waiting::Event(_) => None,
            })
            .min()
    }
    pub fn spawn(
        &mut self,
        name: &str,
        arguments: &[Value<'s>],
        limits: ExecutionLimits,
    ) -> Result<CoroutineId> {
        self.check_cancelled()?;
        let Some(order) = self.order.checked_add(1) else {
            return self.error("Scheduler order exhausted");
        };
        let id = self.instance.spawn_coroutine(name, arguments, limits)?;
        self.order = order;
        self.tasks.insert(
            id,
            ScheduledTask {
                order,
                waiting: Waiting::Ready(self.now),
            },
        );
        Ok(id)
    }
    /// Wake all current subscribers in creation order at the scheduler's current time.
    pub fn emit(&mut self, event: &str) -> Result<usize> {
        self.check_cancelled()?;
        let mut count = 0;
        for task in self.tasks.values_mut() {
            if matches!(&task.waiting,Waiting::Event(name) if name == event) {
                task.waiting = Waiting::Ready(self.now);
                count += 1;
            }
        }
        Ok(count)
    }
    pub fn cancel(&mut self, id: CoroutineId) -> bool {
        if self.tasks.remove(&id).is_none() {
            return false;
        }
        self.instance.cancel_coroutine(id);
        true
    }
    pub fn cancel_all(&mut self) {
        for (id, _) in self.tasks.drain() {
            self.instance.cancel_coroutine(id);
        }
    }
    fn error<T>(&self, message: &str) -> Result<T> {
        self.instance.runtime.error(self.instance.span, message)
    }
    fn check_cancelled(&mut self) -> Result<()> {
        if self.instance.runtime.cancellation.is_cancelled() {
            self.cancel_all();
            return self.error("Execution cancelled");
        }
        Ok(())
    }
    fn request(value: Value<'s>) -> std::result::Result<WakeRequest, &'static str> {
        let Value::UserData(data) = value else {
            return Err("Yield a Wait.After(number) or Wait.Event(str) request");
        };
        if !data.type_name.ends_with("::Wait") {
            return Err("Scheduler request must use the Wait enum");
        }
        match (data.variant.as_deref(), data.values.as_slice()) {
            (Some("After"), [Value::Number(seconds)]) => Duration::try_from_secs_f64(*seconds)
                .map(WakeRequest::After)
                .map_err(|_| "Wait.After requires a finite non-negative duration"),
            (Some("Event"), [Value::String(name)]) => Ok(WakeRequest::Event(name.clone())),
            _ => Err("Yield a Wait.After(number) or Wait.Event(str) request"),
        }
    }
    /// Monotonic host time. Ready tasks run by wake time, then creation order.
    /// One resume per task per poll prevents zero-delay tasks from starving peers.
    /// Per-task errors are terminal steps; other ready tasks still run.
    pub fn poll(
        &mut self,
        now: Duration,
        limits: ExecutionLimits,
    ) -> Result<Vec<ScheduledStep<'s>>> {
        self.check_cancelled()?;
        if now < self.now {
            return self.error("Scheduler time must be monotonic");
        }
        if limits.max_depth > 64 {
            return self.error("Maximum evaluation depth cannot exceed 64");
        }
        self.now = now;
        let mut ready: Vec<_> = self
            .tasks
            .iter()
            .filter_map(|(id, task)| match task.waiting {
                Waiting::Ready(at) | Waiting::Timer(at) if at <= now => Some((at, task.order, *id)),
                _ => None,
            })
            .collect();
        ready.sort_by_key(|(at, order, _)| (*at, *order));
        let mut steps = Vec::new();
        for (_, _, id) in ready {
            if self.instance.runtime.cancellation.is_cancelled() {
                self.cancel_all();
                break;
            }
            let mut task = self.tasks.remove(&id).expect("ready task");
            let state = match self.instance.resume_coroutine(id, limits) {
                Ok(CoroutineState::Complete(value)) => ScheduledState::Complete(value),
                Err(error) => ScheduledState::Failed(error),
                Ok(CoroutineState::Yielded(value)) => {
                    match Self::request(value).and_then(|request| {
                        task.waiting = match &request {
                            WakeRequest::After(delay) => Waiting::Timer(
                                now.checked_add(*delay).ok_or("Timer deadline overflow")?,
                            ),
                            WakeRequest::Event(name) => Waiting::Event(name.clone()),
                        };
                        Ok(request)
                    }) {
                        Ok(request) => {
                            self.tasks.insert(id, task);
                            ScheduledState::Waiting(request)
                        }
                        Err(message) => {
                            self.instance.cancel_coroutine(id);
                            ScheduledState::Failed(self.error::<()>(message).unwrap_err())
                        }
                    }
                }
            };
            steps.push(ScheduledStep { id, state });
            if self.instance.runtime.cancellation.is_cancelled() {
                self.cancel_all();
                break;
            }
        }
        Ok(steps)
    }
}
impl Drop for CoroutineScheduler<'_, '_, '_> {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

/// An owned scheduler suitable for browser and application handles.
/// Calls preserve task state without keeping a borrow of the script alive.
pub struct OwnedCoroutineScheduler {
    instance: instance::OwnedScriptInstance,
    tasks: HashMap<CoroutineId, ScheduledTask>,
    now: Duration,
    order: u64,
}
impl OwnedCoroutineScheduler {
    pub fn new(source: impl Into<String>, limits: ExecutionLimits) -> Result<Self> {
        Ok(Self {
            instance: instance::OwnedScriptInstance::new(source, limits)?,
            tasks: HashMap::new(),
            now: Duration::ZERO,
            order: 0,
        })
    }
    fn with_scheduler<R>(
        &mut self,
        f: impl for<'a> FnOnce(&mut CoroutineScheduler<'_, 'a, 'a>) -> R,
    ) -> R {
        let tasks = std::mem::take(&mut self.tasks);
        let now = self.now;
        let order = self.order;
        let (result, tasks, now, order) = self.instance.with_instance(|instance| {
            let mut scheduler = CoroutineScheduler {
                instance,
                tasks,
                now,
                order,
            };
            let result = f(&mut scheduler);
            let tasks = std::mem::take(&mut scheduler.tasks);
            (result, tasks, scheduler.now, scheduler.order)
        });
        self.tasks = tasks;
        self.now = now;
        self.order = order;
        result
    }
    pub fn spawn(
        &mut self,
        name: &str,
        arguments: &[Value<'static>],
        limits: ExecutionLimits,
    ) -> Result<CoroutineId> {
        self.with_scheduler(|s| s.spawn(name, arguments, limits))
    }
    pub fn emit(&mut self, event: &str) -> Result<usize> {
        self.with_scheduler(|s| s.emit(event))
    }
    pub fn cancel(&mut self, id: CoroutineId) -> bool {
        self.with_scheduler(|s| s.cancel(id))
    }
    pub fn cancel_all(&mut self) {
        self.with_scheduler(|s| s.cancel_all())
    }
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
    pub fn next_deadline(&self) -> Option<Duration> {
        self.tasks
            .values()
            .filter_map(|t| match t.waiting {
                Waiting::Ready(at) | Waiting::Timer(at) => Some(at),
                Waiting::Event(_) => None,
            })
            .min()
    }
    pub fn poll(
        &mut self,
        now: Duration,
        limits: ExecutionLimits,
    ) -> Result<Vec<ScheduledStep<'static>>> {
        self.with_scheduler(|s| {
            s.poll(now, limits)?
                .into_iter()
                .map(|step| {
                    let state = match step.state {
                        ScheduledState::Complete(value) => ScheduledState::Complete(
                            instance::owned_value(value)
                                .map_err(|message| s.error::<()>(&message).unwrap_err())?,
                        ),
                        ScheduledState::Waiting(request) => ScheduledState::Waiting(request),
                        ScheduledState::Failed(error) => ScheduledState::Failed(error),
                    };
                    Ok(ScheduledStep { id: step.id, state })
                })
                .collect()
        })
    }
}
