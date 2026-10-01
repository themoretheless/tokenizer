use std::rc::Rc;
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostFunction, OwnedScriptInstance, Program, ScriptState,
    StateValue, Value, ValueType,
};
fn limits() -> ExecutionLimits {
    ExecutionLimits::new(1_000_000)
}

#[test]
fn budget_applies_during_initialization_and_reports_peak() {
    let program =
        Program::compile("mut health=100; let names=['one','two']; fn read(){return health}")
            .unwrap();
    let token = CancellationToken::default();
    assert!(
        program
            .instantiate_with_memory_limit(limits(), &token, &[], &[], &[], &[], 0)
            .is_err()
    );
    let first = program
        .instantiate(limits(), &token, &[], &[], &[], &[])
        .unwrap();
    let peak = first.peak_memory_usage();
    assert!(peak >= first.memory_usage());
    assert!(
        program
            .instantiate_with_memory_limit(limits(), &token, &[], &[], &[], &[], peak - 1)
            .is_err()
    );
    let second = program
        .instantiate_with_memory_limit(limits(), &token, &[], &[], &[], &[], peak)
        .unwrap();
    assert_eq!(second.peak_memory_usage(), peak);
    assert_eq!(second.get("health"), Some(Value::Number(100.)));
}

#[test]
fn owned_source_is_admitted_and_long_lived_growth_is_stopped() {
    assert!(OwnedScriptInstance::new_with_memory_limit("fn update(){}", limits(), 0).is_err());
    let source = format!(
        "mut saved=''; fn grow(){{ saved += '{}' }}",
        "x".repeat(256)
    );
    let mut script = OwnedScriptInstance::new(source.clone(), limits()).unwrap();
    let ceiling = script.memory_usage() + 4096;
    script.set_memory_limit(ceiling).unwrap();
    let mut successes = 0;
    loop {
        match script.call("grow", &[], limits()) {
            Ok(_) => successes += 1,
            Err(e) => {
                assert!(e.message.contains("memory limit"));
                break;
            }
        }
        assert!(successes < 100);
    }
    assert!(successes > 0);
    assert!(script.memory_usage() <= ceiling);
    // Peak includes allocations made before lowering the limit. Here initial
    // allocations were already smaller than the chosen ceiling.
    assert!(script.peak_memory_usage() <= ceiling);
    let saved = script.export_state(&["saved"]).unwrap();
    assert_eq!(
        saved["saved"],
        StateValue::String("x".repeat(256 * successes))
    );
    script
        .restore_state(&ScriptState::from([(
            "saved".into(),
            StateValue::String(String::new()),
        )]))
        .unwrap();
    script.call("grow", &[], limits()).unwrap();
    let init_peak = OwnedScriptInstance::new(source.clone(), limits())
        .unwrap()
        .peak_memory_usage();
    let mut admitted =
        OwnedScriptInstance::new_with_memory_limit(source.clone(), limits(), init_peak).unwrap();
    assert!(admitted.memory_usage() <= init_peak);
}

#[test]
fn temporary_collections_are_bounded_and_released_between_calls() {
    let mut script=OwnedScriptInstance::new("fn small(){let v=range(0,32); return len(v)}; fn huge(){let v=range(0,10000); return len(v)}",limits()).unwrap();
    script.call("small", &[], limits()).unwrap();
    let baseline = script.memory_usage();
    let ceiling = script.peak_memory_usage() + 2048;
    script.set_memory_limit(ceiling).unwrap();
    for _ in 0..100 {
        assert_eq!(
            script.call("small", &[], limits()).unwrap(),
            StateValue::Number(32.)
        );
    }
    assert_eq!(script.memory_usage(), baseline);
    assert!(script.call("huge", &[], limits()).is_err());
    assert_eq!(script.memory_usage(), baseline);
    assert!(script.peak_memory_usage() <= ceiling);
    assert_eq!(
        script.call("small", &[], limits()).unwrap(),
        StateValue::Number(32.)
    );
}

#[test]
fn shared_values_are_charged_once_and_aliases_do_not_copy_payloads() {
    let source = "mut data=range(0,128); mut alias=null; fn share(){alias=data}; fn clear(){data=null;alias=null}";
    let mut script = OwnedScriptInstance::new(source, limits()).unwrap();
    let before = script.memory_usage();
    script.call("share", &[], limits()).unwrap();
    assert_eq!(script.memory_usage(), before);
    script.call("clear", &[], limits()).unwrap();
    assert!(script.memory_usage() < before);
}

#[test]
fn incoming_values_and_host_results_cannot_bypass_admission() {
    let token = CancellationToken::default();
    let host = Rc::new(HostFunction {
        name: "payload",
        parameters: vec![],
        result: ValueType::String,
        callback: |_, _| Ok(Value::String("x".repeat(10000))),
    });
    let program =
        Program::compile("mut saved='old'; fn take(x){saved=x}; fn fetch(){saved=payload()}")
            .unwrap();
    let mut instance = program
        .instantiate(limits(), &token, &[], &[host], &[], &[])
        .unwrap();
    let ceiling = instance.memory_usage() + 2048;
    instance.set_memory_limit(ceiling).unwrap();
    assert!(
        instance
            .call("take", &[Value::String("x".repeat(10000))], limits())
            .is_err()
    );
    assert!(instance.call("fetch", &[], limits()).is_err());
    assert_eq!(instance.get("saved"), Some(Value::String("old".into())));
    assert!(instance.peak_memory_usage() <= ceiling);
}

#[test]
fn restores_are_atomic_when_multiple_replacements_exceed_the_budget() {
    let mut script = OwnedScriptInstance::new("mut a='old-a';mut b='old-b'", limits()).unwrap();
    let before = script.export_state(&["a", "b"]).unwrap();
    let baseline = script.memory_usage();
    script.set_memory_limit(baseline + 2048).unwrap();
    let state = ScriptState::from([
        ("a".into(), StateValue::String("x".repeat(512))),
        ("b".into(), StateValue::String("y".repeat(10000))),
    ]);
    assert!(script.restore_state(&state).is_err());
    assert_eq!(script.memory_usage(), baseline);
    assert_eq!(script.export_state(&["a", "b"]).unwrap(), before);
}

#[test]
fn value_nesting_cannot_accumulate_without_bound_between_frames() {
    let mut script =
        OwnedScriptInstance::new("mut saved=[];fn nest(){saved=[saved]}", limits()).unwrap();
    for _ in 0..63 {
        script.call("nest", &[], limits()).unwrap();
    }
    let before = script.memory_usage();
    assert!(script.call("nest", &[], limits()).is_err());
    assert_eq!(script.memory_usage(), before);
}

#[test]
fn foreign_immutable_closure_captures_are_copied_into_the_receiving_budget() {
    let token = CancellationToken::default();
    let source = "let text='abcdefghijklmnopqrstuvwxyz'; ()=>text";
    let foreign = Program::compile(source)
        .unwrap()
        .run(1000, &token, &[])
        .unwrap();
    let program =
        Program::compile("mut saved=null;fn take(x){saved=x};fn read(){return saved()}").unwrap();
    let mut instance = program
        .instantiate(limits(), &token, &[], &[], &[], &[])
        .unwrap();
    let baseline = instance.memory_usage();
    instance.set_memory_limit(baseline + 32).unwrap();
    assert!(
        instance
            .call("take", std::slice::from_ref(&foreign), limits())
            .is_err()
    );
    instance.set_memory_limit(usize::MAX).unwrap();
    instance.call("take", &[foreign], limits()).unwrap();
    assert!(instance.memory_usage() > baseline);
    assert_eq!(
        instance.call("read", &[], limits()).unwrap(),
        Value::String("abcdefghijklmnopqrstuvwxyz".into())
    );
}

#[test]
fn stored_initial_snapshot_cannot_expand_a_shared_dag_past_the_budget() {
    let program =
        Program::compile("mut saved=1;for i in range(0,24){saved=[saved,saved]};saved").unwrap();
    let token = CancellationToken::default();
    assert!(
        program
            .instantiate_with_memory_limit(limits(), &token, &[], &[], &[], &[], 128 * 1024)
            .is_err()
    );
}
