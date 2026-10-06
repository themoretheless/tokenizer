use std::rc::Rc;
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostFunction, Program, Value, ValueType,
};

#[test]
fn depth_limit_is_per_run_and_shared_with_modules() {
    let module = Program::compile(
        "fn count(n) { if n == 0 { return 0 }; return count(n - 1) }; {run:count}",
    )
    .unwrap();
    let program = Program::compile("import helper; helper.run(4)").unwrap();
    let cancellation = CancellationToken::default();
    let modules = [("helper", &module)];
    let error = program
        .run_with_limits(
            ExecutionLimits {
                steps: 1000,
                max_depth: 4,
                max_collection_items: usize::MAX,
                max_string_bytes: usize::MAX,
            },
            &cancellation,
            &[],
            &[],
            &modules,
        )
        .unwrap_err();
    assert!(error.message.contains("limit"));
    assert_eq!(
        program
            .run_with_limits(
                ExecutionLimits::new(1000),
                &cancellation,
                &[],
                &[],
                &modules
            )
            .unwrap(),
        Value::Number(0.)
    );
    assert_eq!(
        program
            .run_with_modules(1000, &cancellation, &[], &[], &modules)
            .unwrap(),
        Value::Number(0.)
    );
    assert!(
        program
            .run_with_limits(
                ExecutionLimits {
                    steps: 1000,
                    max_depth: 65,
                    max_collection_items: usize::MAX,
                    max_string_bytes: usize::MAX,
                },
                &cancellation,
                &[],
                &[],
                &modules
            )
            .unwrap_err()
            .message
            .contains("cannot exceed 64")
    );
}

#[test]
fn zero_limits_stop_before_host_callbacks_and_cancellation_stays_sticky() {
    fn never<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        panic!("Limit must reject before invoking the host")
    }
    let functions = [Rc::new(HostFunction {
        name: "host",
        parameters: vec![],
        result: ValueType::Null,
        callback: never,
    })];
    let program = Program::compile("host()").unwrap();
    let cancellation = CancellationToken::default();
    for limits in [
        ExecutionLimits::new(0),
        ExecutionLimits {
            steps: 100,
            max_depth: 0,
            max_collection_items: usize::MAX,
            max_string_bytes: usize::MAX,
        },
    ] {
        assert!(
            program
                .run_with_limits(limits, &cancellation, &[], &functions, &[])
                .unwrap_err()
                .message
                .contains("limit")
        );
    }
    cancellation.cancel();
    assert_eq!(
        program
            .run_with_limits(
                ExecutionLimits::new(100),
                &cancellation,
                &[],
                &functions,
                &[]
            )
            .unwrap_err()
            .message,
        "Execution cancelled"
    );
}

#[test]
fn ranges_and_collect_obey_materialization_limits() {
    let limits = ExecutionLimits {
        max_collection_items: 2,
        ..ExecutionLimits::new(1000)
    };
    for source in ["range(0,3)", "range_iter(0,1000000000) | collect(3)"] {
        let error = Program::compile(source)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap_err();
        assert_eq!(error.message, "Collection item limit exceeded");
    }
    for source in ["range(0,2)", "range_iter(0,1000000000) | collect(2)"] {
        assert_eq!(
            Program::compile(source)
                .unwrap()
                .run_with_limits(limits, &Default::default(), &[], &[], &[])
                .unwrap(),
            Value::List(vec![Value::Number(0.), Value::Number(1.)])
        );
    }
    let limits = ExecutionLimits {
        max_collection_items: 0,
        ..limits
    };
    for source in ["range(0,0)", "range_iter(0,10) | collect(0)"] {
        assert_eq!(
            Program::compile(source)
                .unwrap()
                .run_with_limits(limits, &Default::default(), &[], &[], &[])
                .unwrap(),
            Value::List(vec![])
        );
    }
}

#[test]
fn literals_and_flat_map_check_size_before_growing_output() {
    let limits = ExecutionLimits {
        max_collection_items: 2,
        ..ExecutionLimits::new(1000)
    };
    for source in ["[1,2,3]", "(1,2,3)", "[1,2] | flat_map(x => [x,x])"] {
        let error = Program::compile(source)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap_err();
        assert_eq!(error.message, "Collection item limit exceeded", "{source}");
    }
    // Size is known before evaluating any literal element.
    let error = Program::compile("[1 / 0, 2, 3]")
        .unwrap()
        .run_with_limits(limits, &Default::default(), &[], &[], &[])
        .unwrap_err();
    assert_eq!(error.message, "Collection item limit exceeded");
    for source in [
        "[1,2]",
        "[1,2] | map(x => x)",
        "[1,2] | filter(x => true)",
        "[1,2] | flat_map(x => [x])",
    ] {
        assert_eq!(
            Program::compile(source)
                .unwrap()
                .run_with_limits(limits, &Default::default(), &[], &[], &[])
                .unwrap(),
            Value::List(vec![Value::Number(1.), Value::Number(2.)])
        );
    }
}

#[test]
fn record_zip_and_grid_growth_are_bounded_before_work() {
    for (source, maximum) in [
        ("{a:1/0,b:2,c:3}", 2),
        ("zip([1],[2])", 1),
        ("grid_mesh([0,1,2],[0,1,2],(x,y) => 1/0)", 8),
        // Vertex count fits, triangle count does not; callback must not run.
        ("grid_mesh([0,1,2,3],[0,1,2,3],(x,y) => 1/0)", 16),
    ] {
        let limits = ExecutionLimits {
            max_collection_items: maximum,
            ..ExecutionLimits::new(10000)
        };
        let error = Program::compile(source)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap_err();
        assert_eq!(error.message, "Collection item limit exceeded", "{source}");
    }
    let limits = ExecutionLimits {
        max_collection_items: 4,
        ..ExecutionLimits::new(10000)
    };
    let source = "grid_mesh([0,1],[0,1],(x,y) => vec3(x,y,0))";
    let Value::Mesh(mesh) = Program::compile(source)
        .unwrap()
        .run_with_limits(limits, &Default::default(), &[], &[], &[])
        .unwrap()
    else {
        panic!("Expected mesh")
    };
    assert_eq!(mesh.vertices().len(), 4);
    assert_eq!(mesh.triangles().len(), 2);
}

#[test]
fn string_limits_count_decoded_utf8_bytes_and_concatenation() {
    let limits = ExecutionLimits {
        max_string_bytes: 4,
        ..ExecutionLimits::new(10000)
    };
    for source in [r#""😀""#, r#""\u{1f600}""#, r#""é" + "é""#] {
        let value = Program::compile(source)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap();
        let Value::String(text) = value else {
            panic!("Expected string")
        };
        assert_eq!(text.len(), 4);
    }
    for source in [r#""😀x""#, r#""\u{1f600}x""#, r#""éé" + "x""#] {
        let error = Program::compile(source)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap_err();
        assert_eq!(error.message, "String byte limit exceeded", "{source}");
    }
    let limits = ExecutionLimits {
        max_string_bytes: 0,
        ..limits
    };
    assert_eq!(
        Program::compile(r#""" + """#)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap(),
        Value::String(String::new())
    );
    assert!(
        Program::compile(r#""a""#)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .is_err()
    );
}

#[test]
fn record_keys_obey_byte_limits_before_evaluating_values() {
    let limits = ExecutionLimits {
        max_string_bytes: 2,
        ..ExecutionLimits::new(1000)
    };
    for source in ["{abc:1/0}", r#"{"abc":1/0}"#] {
        let error = Program::compile(source)
            .unwrap()
            .run_with_limits(limits, &Default::default(), &[], &[], &[])
            .unwrap_err();
        assert_eq!(error.message, "String byte limit exceeded");
    }
    for source in ["{ab:42}.ab", r#"{"é":42}["é"]"#] {
        assert_eq!(
            Program::compile(source)
                .unwrap()
                .run_with_limits(limits, &Default::default(), &[], &[], &[])
                .unwrap(),
            Value::Number(42.)
        );
    }
}

#[test]
fn host_results_check_nested_sizes() {
    fn result<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        Ok(Value::List(vec![Value::String("abcd".into())]))
    }
    let functions = [Rc::new(HostFunction {
        name: "host",
        parameters: vec![],
        result: ValueType::List(Box::new(ValueType::String)),
        callback: result,
    })];
    let program = Program::compile("host()").unwrap();
    for (items, bytes, expected) in [
        (0, 4, "Collection item limit exceeded"),
        (1, 3, "String byte limit exceeded"),
    ] {
        let limits = ExecutionLimits {
            max_collection_items: items,
            max_string_bytes: bytes,
            ..ExecutionLimits::new(1000)
        };
        assert_eq!(
            program
                .run_with_limits(limits, &Default::default(), &[], &functions, &[])
                .unwrap_err()
                .message,
            expected
        );
    }
    let limits = ExecutionLimits {
        max_collection_items: 1,
        max_string_bytes: 4,
        ..ExecutionLimits::new(1000)
    };
    assert_eq!(
        program
            .run_with_limits(limits, &Default::default(), &[], &functions, &[])
            .unwrap(),
        Value::List(vec![Value::String("abcd".into())])
    );
}

#[test]
fn option_payloads_have_the_same_limits_for_host_and_script_values() {
    fn result<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        Ok(Value::Variant("Some", vec![Value::String("abc".into())]))
    }
    let functions = [Rc::new(HostFunction {
        name: "host",
        parameters: vec![],
        result: ValueType::Option(Box::new(ValueType::String)),
        callback: result,
    })];
    for source in ["host()", "Some(\"abc\")"] {
        let program = Program::compile(source).unwrap();
        let limits = ExecutionLimits {
            max_collection_items: 0,
            max_string_bytes: 3,
            ..ExecutionLimits::new(1000)
        };
        assert_eq!(
            program
                .run_with_limits(limits, &Default::default(), &[], &functions, &[])
                .unwrap(),
            Value::Variant("Some", vec![Value::String("abc".into())])
        );
        let limits = ExecutionLimits {
            max_string_bytes: 2,
            ..limits
        };
        assert_eq!(
            program
                .run_with_limits(limits, &Default::default(), &[], &functions, &[])
                .unwrap_err()
                .message,
            "String byte limit exceeded"
        );
    }
}

#[test]
fn host_result_validation_consumes_budget_and_bounds_nesting() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn wide<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        CALLS.fetch_add(1, Ordering::Relaxed);
        Ok(Value::List(vec![Value::Number(1.); 200]))
    }
    let functions = [Rc::new(HostFunction {
        name: "host",
        parameters: vec![],
        result: ValueType::List(Box::new(ValueType::Number)),
        callback: wide,
    })];
    let program = Program::compile("host()").unwrap();
    let limits = ExecutionLimits {
        max_collection_items: 200,
        ..ExecutionLimits::new(100)
    };
    let error = program
        .run_with_limits(limits, &Default::default(), &[], &functions, &[])
        .unwrap_err();
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(error.message, "Execution limit exceeded");
    assert!(
        program
            .run_with_limits(
                ExecutionLimits {
                    steps: 1000,
                    ..limits
                },
                &Default::default(),
                &[],
                &functions,
                &[]
            )
            .is_ok()
    );

    fn deep<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        let mut value = Value::Number(1.);
        for _ in 0..66 {
            value = Value::List(vec![value]);
        }
        Ok(value)
    }
    let mut result_type = ValueType::Number;
    for _ in 0..66 {
        result_type = ValueType::List(Box::new(result_type));
    }
    let functions = [Rc::new(HostFunction {
        name: "host",
        parameters: vec![],
        result: result_type,
        callback: deep,
    })];
    let limits = ExecutionLimits {
        max_collection_items: 1,
        ..ExecutionLimits::new(1000)
    };
    let error = program
        .run_with_limits(limits, &Default::default(), &[], &functions, &[])
        .unwrap_err();
    assert_eq!(error.message, "Host value nesting limit exceeded");
}

#[test]
fn cancellation_after_host_callback_precedes_result_validation() {
    fn cancelled<'s>(_: &[Value<'s>], token: &CancellationToken) -> Result<Value<'s>, String> {
        token.cancel();
        Ok(Value::String("too long".into()))
    }
    let functions = [Rc::new(HostFunction {
        name: "host",
        parameters: vec![],
        result: ValueType::String,
        callback: cancelled,
    })];
    let limits = ExecutionLimits {
        max_string_bytes: 0,
        ..ExecutionLimits::new(1000)
    };
    let token = CancellationToken::default();
    let error = Program::compile("host()")
        .unwrap()
        .run_with_limits(limits, &token, &[], &functions, &[])
        .unwrap_err();
    assert_eq!(error.message, "Execution cancelled");
    assert!(token.is_cancelled());
}
