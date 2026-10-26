use std::time::Duration;
use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineScheduler, ExecutionLimits, Program, ScheduledState, Value,
    WakeRequest,
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
    assert!(scheduler.emit("go").is_err());
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
    assert!(matches!(&steps[0].state, ScheduledState::Waiting(WakeRequest::Event(name)) if name == "go"));
    assert!(matches!(&steps[1].state, ScheduledState::Waiting(WakeRequest::After(_))));
    assert_eq!(scheduler.subscriptions("go"), 1);
    assert_eq!(scheduler.emit("go").unwrap(), 1);
    let steps = scheduler.poll(Duration::ZERO, LIMITS).unwrap();
    assert_eq!(steps.len(), 2);
    let by_id: std::collections::HashMap<_, _> =
        steps.iter().map(|s| (s.id, &s.state)).collect();
    assert!(matches!(
        by_id[&listener],
        ScheduledState::Complete(Value::Number(1.0))
    ));
    assert!(matches!(by_id[&bad], ScheduledState::Failed(_)));
    assert!(scheduler.is_empty());
}

#[test]
fn nested_await_is_a_stage_a_boundary_error() {
    let source = format!("{WAIT}async fn f() {{let x = await Wait.After(1)}}");
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
