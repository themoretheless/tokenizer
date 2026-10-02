use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineState, ExecutionLimits, Program, Value,
};
const LIMITS: ExecutionLimits = ExecutionLimits::new(10_000);

#[test]
fn yields_keep_locals_and_resume_after_the_yield() {
    let program=Program::compile("mut total=0; fn work(n: number) -> number { mut local=n; yield local; local+=1; total+=local; yield local; return total }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let task = script
        .spawn_coroutine("work", &[Value::Number(4.)], LIMITS)
        .unwrap();
    assert_eq!(script.get("total"), Some(Value::Number(0.)));
    assert_eq!(
        script.resume_coroutine(task, LIMITS).unwrap(),
        CoroutineState::Yielded(Value::Number(4.))
    );
    assert_eq!(
        script.resume_coroutine(task, LIMITS).unwrap(),
        CoroutineState::Yielded(Value::Number(5.))
    );
    assert_eq!(
        script.resume_coroutine(task, LIMITS).unwrap(),
        CoroutineState::Complete(Value::Number(5.))
    );
    assert!(script.resume_coroutine(task, LIMITS).is_err());
}

#[test]
fn loops_branches_break_continue_and_shadowing_keep_correct_continuations() {
    let program=Program::compile("fn work() -> number { mut sum=0; for i in range_iter(0,5) { if i==1 { continue }; if i==4 { break }; let local=i*2; yield local; sum+=local }; mut x=0; while x<2 { x+=1; if x==1 { yield sum } }; return sum+x }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let task = script.spawn_coroutine("work", &[], LIMITS).unwrap();
    for value in [0., 4., 6., 10.] {
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(value))
        );
    }
    assert_eq!(
        script.resume_coroutine(task, LIMITS).unwrap(),
        CoroutineState::Complete(Value::Number(12.))
    );
}

#[test]
fn tasks_interleave_shared_mutations_but_keep_independent_local_state() {
    let program=Program::compile("mut total=0; fn work(n: number) -> number { mut local=n; yield local; local+=1; total+=local; return total }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let a = script
        .spawn_coroutine("work", &[Value::Number(1.)], LIMITS)
        .unwrap();
    let b = script
        .spawn_coroutine("work", &[Value::Number(10.)], LIMITS)
        .unwrap();
    assert_eq!(
        script.resume_coroutine(a, LIMITS).unwrap(),
        CoroutineState::Yielded(Value::Number(1.))
    );
    assert_eq!(
        script.resume_coroutine(b, LIMITS).unwrap(),
        CoroutineState::Yielded(Value::Number(10.))
    );
    assert_eq!(
        script.resume_coroutine(b, LIMITS).unwrap(),
        CoroutineState::Complete(Value::Number(11.))
    );
    assert_eq!(
        script.resume_coroutine(a, LIMITS).unwrap(),
        CoroutineState::Complete(Value::Number(13.))
    );
}

#[test]
fn cancellation_is_terminal_and_preserves_prior_mutations() {
    let program =
        Program::compile("mut total=0; fn work() { total+=1; yield total; total+=100 }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let task = script.spawn_coroutine("work", &[], LIMITS).unwrap();
    script.resume_coroutine(task, LIMITS).unwrap();
    assert!(script.cancel_coroutine(task));
    assert!(!script.cancel_coroutine(task));
    assert!(script.resume_coroutine(task, LIMITS).is_err());
    assert_eq!(script.get("total"), Some(Value::Number(1.)));
    let task = script.spawn_coroutine("work", &[], LIMITS).unwrap();
    token.cancel();
    assert!(
        script
            .resume_coroutine(task, LIMITS)
            .unwrap_err()
            .message
            .contains("cancelled")
    );
    assert!(!script.cancel_coroutine(task));
}

#[test]
fn runtime_errors_and_exhaustion_terminate_execution_and_question_returns_normally() {
    let program=Program::compile("fn fail() { yield 1; let x=1/0 }; fn busy() { while true {} }; fn maybe(v: Option[number]) -> Option[number] { yield 1; return Some(v?) }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let fail = script.spawn_coroutine("fail", &[], LIMITS).unwrap();
    script.resume_coroutine(fail, LIMITS).unwrap();
    assert!(script.resume_coroutine(fail, LIMITS).is_err());
    assert!(script.resume_coroutine(fail, LIMITS).is_err());
    let busy = script.spawn_coroutine("busy", &[], LIMITS).unwrap();
    assert!(
        script
            .resume_coroutine(busy, ExecutionLimits::new(20))
            .unwrap_err()
            .message
            .contains("Execution limit")
    );
    assert!(script.resume_coroutine(busy, LIMITS).is_err());
    let maybe = script
        .spawn_coroutine("maybe", &[Value::Variant("None", vec![])], LIMITS)
        .unwrap();
    script.resume_coroutine(maybe, LIMITS).unwrap();
    assert_eq!(
        script.resume_coroutine(maybe, LIMITS).unwrap(),
        CoroutineState::Complete(Value::Variant("None", vec![]))
    );
}

#[test]
fn opaque_wake_requests_use_nominal_data_and_can_observe_host_events() {
    let program=Program::compile("enum Wait { Seconds(number), Event(str) }; mut event=0; fn set_event(n: number) { event=n }; fn work() -> number { let local=10; yield Wait.Seconds(0.5); yield Wait.Event('click'); return local+event }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let task = script.spawn_coroutine("work", &[], LIMITS).unwrap();
    for expected in ["Seconds", "Event"] {
        let CoroutineState::Yielded(Value::UserData(request)) =
            script.resume_coroutine(task, LIMITS).unwrap()
        else {
            panic!()
        };
        assert_eq!(request.variant.as_deref(), Some(expected));
    }
    script
        .call("set_event", &[Value::Number(7.)], LIMITS)
        .unwrap();
    assert_eq!(
        script.resume_coroutine(task, LIMITS).unwrap(),
        CoroutineState::Complete(Value::Number(17.))
    );
}

#[test]
fn suspended_locals_are_counted_by_the_retained_data_budget() {
    let program =
        Program::compile("fn work() { let text='retained data'; yield 1; return text }").unwrap();
    let token = CancellationToken::default();
    let mut script = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let baseline = script.memory_usage();
    let task = script.spawn_coroutine("work", &[], LIMITS).unwrap();
    script.resume_coroutine(task, LIMITS).unwrap();
    assert!(script.memory_usage() > baseline);
    assert!(script.set_memory_limit(baseline).is_err());
    assert!(script.cancel_coroutine(task));
    assert_eq!(script.memory_usage(), baseline);
}

#[test]
fn task_ids_cannot_accidentally_resume_a_task_in_another_instance() {
    let program = Program::compile("fn work() { yield 1 }").unwrap();
    let token = CancellationToken::default();
    let mut a = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let mut b = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let task = a.spawn_coroutine("work", &[], LIMITS).unwrap();
    b.spawn_coroutine("work", &[], LIMITS).unwrap();
    assert!(b.resume_coroutine(task, LIMITS).is_err());
    assert!(a.resume_coroutine(task, LIMITS).is_ok());
}

#[test]
fn statically_known_suspending_calls_are_rejected_before_execution() {
    for source in [
        "fn work() {yield 1}; if false {work()}",
        "fn work() {yield 1}; let alias=work; alias()",
        "fn work(x) {yield x}; map([1],work)",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
}
