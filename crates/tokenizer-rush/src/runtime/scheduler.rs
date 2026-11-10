//! Deterministic host-driven time/event scheduling. Each ready task advances once per poll.
use super::*;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WakeRequest {
    After(Duration),
    Event(String),
    /// `await task_handle`: wake when the referenced task completes or fails.
    Task(CoroutineId),
    /// `await task_all([...])`: wake with `Value::List` when all tasks complete;
    /// if any task fails or cancels, cancel remaining siblings and fail waiter.
    All(Vec<CoroutineId>),
    /// `await task_race([...])`: wake with the first completed result and
    /// automatically cancel all losing sibling tasks.
    Race(Vec<CoroutineId>),
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
    /// Suspended at `await task`: wakes when the target completes; a failure
    /// or cancellation of the target is terminal for the waiter.
    Task(CoroutineId),
    /// Suspended at `await task_all([...])`; partial values live in `Coroutine::pending_all`.
    All(Vec<CoroutineId>),
    /// Suspended at `await task_race([...])`; first completion cancels remaining siblings.
    Race(Vec<CoroutineId>),
}
impl Waiting {
    fn awaits(&self, target: CoroutineId) -> bool {
        match self {
            Self::Task(t) => *t == target,
            Self::All(targets) | Self::Race(targets) => targets.contains(&target),
            _ => false,
        }
    }
    fn siblings(&self) -> &[CoroutineId] {
        match self {
            Self::Task(t) => std::slice::from_ref(t),
            Self::All(targets) | Self::Race(targets) => targets.as_slice(),
            _ => &[],
        }
    }
}
struct ScheduledTask {
    order: u64,
    waiting: Waiting,
    /// Copied from the coroutine at suspend time: an `await` suspension
    /// consumes a delivered wake value (task result, event payload, or null),
    /// a plain `yield` does not.
    delivers: bool,
}
/// Borrows one instance; time advances only through `poll`, events through `emit`.
/// Events are broadcast with an optional payload to current subscribers, never buffered.
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
    /// Access handlers/state on the same instance between scheduler calls.
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
                Waiting::Event(_) | Waiting::Task(_) | Waiting::All(_) | Waiting::Race(_) => None,
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
                delivers: false,
            },
        );
        Ok(id)
    }
    /// Adopt a task produced by calling an `async fn` in the script (the id
    /// comes from `Value::Task`): the task becomes ready at the current time.
    pub fn spawn_task(&mut self, id: CoroutineId) -> Result<CoroutineId> {
        self.check_cancelled()?;
        if !self.instance.runtime.coroutines.contains_key(&id) {
            return self.error("Task is not active");
        }
        if self.tasks.contains_key(&id) {
            return Ok(id);
        }
        let Some(order) = self.order.checked_add(1) else {
            return self.error("Scheduler order exhausted");
        };
        self.order = order;
        self.tasks.insert(
            id,
            ScheduledTask {
                order,
                waiting: Waiting::Ready(self.now),
                delivers: false,
            },
        );
        Ok(id)
    }
    /// Wake all current subscribers with `null`, preserving the original API.
    pub fn emit(&mut self, event: &str) -> Result<usize> {
        self.emit_with_payload(event, Value::Null)
    }
    /// Wake all current subscribers and deliver a clone of `payload` to each
    /// `await Wait.Event`. Events are not buffered for future subscribers.
    pub fn emit_with_payload(&mut self, event: &str, payload: Value<'s>) -> Result<usize> {
        self.check_cancelled()?;
        let mut count = 0;
        for (id, task) in self.tasks.iter_mut() {
            if matches!(&task.waiting,Waiting::Event(name) if name == event) {
                task.waiting = Waiting::Ready(self.now);
                if task.delivers
                    && let Some(coroutine) = self.instance.runtime.coroutines.get_mut(id)
                {
                    coroutine.pending_wake = Some(payload.clone());
                }
                count += 1;
            }
        }
        Ok(count)
    }
    fn any_waiter_awaits(&self, target: CoroutineId) -> bool {
        self.tasks.values().any(|task| task.waiting.awaits(target))
    }
    pub fn cancel(&mut self, id: CoroutineId) -> bool {
        if self.tasks.remove(&id).is_none() {
            return false;
        }
        self.instance.cancel_coroutine(id);
        // Awaiting a cancelled task is terminal for the waiters (and any
        // sibling tasks owned by structured `task_all` / `task_race` groups).
        let waiters: Vec<(CoroutineId, Vec<CoroutineId>)> = self
            .tasks
            .iter()
            .filter(|(_, task)| task.waiting.awaits(id))
            .map(|(waiter, task)| (*waiter, task.waiting.siblings().to_vec()))
            .collect();
        let mut sibling_candidates = Vec::new();
        for (waiter, siblings) in waiters {
            sibling_candidates.extend(siblings.into_iter().filter(|sib| *sib != id));
            self.cancel(waiter);
        }
        for sib in sibling_candidates {
            if !self.any_waiter_awaits(sib) {
                self.cancel(sib);
            }
        }
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
    fn extract_task_list(
        items: &[Value<'s>],
    ) -> std::result::Result<Vec<CoroutineId>, &'static str> {
        items
            .iter()
            .map(|item| match item {
                Value::Task(id) => Ok(*id),
                _ => Err("Task group requires task handles produced by async functions"),
            })
            .collect()
    }
    fn request(value: Value<'s>) -> std::result::Result<WakeRequest, &'static str> {
        match value {
            Value::Task(id) => return Ok(WakeRequest::Task(id)),
            Value::Variant("TaskAll", ref items) => {
                return Self::extract_task_list(items).map(WakeRequest::All);
            }
            Value::Variant("TaskRace", ref items) => {
                return Self::extract_task_list(items).map(WakeRequest::Race);
            }
            _ => {}
        }
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
            (Some("All"), [Value::List(items) | Value::Tuple(items)]) => {
                Self::extract_task_list(items).map(WakeRequest::All)
            }
            (Some("Race"), [Value::List(items) | Value::Tuple(items)]) => {
                Self::extract_task_list(items).map(WakeRequest::Race)
            }
            _ => Err("Yield a Wait.After(number) or Wait.Event(str) request"),
        }
    }
    /// Would `waiter` awaiting `target` close a cycle? Follows the graph of
    /// task waits (including `All` and `Race` groups); self-await is length 1.
    fn would_cycle(&self, waiter: CoroutineId, target: CoroutineId) -> bool {
        let mut stack = vec![target];
        let mut visited = std::collections::HashSet::new();
        while let Some(current) = stack.pop() {
            if current == waiter {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(task) = self.tasks.get(&current) {
                stack.extend_from_slice(task.waiting.siblings());
            }
        }
        false
    }
    /// Adopt an active but unscheduled task (awaited before registration):
    /// it becomes ready at the scheduler's current time.
    fn adopt(&mut self, id: CoroutineId) -> std::result::Result<(), &'static str> {
        let Some(order) = self.order.checked_add(1) else {
            return Err("Scheduler order exhausted");
        };
        self.order = order;
        self.tasks.insert(
            id,
            ScheduledTask {
                order,
                waiting: Waiting::Ready(self.now),
                delivers: false,
            },
        );
        Ok(())
    }
    /// Wake or fail every task awaiting `finished`, in creation order; a
    /// failure cascades to the waiters' own waiters and structured siblings.
    fn settle_waiters(
        &mut self,
        finished: CoroutineId,
        outcome: &ScheduledState<'s>,
        steps: &mut Vec<ScheduledStep<'s>>,
    ) {
        let mut waiters: Vec<_> = self
            .tasks
            .iter()
            .filter(|(_, task)| task.waiting.awaits(finished))
            .map(|(id, task)| (task.order, *id))
            .collect();
        waiters.sort_unstable_by_key(|(order, _)| *order);
        for (_, waiter) in waiters {
            if !self.tasks.contains_key(&waiter) {
                continue;
            }
            match outcome {
                ScheduledState::Complete(value) => {
                    let mut cancel_losers: Vec<CoroutineId> = Vec::new();
                    if let Some(task) = self.tasks.get_mut(&waiter) {
                        match &task.waiting {
                            Waiting::Task(_) => {
                                task.waiting = Waiting::Ready(self.now);
                                if task.delivers
                                    && let Some(coroutine) =
                                        self.instance.runtime.coroutines.get_mut(&waiter)
                                {
                                    coroutine.pending_wake = Some(value.clone());
                                }
                            }
                            Waiting::All(targets) => {
                                let all_done = if let Some(coroutine) =
                                    self.instance.runtime.coroutines.get_mut(&waiter)
                                {
                                    for (idx, target) in targets.iter().enumerate() {
                                        if *target == finished
                                            && let Some(slot) = coroutine.pending_all.get_mut(idx)
                                        {
                                            *slot = Some(value.clone());
                                        }
                                    }
                                    if coroutine.pending_all.iter().all(Option::is_some) {
                                        let collected: Vec<Value<'s>> = coroutine
                                            .pending_all
                                            .drain(..)
                                            .map(|slot| slot.expect("completed slot"))
                                            .collect();
                                        if task.delivers {
                                            coroutine.pending_wake = Some(Value::List(collected));
                                        }
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if all_done {
                                    task.waiting = Waiting::Ready(self.now);
                                }
                            }
                            Waiting::Race(targets) => {
                                cancel_losers =
                                    targets.iter().copied().filter(|t| *t != finished).collect();
                                task.waiting = Waiting::Ready(self.now);
                                if task.delivers
                                    && let Some(coroutine) =
                                        self.instance.runtime.coroutines.get_mut(&waiter)
                                {
                                    coroutine.pending_wake = Some(value.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    for loser in cancel_losers {
                        if !self.any_waiter_awaits(loser) {
                            self.cancel(loser);
                        }
                    }
                }
                ScheduledState::Failed(error) => {
                    let siblings = self
                        .tasks
                        .remove(&waiter)
                        .map(|t| t.waiting.siblings().to_vec())
                        .unwrap_or_default();
                    self.instance.cancel_coroutine(waiter);
                    for sib in siblings {
                        if sib != finished && !self.any_waiter_awaits(sib) {
                            self.cancel(sib);
                        }
                    }
                    let state = ScheduledState::Failed(error.clone());
                    self.settle_waiters(waiter, &state, steps);
                    steps.push(ScheduledStep { id: waiter, state });
                }
                ScheduledState::Waiting(_) => {}
            }
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
        // Reap tasks whose coroutine was cancelled directly, bypassing the
        // scheduler (a deliberate host cancellation produces no failure step);
        // the orphan pass below fails their waiters as cancelled.
        let mut zombies: Vec<_> = self
            .tasks
            .iter()
            .filter(|(id, _)| !self.instance.runtime.coroutines.contains_key(id))
            .map(|(id, task)| (task.order, *id))
            .collect();
        zombies.sort_unstable_by_key(|(order, _)| *order);
        let mut steps = Vec::new();
        for (_, id) in zombies {
            self.tasks.remove(&id);
        }
        // A waiter whose target vanished without completing (cancelled via
        // cancel_coroutine, bypassing the scheduler) fails terminally instead
        // of hanging. Completed targets settle their waiters immediately, so
        // an absent coroutine can only mean cancellation.
        let mut orphaned: Vec<_> = self
            .tasks
            .iter()
            .filter(|(waiter, task)| match &task.waiting {
                Waiting::Task(target) => !self.instance.runtime.coroutines.contains_key(target),
                Waiting::All(targets) => targets.iter().enumerate().any(|(idx, target)| {
                    let already_settled = self
                        .instance
                        .runtime
                        .coroutines
                        .get(waiter)
                        .and_then(|c| c.pending_all.get(idx))
                        .is_some_and(Option::is_some);
                    !already_settled && !self.instance.runtime.coroutines.contains_key(target)
                }),
                Waiting::Race(targets) => targets
                    .iter()
                    .any(|target| !self.instance.runtime.coroutines.contains_key(target)),
                _ => false,
            })
            .map(|(id, task)| (task.order, *id))
            .collect();
        orphaned.sort_unstable_by_key(|(order, _)| *order);
        for (_, id) in orphaned {
            let Some(removed) = self.tasks.remove(&id) else {
                continue;
            };
            let siblings = removed.waiting.siblings().to_vec();
            self.instance.cancel_coroutine(id);
            for sib in siblings {
                if !self.any_waiter_awaits(sib) {
                    self.cancel(sib);
                }
            }
            let state =
                ScheduledState::Failed(self.error::<()>("Awaited task was cancelled").unwrap_err());
            self.settle_waiters(id, &state, &mut steps);
            steps.push(ScheduledStep { id, state });
        }
        let mut ready: Vec<_> = self
            .tasks
            .iter()
            .filter_map(|(id, task)| match task.waiting {
                Waiting::Ready(at) | Waiting::Timer(at) if at <= now => Some((at, task.order, *id)),
                _ => None,
            })
            .collect();
        ready.sort_by_key(|(at, order, _)| (*at, *order));
        for (_, _, id) in ready {
            if self.instance.runtime.cancellation.is_cancelled() {
                self.cancel_all();
                break;
            }
            // A sibling completing earlier in the same poll (e.g. `task_race` or
            // failed `task_all`) may have cancelled this ready task already.
            let Some(mut task) = self.tasks.remove(&id) else {
                continue;
            };
            let state = match self.instance.resume_coroutine(id, limits) {
                Ok(CoroutineState::Complete(value)) => ScheduledState::Complete(value),
                Err(error) => ScheduledState::Failed(error),
                Ok(CoroutineState::Yielded(value)) => {
                    let expects_wake = self
                        .instance
                        .runtime
                        .coroutines
                        .get(&id)
                        .is_some_and(|coroutine| coroutine.expects_wake);
                    match Self::request(value).and_then(|request| {
                        task.waiting = match &request {
                            WakeRequest::After(delay) => Waiting::Timer(
                                now.checked_add(*delay).ok_or("Timer deadline overflow")?,
                            ),
                            WakeRequest::Event(name) => Waiting::Event(name.clone()),
                            WakeRequest::Task(target) => {
                                if !expects_wake {
                                    return Err(
                                        "Yielding a task handle does not await it; use await",
                                    );
                                }
                                if self.would_cycle(id, *target) {
                                    return Err("Cyclic task await");
                                }
                                if !self.tasks.contains_key(target) {
                                    if self.instance.runtime.coroutines.contains_key(target) {
                                        self.adopt(*target)?;
                                    } else {
                                        return Err("Awaited task is not active");
                                    }
                                }
                                Waiting::Task(*target)
                            }
                            WakeRequest::All(targets) => {
                                if !expects_wake {
                                    return Err(
                                        "Yielding a task group does not await it; use await",
                                    );
                                }
                                if targets.is_empty() {
                                    if let Some(coroutine) =
                                        self.instance.runtime.coroutines.get_mut(&id)
                                    {
                                        coroutine.pending_wake = Some(Value::List(Vec::new()));
                                    }
                                    Waiting::Ready(now)
                                } else {
                                    for target in targets {
                                        if self.would_cycle(id, *target) {
                                            return Err("Cyclic task await");
                                        }
                                        if !self.tasks.contains_key(target) {
                                            if self.instance.runtime.coroutines.contains_key(target)
                                            {
                                                self.adopt(*target)?;
                                            } else {
                                                return Err("Awaited task is not active");
                                            }
                                        }
                                    }
                                    if let Some(coroutine) =
                                        self.instance.runtime.coroutines.get_mut(&id)
                                    {
                                        coroutine.pending_all = vec![None; targets.len()];
                                    }
                                    Waiting::All(targets.clone())
                                }
                            }
                            WakeRequest::Race(targets) => {
                                if !expects_wake {
                                    return Err(
                                        "Yielding a task group does not await it; use await",
                                    );
                                }
                                if targets.is_empty() {
                                    return Err("task_race requires at least one task");
                                }
                                for target in targets {
                                    if self.would_cycle(id, *target) {
                                        return Err("Cyclic task await");
                                    }
                                    if !self.tasks.contains_key(target) {
                                        if self.instance.runtime.coroutines.contains_key(target) {
                                            self.adopt(*target)?;
                                        } else {
                                            return Err("Awaited task is not active");
                                        }
                                    }
                                }
                                Waiting::Race(targets.clone())
                            }
                        };
                        task.delivers = expects_wake;
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
            if let Some(step) = steps.last()
                && matches!(
                    step.state,
                    ScheduledState::Complete(_) | ScheduledState::Failed(_)
                )
            {
                let state = step.state.clone();
                self.settle_waiters(id, &state, &mut steps);
            }
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
    pub fn emit_with_payload(&mut self, event: &str, payload: Value<'static>) -> Result<usize> {
        self.with_scheduler(|s| s.emit_with_payload(event, payload))
    }
    /// Adopt a task handle produced by an `async fn` call inside the script.
    pub fn spawn_task(&mut self, id: CoroutineId) -> Result<CoroutineId> {
        self.with_scheduler(|s| s.spawn_task(id))
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
                Waiting::Event(_) | Waiting::Task(_) | Waiting::All(_) | Waiting::Race(_) => None,
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
