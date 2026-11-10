use std::time::Duration;
use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineScheduler, ExecutionLimits, OwnedCoroutineScheduler, Program,
    ScheduledState, Value, WakeRequest, analyze_host_calls,
};
const LIMITS: ExecutionLimits = ExecutionLimits::new(10_000);
const WAIT: &str = "enum Wait{After(number),Event(str)}; ";
#[test]
fn timers_events_and_zero_delay_are_ordered_and_fair() {
    let source = format!(
        "{WAIT}mut log=''; fn a()->number {{yield Wait.After(2); log+='a'; yield Wait.Event('go'); log+='A'; return 1}}; fn b()->number {{yield Wait.After(1); log+='b'; yield Wait.After(0); log+='B'; return 2}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let a = scheduler.spawn("a", &[], LIMITS).unwrap();
    let b = scheduler.spawn("b", &[], LIMITS).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(scheduler.next_deadline(), Some(Duration::from_secs(1)));
    let steps = scheduler.poll(Duration::from_secs(2), LIMITS).unwrap();
    assert_eq!(steps.iter().map(|s| s.id).collect::<Vec<_>>(), vec![b, a]);
    assert_eq!(
        scheduler.instance_mut().get("log"),
        Some(Value::String("ba".into()))
    );
    assert_eq!(scheduler.subscriptions("go"), 1);
    assert_eq!(scheduler.emit("go").unwrap(), 1);
    let steps = scheduler.poll(Duration::from_secs(2), LIMITS).unwrap();
    assert_eq!(steps.iter().map(|s| s.id).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(
        scheduler.instance_mut().get("log"),
        Some(Value::String("baAB".into()))
    );
    assert!(scheduler.is_empty());
    assert_eq!(scheduler.emit("go").unwrap(), 0);
}
#[test]
fn cancellation_drop_and_broadcast_release_subscriptions() {
    let source = format!("{WAIT}fn task(){{yield Wait.Event('go'); return 1}}");
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let task;
    {
        let mut scheduler = CoroutineScheduler::new(&mut instance);
        task = scheduler.spawn("task", &[], LIMITS).unwrap();
        let other = scheduler.spawn("task", &[], LIMITS).unwrap();
        scheduler.poll(Duration::ZERO, LIMITS).unwrap();
        assert_eq!(scheduler.subscriptions("go"), 2);
        assert!(scheduler.cancel(other));
        assert_eq!(scheduler.subscriptions("go"), 1);
    }
    assert!(instance.resume_coroutine(task, LIMITS).is_err());
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("task", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    token.cancel();
    assert!(
        scheduler
            .emit_with_payload("go", Value::Number(1.))
            .is_err()
    );
    assert!(scheduler.is_empty());
}
#[test]
fn event_payload_reaches_all_awaiters_and_emit_defaults_to_null() {
    let source = format!(
        "{WAIT}async fn listener() {{let payload = await Wait.Event('go'); return payload}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("listener", &[], LIMITS).unwrap();
    scheduler.spawn("listener", &[], LIMITS).unwrap();
    assert_eq!(
        scheduler
            .emit_with_payload("go", Value::String("early".into()))
            .unwrap(),
        0
    );
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(scheduler.subscriptions("go"), 2);
    assert_eq!(
        scheduler
            .emit_with_payload("go", Value::String("value".into()))
            .unwrap(),
        2
    );
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 2);
    for step in steps {
        assert_eq!(
            step.state,
            ScheduledState::Complete(Value::String("value".into()))
        );
    }
    scheduler.spawn("listener", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(scheduler.emit("go").unwrap(), 1);
    assert_eq!(
        scheduler.poll(Duration::ZERO, LIMITS).unwrap()[0].state,
        ScheduledState::Complete(Value::Null)
    );
    assert!(scheduler.is_empty());
}

#[test]
fn bad_requests_fail_one_task_and_monotonic_time_preserves_peers() {
    let source = format!(
        "{WAIT}fn bad(){{yield Wait.After(-1)}}; fn good(){{yield Wait.After(0);return 7}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("bad", &[], LIMITS).unwrap();
    let good = scheduler.spawn("good", &[], LIMITS).unwrap();
    let steps = scheduler.poll(Duration::from_secs(2), LIMITS).unwrap();
    assert!(matches!(steps[0].state, ScheduledState::Failed(_)));
    assert_eq!(
        steps[1].state,
        ScheduledState::Waiting(WakeRequest::After(Duration::ZERO))
    );
    assert_eq!(scheduler.len(), 1);
    assert!(scheduler.poll(Duration::from_secs(1), LIMITS).is_err());
    let steps = scheduler.poll(Duration::from_secs(2), LIMITS).unwrap();
    assert_eq!(steps[0].id, good);
    assert_eq!(steps[0].state, ScheduledState::Complete(Value::Number(7.)));
}
#[test]
fn early_events_are_not_buffered_and_invalid_limits_do_not_advance_time() {
    let source = format!("{WAIT}fn task(){{yield Wait.Event('ready');return 9}}");
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("task", &[], LIMITS).unwrap();
    assert_eq!(scheduler.emit("ready").unwrap(), 0);
    let mut invalid = LIMITS;
    invalid.max_depth = 65;
    assert!(scheduler.poll(Duration::from_secs(9), invalid).is_err());
    scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(scheduler.poll(Duration::ZERO, LIMITS).unwrap().is_empty());
    assert_eq!(scheduler.emit("ready").unwrap(), 1);
    assert!(matches!(
        scheduler.poll(Duration::ZERO, LIMITS).unwrap()[0].state,
        ScheduledState::Complete(Value::Number(9.))
    ));
}
#[test]
fn cancellation_during_poll_preserves_completed_results_and_cancels_peers() {
    use std::rc::Rc;
    use themoretheless_tokenizer_rush::{HostFunction, ValueType};
    fn stop<'s>(_: &[Value<'s>], token: &CancellationToken) -> Result<Value<'s>, String> {
        token.cancel();
        Ok(Value::Number(0.))
    }
    let host = Rc::new(HostFunction {
        name: "stop",
        parameters: vec![],
        result: ValueType::Number,
        callback: stop,
    });
    let program=Program::compile("mut effects=0; fn first(){return 1}; fn second(){stop();return 2}; fn third(){effects+=1;return 3}").unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[host], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("first", &[], LIMITS).unwrap();
    scheduler.spawn("second", &[], LIMITS).unwrap();
    scheduler.spawn("third", &[], LIMITS).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].state, ScheduledState::Complete(Value::Number(1.)));
    assert!(matches!(steps[1].state, ScheduledState::Failed(_)));
    assert!(scheduler.is_empty());
    assert_eq!(
        scheduler.instance_mut().get("effects"),
        Some(Value::Number(0.))
    );
    assert!(scheduler.poll(Duration::ZERO, LIMITS).is_err());
}

#[test]
fn owned_scheduler_preserves_time_events_and_locals_between_host_calls() {
    use themoretheless_tokenizer_rush::OwnedCoroutineScheduler;
    let mut scheduler = OwnedCoroutineScheduler::new("enum Wait {After(number), Event(str)}; fn work()->number {mut n=1; yield Wait.After(0.25); n+=1; yield Wait.Event('done'); return n}", LIMITS).unwrap();
    scheduler.spawn("work", &[], LIMITS).unwrap();
    assert!(matches!(
        scheduler.poll(Duration::ZERO, LIMITS).unwrap()[0].state,
        ScheduledState::Waiting(_)
    ));
    assert!(
        scheduler
            .poll(Duration::from_millis(249), LIMITS)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        scheduler.poll(Duration::from_millis(250), LIMITS).unwrap()[0].state,
        ScheduledState::Waiting(_)
    ));
    assert_eq!(scheduler.emit("done").unwrap(), 1);
    assert_eq!(
        scheduler.poll(Duration::from_millis(250), LIMITS).unwrap()[0].state,
        ScheduledState::Complete(Value::Number(2.))
    );
    assert!(scheduler.is_empty());
}

#[test]
fn async_fn_call_returns_a_task_handle_adopted_by_the_scheduler() {
    let source = format!(
        "{WAIT}async fn worker()->number {{await Wait.After(1); return 42}}; let t = worker()"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    // The async call already ran at instantiate: it returned a handle without
    // executing the body.
    let handle = scheduler.instance_mut().get("t").unwrap();
    let id = handle.task_id().expect("task handle");
    scheduler.spawn_task(id).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 1);
    assert!(
        matches!(&steps[0].state, ScheduledState::Waiting(WakeRequest::After(d)) if *d == Duration::from_secs(1))
    );
    let steps = scheduler.poll(Duration::from_secs(1), LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Complete(Value::Number(n)) if *n == 42.0),
        "{:?}",
        steps[0].state
    );
    assert!(scheduler.is_empty());
    // Re-adopting an inactive handle is an error, not a panic.
    assert!(scheduler.spawn_task(id).is_err());
}

#[test]
fn async_tasks_spawned_by_name_await_events_and_fail_terminally() {
    let source = format!(
        "{WAIT}async fn listener()->number {{await Wait.Event('go'); return 1}}; async fn bad() {{await Wait.After(0); missing()}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let listener = scheduler.spawn("listener", &[], LIMITS).unwrap();
    let bad = scheduler.spawn("bad", &[], LIMITS).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Waiting(WakeRequest::Event(name)) if name == "go")
    );
    assert!(matches!(
        &steps[1].state,
        ScheduledState::Waiting(WakeRequest::After(_))
    ));
    assert_eq!(scheduler.subscriptions("go"), 1);
    assert_eq!(scheduler.emit("go").unwrap(), 1);
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 2);
    let by_id: std::collections::HashMap<_, _> = steps.iter().map(|s| (s.id, &s.state)).collect();
    assert!(matches!(
        by_id[&listener],
        ScheduledState::Complete(Value::Number(1.0))
    ));
    assert!(matches!(by_id[&bad], ScheduledState::Failed(_)));
    assert!(scheduler.is_empty());
}

#[test]
fn nested_await_is_a_stage_a_boundary_error() {
    // `let x = await task` is stage B; deeper positions stay a boundary.
    let source = format!("{WAIT}async fn f() {{show await Wait.After(1)}}");
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("f", &[], LIMITS).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    let ScheduledState::Failed(error) = &steps[0].state else {
        panic!("{:?}", steps[0].state)
    };
    assert!(
        error.message.contains("only supported as a statement"),
        "{}",
        error.message
    );
}

#[test]
fn async_fn_result_annotation_is_enforced() {
    // Static path: a literal mismatch is rejected at compile time, exactly as
    // for plain functions; the runtime completion check is the shared
    // resume_coroutine path covered by coroutine tests.
    let source = format!("{WAIT}async fn f()->str {{await Wait.After(0); return 1}}");
    let Err(error) = Program::compile(&source) else {
        panic!("mismatched async result compiled")
    };
    assert!(
        error.message.contains("result annotation"),
        "{}",
        error.message
    );
    // A matching annotation compiles and the value arrives at completion.
    let source = format!("{WAIT}async fn f()->str {{await Wait.After(0); return 'ok'}}");
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("f", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Complete(Value::String(s)) if s == "ok"),
        "{:?}",
        steps[0].state
    );
}

#[test]
fn await_task_delivers_the_result_and_preserves_fairness() {
    let source = format!(
        "{WAIT}async fn inner()->number {{await Wait.After(1); return 7}}; async fn outer()->number {{let x = await inner(); return x + 1}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let outer = scheduler.spawn("outer", &[], LIMITS).unwrap();
    // First step: outer suspends on the task handle; inner is auto-adopted.
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        matches!(
            &steps[0].state,
            ScheduledState::Waiting(WakeRequest::Task(_))
        ),
        "{:?}",
        steps[0].state
    );
    // Second step: inner starts and suspends on its timer.
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(matches!(
        &steps[0].state,
        ScheduledState::Waiting(WakeRequest::After(_))
    ));
    // Inner completes; outer is woken with the result but runs next poll.
    let steps = scheduler.poll(Duration::from_secs(1), LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Complete(Value::Number(n)) if *n == 7.0),
        "{:?}",
        steps[0].state
    );
    let steps = scheduler.poll(Duration::from_secs(1), LIMITS).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].id, outer);
    assert!(
        matches!(&steps[0].state, ScheduledState::Complete(Value::Number(n)) if *n == 8.0),
        "{:?}",
        steps[0].state
    );
    assert!(scheduler.is_empty());
}

#[test]
fn await_task_failure_is_terminal_and_cascades_to_waiters() {
    let source = format!(
        "{WAIT}async fn inner() {{await Wait.After(0); missing()}}; async fn middle() {{let x = await inner()}}; async fn top() {{let y = await middle()}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("top", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap(); // top awaits middle
    scheduler.poll(Duration::ZERO, LIMITS).unwrap(); // middle awaits inner
    scheduler.poll(Duration::ZERO, LIMITS).unwrap(); // inner timer
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    // inner fails; the failure terminates middle and top in the same poll,
    // after the failing task's own step.
    assert_eq!(steps.len(), 3, "{steps:?}");
    assert!(
        matches!(&steps[0].state, ScheduledState::Failed(e) if e.message.contains("missing")),
        "{:?}",
        steps[0]
    );
    assert!(
        steps
            .iter()
            .all(|s| matches!(&s.state, ScheduledState::Failed(_)))
    );
    assert!(scheduler.is_empty());
}

#[test]
fn cyclic_await_and_self_await_fail_instead_of_hanging() {
    // A cycle needs shared handles: each `a()`/`b()` call creates a fresh task.
    // `mut` cells are declared before the functions so their closures capture
    // the cells and observe the later assignments.
    let source = format!(
        "{WAIT}mut ha = null; mut hb = null; async fn a() {{let x = await hb}}; async fn b() {{let y = await ha}}; ha = a(); hb = b()"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let ha = scheduler
        .instance_mut()
        .get("ha")
        .and_then(|v| v.task_id())
        .unwrap();
    let hb = scheduler
        .instance_mut()
        .get("hb")
        .and_then(|v| v.task_id())
        .unwrap();
    scheduler.spawn_task(ha).unwrap();
    scheduler.spawn_task(hb).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        steps.iter().any(|s| matches!(&s.state, ScheduledState::Failed(e) if e.message.contains("Cyclic task await"))),
        "{:?}",
        steps
    );
    assert!(scheduler.is_empty());

    // Self-await: the task holds a handle to itself.
    let source = format!("{WAIT}mut h = null; async fn s() {{let x = await h}}; h = s()");
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let id = scheduler
        .instance_mut()
        .get("h")
        .and_then(|v| v.task_id())
        .unwrap();
    scheduler.spawn_task(id).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Failed(e) if e.message.contains("Cyclic task await")),
        "{:?}",
        steps[0].state
    );
    assert!(scheduler.is_empty());
}

#[test]
fn cancelling_the_awaited_task_cancels_waiters_and_orphans_fail() {
    // outer awaits the shared handle h (the task created at top level); the
    // `mut` cell is declared before the functions so closures observe it.
    let source = format!(
        "{WAIT}mut h = null; async fn inner() {{await Wait.Event('never')}}; async fn outer() {{let x = await h}}; h = inner()"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let h = instance.get("h").and_then(|v| v.task_id()).unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn_task(h).unwrap();
    scheduler.spawn("outer", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(scheduler.len(), 2);
    // Cancelling the awaited task cascades to the waiter.
    assert!(scheduler.cancel(h));
    assert!(scheduler.is_empty());
    drop(scheduler);

    // Direct coroutine cancellation bypassing the scheduler leaves an orphan
    // that fails on the next poll instead of hanging.
    let program = Program::compile(&source).unwrap();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let h = instance.get("h").and_then(|v| v.task_id()).unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn_task(h).unwrap();
    scheduler.spawn("outer", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(scheduler.len(), 2);
    assert!(scheduler.instance_mut().cancel_coroutine(h));
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        steps.iter().any(|s| matches!(&s.state, ScheduledState::Failed(e) if e.message.contains("Awaited task was cancelled"))),
        "{:?}",
        steps
    );
    assert!(scheduler.is_empty());
}

#[test]
fn async_closures_create_tasks_capture_and_await() {
    let source = format!(
        "{WAIT}let captured = 40; let identity = async x => x; let direct = async => captured; let awaiting = async x => await x; let awaited_task = identity(2); let direct_task = direct(); let await_task = awaiting(awaited_task)"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let direct = instance.get("direct_task").unwrap().task_id().unwrap();
    let awaiting = instance.get("await_task").unwrap().task_id().unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn_task(direct).unwrap();
    scheduler.spawn_task(awaiting).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].state, ScheduledState::Complete(Value::Number(40.)));
    assert!(
        matches!(
            steps[1].state,
            ScheduledState::Waiting(WakeRequest::Task(_))
        ),
        "{:?}",
        steps[1].state
    );
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].state, ScheduledState::Complete(Value::Number(2.)));
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].state, ScheduledState::Complete(Value::Number(2.)));
    assert!(scheduler.is_empty());
}

#[test]
fn await_of_a_plain_value_is_immediate() {
    let source = format!(
        "{WAIT}async fn f()->number {{let x = await 41; return x + 1}}; async fn g()->number {{return await 9}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("f", &[], LIMITS).unwrap();
    scheduler.spawn("g", &[], LIMITS).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Complete(Value::Number(n)) if *n == 42.0),
        "{:?}",
        steps[0].state
    );
    assert!(
        matches!(&steps[1].state, ScheduledState::Complete(Value::Number(n)) if *n == 9.0),
        "{:?}",
        steps[1].state
    );
}

#[test]
fn awaited_task_memory_limit_fails_both_tasks() {
    let source = format!(
        "{WAIT}async fn hungry() {{await Wait.After(0); range(0, 1000000) | collect(1000000)}}; async fn outer() {{let x = await hungry()}}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits {
        steps: 100_000,
        max_depth: 64,
        max_collection_items: usize::MAX,
        max_string_bytes: usize::MAX,
        max_heap_bytes: 1_000_000,
        max_allocated_bytes: usize::MAX,
    };
    let mut instance = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    scheduler.spawn("outer", &[], limits).unwrap();
    scheduler.poll(Duration::ZERO, limits).unwrap();
    scheduler.poll(Duration::ZERO, limits).unwrap();
    let steps = scheduler.poll(Duration::ZERO, limits).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Failed(e) if e.message.contains("Heap limit")),
        "{:?}",
        steps[0].state
    );
    // The waiter's failure follows in the same poll.
    assert!(matches!(&steps[1].state, ScheduledState::Failed(_)));
    assert!(scheduler.is_empty());
}

#[test]
fn task_all_preserves_input_order_handles_empty_and_cancels_siblings_on_failure() {
    let source = format!(
        "{WAIT}async fn slow() -> number {{ await Wait.After(3); return 10 }}; \
         async fn mid() -> number {{ await Wait.After(2); return 20 }}; \
         async fn fast() -> number {{ await Wait.After(1); return 30 }}; \
         async fn run_all() {{ let xs = await task_all([slow(), mid(), fast()]); return xs }}; \
         async fn run_empty() {{ let xs = await task_all([]); return xs }}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let empty_id = scheduler.spawn("run_empty", &[], LIMITS).unwrap();
    let all_id = scheduler.spawn("run_all", &[], LIMITS).unwrap();

    let mut completed = std::collections::HashMap::new();
    for tick in 0..=6 {
        for step in scheduler.poll(Duration::from_secs(tick), LIMITS).unwrap() {
            if let ScheduledState::Complete(val) = step.state {
                completed.insert(step.id, val);
            }
        }
        if scheduler.is_empty() {
            break;
        }
    }
    assert_eq!(completed.get(&empty_id), Some(&Value::List(vec![])));
    assert_eq!(
        completed.get(&all_id),
        Some(&Value::List(vec![
            Value::Number(10.),
            Value::Number(20.),
            Value::Number(30.),
        ]))
    );

    // Child failure in `task_all` cancels still-waiting siblings and cleans up subscriptions.
    let fail_source = format!(
        "{WAIT}mut post_cancel = 0; \
         async fn sleeper() {{ await Wait.Event('never'); post_cancel += 1; return 1 }}; \
         async fn boom() {{ await Wait.After(1); assert(false); return 2 }}; \
         async fn coordinator() {{ let xs = await task_all([sleeper(), boom()]); return xs }}"
    );
    let program = Program::compile(&fail_source).unwrap();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let coord = scheduler.spawn("coordinator", &[], LIMITS).unwrap();
    scheduler.poll(Duration::ZERO, LIMITS).unwrap(); // coordinator adopts children
    scheduler.poll(Duration::ZERO, LIMITS).unwrap(); // sleeper subscribes to 'never', boom sets timer
    assert_eq!(scheduler.subscriptions("never"), 1);

    let steps = scheduler.poll(Duration::from_secs(1), LIMITS).unwrap();
    assert!(
        steps
            .iter()
            .any(|s| s.id == coord && matches!(s.state, ScheduledState::Failed(_))),
        "{steps:?}"
    );
    assert_eq!(scheduler.subscriptions("never"), 0);
    assert!(scheduler.is_empty());
    assert_eq!(
        scheduler.instance_mut().get("post_cancel"),
        Some(Value::Number(0.))
    );
}

#[test]
fn task_race_returns_first_winner_cancels_losers_handles_ties_and_rejects_empty() {
    let source = format!(
        "{WAIT}mut loser_ran = 0; \
         async fn slow() -> number {{ await Wait.Event('never'); loser_ran += 1; return 99 }}; \
         async fn fast() -> number {{ await Wait.After(1); return 7 }}; \
         async fn run_race() -> number {{ return await task_race([slow(), fast()]) }}; \
         async fn tie_a() -> number {{ await Wait.After(1); return 11 }}; \
         async fn tie_b() -> number {{ await Wait.After(1); return 22 }}; \
         async fn run_tie() -> number {{ return await task_race([tie_a(), tie_b()]) }}; \
         async fn run_empty_race() {{ return await task_race([]) }}"
    );
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let race_id = scheduler.spawn("run_race", &[], LIMITS).unwrap();
    let tie_id = scheduler.spawn("run_tie", &[], LIMITS).unwrap();
    let empty_id = scheduler.spawn("run_empty_race", &[], LIMITS).unwrap();

    let mut outcomes = std::collections::HashMap::new();
    for tick in 0..=4 {
        for step in scheduler.poll(Duration::from_secs(tick), LIMITS).unwrap() {
            match step.state {
                ScheduledState::Complete(_) | ScheduledState::Failed(_) => {
                    outcomes.insert(step.id, step.state);
                }
                ScheduledState::Waiting(WakeRequest::Race(_)) => {}
                ScheduledState::Waiting(_) => {}
            }
        }
        if scheduler.is_empty() {
            break;
        }
    }
    assert_eq!(
        outcomes.get(&race_id),
        Some(&ScheduledState::Complete(Value::Number(7.)))
    );
    assert_eq!(
        outcomes.get(&tie_id),
        Some(&ScheduledState::Complete(Value::Number(11.)))
    );
    assert!(matches!(
        outcomes.get(&empty_id),
        Some(ScheduledState::Failed(_))
    ));
    assert_eq!(scheduler.subscriptions("never"), 0);
    assert!(scheduler.is_empty());
    assert_eq!(
        scheduler.instance_mut().get("loser_ran"),
        Some(Value::Number(0.))
    );
}

#[test]
fn structured_concurrency_detects_cycles_supports_owned_scheduler_and_propagates_static_shapes() {
    // Cycle detection through `task_all`.
    let cycle_src =
        format!("{WAIT}mut h = null; async fn s() {{ let xs = await task_all([h]) }}; h = s()");
    let program = Program::compile(&cycle_src).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let id = scheduler
        .instance_mut()
        .get("h")
        .and_then(|v| v.task_id())
        .unwrap();
    scheduler.spawn_task(id).unwrap();
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert!(
        matches!(&steps[0].state, ScheduledState::Failed(e) if e.message.contains("Cyclic task await")),
        "{:?}",
        steps[0].state
    );
    assert!(scheduler.is_empty());

    // `OwnedCoroutineScheduler` runs `task_all` and `task_race` end-to-end.
    let owned_src = format!(
        "{WAIT}async fn p(v: number, d: number) -> number {{ await Wait.After(d); return v }}; \
         async fn main_task() {{ \
             let xs = await task_all([p(3, 2), p(4, 1)]); \
             let w = await task_race([p(100, 5), p(xs[0] + xs[1], 1)]); \
             return w \
         }}"
    );
    let mut owned = OwnedCoroutineScheduler::new(owned_src, LIMITS).unwrap();
    let main_id = owned.spawn("main_task", &[], LIMITS).unwrap();
    let mut final_val = None;
    for tick in 0..=8 {
        for step in owned.poll(Duration::from_secs(tick), LIMITS).unwrap() {
            if step.id == main_id
                && let ScheduledState::Complete(v) = step.state
            {
                final_val = Some(v);
            }
        }
        if owned.is_empty() {
            break;
        }
    }
    assert_eq!(final_val, Some(Value::Number(7.)));

    // Static shape propagation: `await task_all([a(), b()])` has shape `list[vec3]`.
    let bad_shape = "\
async fn pt() -> vec3 { return vec3(1, 2, 3) }
async fn f() {
    let pts = await task_all([pt(), pt()])
    pts[0].w
}
";
    let parsed = analyze_host_calls(bad_shape, &[]);
    assert!(!parsed.is_valid(), "expected shape error on pts[0].w");
}
