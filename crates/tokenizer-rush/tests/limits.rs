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
                max_heap_bytes: usize::MAX,
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
                    max_heap_bytes: usize::MAX,
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
            max_heap_bytes: usize::MAX,
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

use themoretheless_tokenizer_core::InputLimits;
use themoretheless_tokenizer_rush::parse_with;

/// Dense expression source: many AST nodes per source byte.
fn dense_source(statements: usize, nesting: usize) -> String {
    let mut source = String::new();
    for index in 0..statements {
        source.push_str(&format!("let v{index} = "));
        for _ in 0..nesting {
            source.push('-');
        }
        source.push_str("1\n");
    }
    source
}

#[test]
fn ast_byte_limit_rejects_inflated_parse_output() {
    // One `let v0 = ---1` line is ~12 source bytes but several boxed Expr nodes.
    let source = dense_source(50, 16);
    let generous = parse_with(&source, InputLimits::conservative().max_ast_bytes(1_000_000));
    assert!(generous.is_valid());
    let tight = parse_with(&source, InputLimits::conservative().max_ast_bytes(1024));
    assert!(!tight.is_valid());
    assert!(
        tight
            .diagnostics
            .iter()
            .any(|d| d.code == "ast-limit" && d.message == "Parse output byte limit exceeded"),
        "{:?}",
        tight.diagnostics
    );
    // The limit stops the parse early instead of building the whole tree.
    assert!(tight.module.items.len() < 50);
    // usize::MAX opts out, matching the "zero is never unlimited" convention.
    let disabled = parse_with(&source, InputLimits::conservative().max_ast_bytes(usize::MAX));
    assert!(disabled.is_valid());
}

#[test]
fn ast_byte_limit_scales_with_input_bytes_and_holds_at_one_megabyte() {
    // Default derivation: 64 parse-output bytes per input byte.
    let limits = InputLimits::conservative().max_input_bytes(4096);
    assert_eq!(limits.max_ast_bytes, 64 * 4096);
    // Measured parse-output density is 33-58 bytes per source byte
    // (docs/benchmarks/rush-memory-inventory-2026-10-24.txt, section 4), so a
    // ~1 MiB legal source must fit the conservative default with margin.
    let mut source = String::from("fn f(x) { return x * 2 + 1 }\n");
    for index in 0..40_000_usize {
        source.push_str(&format!(
            "let v{index} = f({index}) + f(v{}) / 2\n",
            index.saturating_sub(1)
        ));
    }
    assert!(source.len() > 1024 * 1024, "{}", source.len());
    let parsed = parse_with(&source, InputLimits::conservative());
    assert!(parsed.is_valid(), "{:?}", parsed.diagnostics);
    // An explicit budget below the output size rejects the same source;
    // an explicit budget above it accepts.
    let tight = parse_with(&source, InputLimits::conservative().max_ast_bytes(source.len()));
    assert!(!tight.is_valid());
    assert!(tight.diagnostics.iter().any(|d| d.code == "ast-limit"));
    let exact_style = parse_with(
        &source,
        InputLimits::conservative().max_ast_bytes(usize::MAX),
    );
    assert!(exact_style.is_valid());
}

#[test]
fn ast_byte_limit_respects_the_diagnostic_budget() {
    let source = dense_source(50, 16);
    let silent = parse_with(
        &source,
        InputLimits::conservative().max_ast_bytes(1024).max_diagnostics(0),
    );
    assert!(!silent.is_valid());
    assert!(silent.diagnostics.is_empty());
}

#[test]
fn heap_limit_bounds_eager_peaks_at_a_reproducible_boundary() {
    let program = Program::compile(
        "range(0,100000) | map(x => x * 2) | filter(x => x % 3 == 0) | fold(0, (sum,x) => sum + x)",
    )
    .unwrap();
    let token = CancellationToken::default();
    let steps = 100_000 * 100 + 100_000;
    // Off by default: the run succeeds and reports its ledger peak.
    let script = program
        .instantiate(ExecutionLimits::new(steps), &token, &[], &[], &[], &[])
        .unwrap();
    let peak = script.peak_heap_bytes();
    assert!(peak > 1_000_000, "eager peak should be ledger-visible: {peak}");
    drop(script);
    // The boundary is reproducible: exactly peak succeeds, peak - 1 fails.
    let script = program
        .instantiate(
            ExecutionLimits {
                max_heap_bytes: peak,
                ..ExecutionLimits::new(steps)
            },
            &token,
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(script.peak_heap_bytes(), peak);
    drop(script);
    let error = program
        .instantiate(
            ExecutionLimits {
                max_heap_bytes: peak - 1,
                ..ExecutionLimits::new(steps)
            },
            &token,
            &[],
            &[],
            &[],
            &[],
        )
        .err()
        .expect("peak - 1 must fail");
    assert_eq!(error.message, "Heap limit exceeded");
}

#[test]
fn heap_limit_terminates_only_the_failing_coroutine() {
    let program = Program::compile(
        "fn big() { let items = range(0, 100000); yield len(items); return 1 }; fn small() { yield 1; return 2 }",
    )
    .unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits {
        max_heap_bytes: 100_000,
        ..ExecutionLimits::new(10_000_000)
    };
    let mut script = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let steps = ExecutionLimits::new(10_000_000);
    let big = script.spawn_coroutine("big", &[], steps).unwrap();
    let small = script.spawn_coroutine("small", &[], steps).unwrap();
    // The failing task is terminated by the limit; the instance ledger
    // releases the task's reservations when its frames drop.
    let error = script.resume_coroutine(big, steps).unwrap_err();
    assert_eq!(error.message, "Heap limit exceeded");
    assert!(script.resume_coroutine(big, steps).is_err());
    // Other tasks in the same instance continue.
    assert_eq!(
        script.resume_coroutine(small, steps).unwrap(),
        themoretheless_tokenizer_rush::CoroutineState::Yielded(Value::Number(1.))
    );
    assert_eq!(
        script.resume_coroutine(small, steps).unwrap(),
        themoretheless_tokenizer_rush::CoroutineState::Complete(Value::Number(2.))
    );
}

#[test]
fn heap_limit_stops_lazy_materialization_mid_iteration_without_side_effects() {
    let program = Program::compile(
        "mut runs = 0; fn collect_many() { runs += 1; return range_iter(0, 100000) | collect(100000) | len }; fn collect_few() { return range_iter(0, 1000) | collect(1000) | len }; fn tick() { return runs }; {collect_many:collect_many, collect_few:collect_few, tick:tick}",
    )
    .unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits {
        max_heap_bytes: 200_000,
        ..ExecutionLimits::new(10_000_000)
    };
    let mut script = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let error = script.call("collect_many", &[], limits).unwrap_err();
    assert_eq!(error.message, "Heap limit exceeded");
    // No stale reservations: the same instance still runs within the budget.
    assert_eq!(
        script.call("tick", &[], limits).unwrap(),
        Value::Number(1.)
    );
    assert_eq!(
        script.call("collect_few", &[], limits).unwrap(),
        Value::Number(1000.)
    );
}

#[test]
fn heap_limit_and_retained_data_limit_are_independent() {
    let program = Program::compile(
        "mut kept = []; fn grow() { kept = range(0, 1000); return len(kept) }; {grow:grow}",
    )
    .unwrap();
    let token = CancellationToken::default();
    let steps = ExecutionLimits::new(1_000_000);
    // Retained budget tight, heap budget off: only the state limit fires.
    let mut script = program
        .instantiate(steps, &token, &[], &[], &[], &[])
        .unwrap();
    let base = script.memory_usage();
    script.set_memory_limit(base + 1000).unwrap();
    let error = script.call("grow", &[], steps).unwrap_err();
    assert_eq!(error.message, "Instance retained-data limit exceeded");
    drop(script);
    // Heap budget tight, retained budget off: only the execution limit fires.
    // The instantiate-time peak (builtins environment) must fit; grow() adds
    // a 1000-element range cover well above the headroom.
    let probe = program
        .instantiate(steps, &token, &[], &[], &[], &[])
        .unwrap();
    let baseline_peak = probe.peak_heap_bytes();
    drop(probe);
    let heap = ExecutionLimits {
        max_heap_bytes: baseline_peak + 1000,
        ..ExecutionLimits::new(1_000_000)
    };
    let mut script = program
        .instantiate(heap, &token, &[], &[], &[], &[])
        .unwrap();
    let error = script.call("grow", &[], heap).unwrap_err();
    assert_eq!(error.message, "Heap limit exceeded");
    drop(script);
    // Both off: the same call succeeds.
    let mut script = program
        .instantiate(steps, &token, &[], &[], &[], &[])
        .unwrap();
    assert_eq!(
        script.call("grow", &[], steps).unwrap(),
        Value::Number(1000.)
    );
}
