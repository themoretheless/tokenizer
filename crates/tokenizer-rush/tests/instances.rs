use std::{cell::RefCell, rc::Rc};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostRegistration, Program, Value, ValueType,
};
fn limits() -> ExecutionLimits {
    ExecutionLimits::new(10_000)
}
#[test]
fn state_and_named_lifecycle_calls_survive_frames_and_errors() {
    let program = Program::compile("mut health = 100; mut elapsed = 0; fn start() { health -= 10 }; fn update(delta) { elapsed += delta; return elapsed }; fn fixed_update(delta) { health -= delta; return health }; fn fail() { assert(false) }").unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(limits(), &token, &[], &[], &[], &[])
        .unwrap();
    instance.call("start", &[], limits()).unwrap();
    for i in 1..=200 {
        assert_eq!(
            instance
                .call("update", &[Value::Number(0.5)], limits())
                .unwrap(),
            Value::Number(i as f64 * 0.5)
        );
    }
    assert_eq!(
        instance
            .call("fixed_update", &[Value::Number(2.0)], limits())
            .unwrap(),
        Value::Number(88.0)
    );
    assert_eq!(instance.get("health"), Some(Value::Number(88.0)));
    assert!(instance.call("fail", &[], limits()).is_err());
    assert!(instance.call("missing", &[], limits()).is_err());
    assert!(instance.call("update", &[], limits()).is_err());
    assert!(
        instance
            .call("update", &[Value::Number(1.0)], ExecutionLimits::new(0))
            .is_err()
    );
    assert_eq!(
        instance
            .call("update", &[Value::Number(1.0)], limits())
            .unwrap(),
        Value::Number(101.0)
    );
    let other = program
        .instantiate(limits(), &token, &[], &[], &[], &[])
        .unwrap();
    assert_eq!(other.get("health"), Some(Value::Number(100.0)));
    token.cancel();
    assert!(instance.call("start", &[], limits()).is_err());
}
#[test]
fn captured_host_context_and_arbitrary_inputs_and_events() {
    let queue = Rc::new(RefCell::new(Vec::new()));
    let captured = queue.clone();
    let host = HostRegistration::new(
        "emit",
        vec![ValueType::String],
        ValueType::String,
        move |args, _| {
            let Value::String(text) = &args[0] else {
                unreachable!()
            };
            captured.borrow_mut().push(text.clone());
            Ok(args[0].clone())
        },
    );
    let program =
        Program::compile("emit(scene); fn event(data) { return emit(data.message) }; scene")
            .unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(
            limits(),
            &token,
            &[("scene", Value::String("scene A".into()))],
            &[],
            &[host],
            &[],
        )
        .unwrap();
    assert_eq!(instance.initial_value(), &Value::String("scene A".into()));
    let event = Value::Record(std::collections::BTreeMap::from([(
        "message".into(),
        Value::String("hit".into()),
    )]));
    assert_eq!(
        instance.call("event", &[event], limits()).unwrap(),
        Value::String("hit".into())
    );
    assert!(
        instance
            .call(
                "event",
                &[Value::Record(std::collections::BTreeMap::from([(
                    "message".into(),
                    Value::Bool(true)
                )]))],
                limits()
            )
            .is_err()
    );
    assert_eq!(*queue.borrow(), vec!["scene A", "hit"]);
    assert_eq!(
        Program::compile("data")
            .unwrap()
            .run_with_values(
                limits(),
                &token,
                &[("data", Value::Tuple(vec![Value::Bool(true), Value::Null]))],
                &[],
                &[]
            )
            .unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Null])
    );
}

#[test]
fn captured_context_lives_until_instance_drop_and_failed_initialization_releases_it() {
    for source in ["fn event() { return read() }", "assert(false)"] {
        let state = Rc::new(42);
        let weak = Rc::downgrade(&state);
        let host = HostRegistration::new("read", vec![], ValueType::Number, move |_, _| {
            Ok(Value::Number(*state as f64))
        });
        let program = Program::compile(source).unwrap();
        let token = CancellationToken::default();
        let result =
            program.instantiate(limits(), &token, &[], &[], std::slice::from_ref(&host), &[]);
        drop(host);
        if source.starts_with("fn") {
            let mut instance = result.unwrap();
            assert!(weak.upgrade().is_some());
            assert_eq!(
                instance.call("event", &[], limits()).unwrap(),
                Value::Number(42.0)
            );
            let descriptor = instance.get("read").unwrap();
            drop(instance);
            assert!(
                weak.upgrade().is_none(),
                "A detached descriptor must not retain instance context"
            );
            drop(descriptor);
        } else {
            assert!(result.is_err());
            assert!(
                weak.upgrade().is_none(),
                "Failed initialization retained context"
            );
        }
    }
}

#[test]
fn contextual_hosts_share_analysis_contract_and_check_results_and_cancellation() {
    use themoretheless_tokenizer_rush::analyze_host_calls;
    for cancel in [false, true] {
        let host = HostRegistration::new(
            "read",
            vec![ValueType::Number],
            ValueType::Number,
            move |_, token| {
                if cancel {
                    token.cancel();
                }
                Ok(Value::Bool(true))
            },
        );
        let parsed = analyze_host_calls("read(true)", std::slice::from_ref(&host.function));
        assert!(parsed.diagnostics.iter().any(|d| d.code == "argument-type"));
        let program = Program::compile("fn event() { return read(1) }").unwrap();
        let token = CancellationToken::default();
        let mut instance = program
            .instantiate(limits(), &token, &[], &[], &[host], &[])
            .unwrap();
        let error = instance.call("event", &[], limits()).unwrap_err();
        if cancel {
            assert!(error.message.to_lowercase().contains("cancel"), "{error:?}");
        } else {
            assert_eq!(error.message, "Host function returned an invalid value");
        }
    }
}

#[test]
fn module_state_is_cached_per_instance_and_errors_restore_caller_context() {
    let initialized = Rc::new(RefCell::new(0));
    let counter = initialized.clone();
    let host = HostRegistration::new("initialized", vec![], ValueType::Number, move |_, _| {
        *counter.borrow_mut() += 1;
        Ok(Value::Number(0.0))
    });
    let library_source = "initialized(); mut count=0; fn next() { count+=1; return count }; fn fail() { count+=10; return 1/0 }; {next:next,fail:fail}";
    let library = Program::compile(library_source).unwrap();
    let source = "fn next() { import counter; return counter.next() }; fn fail() { import counter; return counter.fail() }; fn local_fail() { return 2/0 }";
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();
    let mut first = program
        .instantiate(
            limits(),
            &token,
            &[],
            &[],
            std::slice::from_ref(&host),
            &[("counter", &library)],
        )
        .unwrap();
    let mut second = program
        .instantiate(
            limits(),
            &token,
            &[],
            &[],
            &[host],
            &[("counter", &library)],
        )
        .unwrap();
    assert_eq!(
        *initialized.borrow(),
        0,
        "Imports inside functions are lazy"
    );
    for expected in 1..=3 {
        assert_eq!(
            first.call("next", &[], limits()).unwrap(),
            Value::Number(expected as f64)
        );
    }
    assert_eq!(*initialized.borrow(), 1);
    assert_eq!(
        second.call("next", &[], limits()).unwrap(),
        Value::Number(1.0)
    );
    assert_eq!(*initialized.borrow(), 2);
    let error = first.call("fail", &[], limits()).unwrap_err();
    assert_eq!(error.module.as_deref(), Some("counter"));
    assert_eq!(&library_source[error.span.start..error.span.end], "1/0");
    assert_eq!(
        first.call("next", &[], limits()).unwrap(),
        Value::Number(14.0)
    );
    assert_eq!(
        second.call("next", &[], limits()).unwrap(),
        Value::Number(2.0)
    );
    let error = first.call("local_fail", &[], limits()).unwrap_err();
    assert_eq!(error.module, None);
    assert_eq!(&source[error.span.start..error.span.end], "2/0");
    assert_eq!(
        *initialized.borrow(),
        2,
        "Failures must not reinitialize cached modules"
    );
}
