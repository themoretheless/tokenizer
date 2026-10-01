use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostRegistration, OwnedScriptInstance, ScriptState,
    StateValue, Value, ValueType,
};
fn limits() -> ExecutionLimits {
    ExecutionLimits::new(100_000)
}
#[test]
fn owns_dynamic_source_modules_and_token_and_moves_in_manager() {
    let source = String::from("fn update(delta) { import counter; return counter.next(delta) }");
    let module = String::from("mut n=0; fn next(delta) { n+=delta; return n }; {next:next}");
    let token = CancellationToken::default();
    let script = OwnedScriptInstance::with_host(
        source,
        limits(),
        token,
        vec![],
        vec![],
        vec![],
        vec![("counter".into(), module)],
    )
    .unwrap();
    let mut manager = vec![script];
    for _ in 0..100 {
        manager.push(OwnedScriptInstance::new(String::from("fn start() {}"), limits()).unwrap());
    }
    assert_eq!(
        manager[0]
            .call("update", &[StateValue::Number(0.5)], limits())
            .unwrap(),
        StateValue::Number(0.5)
    );
    assert_eq!(
        manager[0]
            .call_values("update", &[Value::Number(0.5)], limits())
            .unwrap(),
        Value::Number(1.0)
    );
    manager[0].cancellation_token().cancel();
    assert!(
        manager[0]
            .call("update", &[StateValue::Number(0.5)], limits())
            .is_err()
    );
}
#[test]
fn host_failures_include_named_stack_and_source_positions() {
    let source = "fn step() {\n  return fail()\n}\nfn update() { return step() }";
    let host = HostRegistration::new("fail", vec![], ValueType::Number, |_, _| {
        Err("scene unavailable".into())
    });
    let mut script = OwnedScriptInstance::with_host(
        source,
        limits(),
        CancellationToken::default(),
        vec![],
        vec![],
        vec![host],
        vec![],
    )
    .unwrap();
    for _ in 0..2 {
        let error = script.call("update", &[], limits()).unwrap_err();
        assert_eq!(
            error
                .stack
                .iter()
                .map(|f| f.function.as_str())
                .collect::<Vec<_>>(),
            vec!["fail", "step", "update"]
        );
        assert_eq!((error.stack[0].line, error.stack[0].column), (2, 10));
        assert_eq!(
            &source[error.stack[0].span.start..error.stack[0].span.end],
            "fail()"
        );
        assert_eq!(error.message, "scene unavailable");
    }
}
#[test]
fn nested_module_stack_retains_call_site_module() {
    let source = "fn update() { import movement; return movement.step() }";
    let module = "fn step() { return 1/0 }; {step:step}";
    let mut script = OwnedScriptInstance::with_host(
        source,
        limits(),
        CancellationToken::default(),
        vec![],
        vec![],
        vec![],
        vec![("movement".into(), module.into())],
    )
    .unwrap();
    let error = script.call("update", &[], limits()).unwrap_err();
    assert_eq!(error.module.as_deref(), Some("movement"));
    assert_eq!(&module[error.span.start..error.span.end], "1/0");
    assert_eq!(
        error
            .stack
            .iter()
            .map(|f| f.function.as_str())
            .collect::<Vec<_>>(),
        vec!["step", "update"]
    );
    assert_eq!(error.stack[0].module, None); // step() was invoked from the entry source
}
#[test]
fn selected_state_roundtrips_json_and_restores_after_reload_atomically() {
    let mut old = OwnedScriptInstance::new("mut health:number=100; mut state={name:\"npc\",items:[1,true,null]}; fn hit() { health-=7 }; fn object() {}",limits()).unwrap();
    old.call("hit", &[], limits()).unwrap();
    let save = old.export_state(&["health", "state"]).unwrap();
    let json = serde_json::to_string(&save).unwrap();
    let save: ScriptState = serde_json::from_str(&json).unwrap();
    let mut new = OwnedScriptInstance::new(
        "mut health:number=0; mut state=null; const version=2",
        limits(),
    )
    .unwrap();
    new.restore_state(&save).unwrap();
    assert_eq!(new.export_state(&["health", "state"]).unwrap(), save);
    assert!(old.export_state(&["object"]).is_err());
    let mut invalid = save.clone();
    invalid.insert("version".into(), StateValue::Number(3.0));
    assert!(new.restore_state(&invalid).is_err());
    assert_eq!(new.export_state(&["health", "state"]).unwrap(), save);
    invalid.remove("version");
    invalid.insert("health".into(), StateValue::String("oops".into()));
    assert!(new.restore_state(&invalid).is_err());
    assert_eq!(new.export_state(&["health", "state"]).unwrap(), save);
}
#[test]
fn aggregate_memory_budget_stops_growth_and_restore_and_preserves_previous_data() {
    let mut script = OwnedScriptInstance::new(
        "mut a=\"\"; mut b=\"\"; fn update() { a+=\"0123456789\"; b+=\"abcdefghij\" }",
        limits(),
    )
    .unwrap();
    let baseline = script.memory_usage();
    script.set_memory_limit(baseline + 100).unwrap();
    let mut failed = false;
    for _ in 0..30 {
        if let Err(error) = script.call("update", &[], limits()) {
            assert!(error.message.contains("memory limit"));
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(script.memory_usage() <= baseline + 100);
    let before = script.export_state(&["a", "b"]).unwrap();
    let mut too_large = before.clone();
    too_large.insert("a".into(), StateValue::String("x".repeat(1000)));
    assert!(script.restore_state(&too_large).is_err());
    assert_eq!(script.export_state(&["a", "b"]).unwrap(), before);
    assert!(script.set_memory_limit(0).is_err());
    script.set_memory_limit(usize::MAX).unwrap();
    script.call("update", &[], limits()).unwrap();
}

#[test]
fn owned_instance_drop_releases_context_and_borrowed_results_cannot_escape() {
    use std::rc::Rc;
    let context = Rc::new(String::from("scene"));
    let weak = Rc::downgrade(&context);
    let host = HostRegistration::new("scene", vec![], ValueType::String, move |_, _| {
        Ok(Value::String((*context).clone()))
    });
    let mut script = OwnedScriptInstance::with_host(
        "fn get() { return scene() }; fn closure() { return () => 1 }",
        limits(),
        CancellationToken::default(),
        vec![],
        vec![],
        vec![host],
        vec![],
    )
    .unwrap();
    assert_eq!(
        script.call_values("get", &[], limits()).unwrap(),
        Value::String("scene".into())
    );
    assert!(script.call_values("closure", &[], limits()).is_err());
    script.with_instance(|instance| {
        assert!(matches!(
            instance.call("closure", &[], limits()).unwrap(),
            Value::Function(_)
        ))
    });
    drop(script);
    assert!(weak.upgrade().is_none());
}
#[test]
fn unicode_positions_and_error_location_are_character_based() {
    let source = "fn update() { let text=\"ёж\"; return 1/0 }";
    let mut script = OwnedScriptInstance::new(source, limits()).unwrap();
    let error = script.call("update", &[], limits()).unwrap_err();
    let location = error.location.unwrap();
    assert_eq!(location.line, 1);
    assert_eq!(
        location.column,
        source[..source.find("1/0").unwrap()].chars().count() + 1
    );
}
