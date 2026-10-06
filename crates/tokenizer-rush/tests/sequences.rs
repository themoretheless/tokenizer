use themoretheless_tokenizer_rush::{Value, evaluate};

#[test]
fn huge_range_is_lazy_and_repeatable() {
    let source = "let numbers = range_iter(0, 1000000000000)\nmut sum = 0\nfor x in numbers { sum += x; if x == 3 { break } }\nfor x in numbers { sum += x; if x == 3 { break } }\nsum";
    assert_eq!(evaluate(source, 300).unwrap(), Value::Number(12.0));
    assert!(evaluate("range(0, 1000000000000)", 300).is_err());
}

#[test]
fn range_direction_and_exclusive_endpoint_match_lists() {
    for args in ["0, 4", "4, 0, -1", "0, 1, 0.25", "4, 0", "0, 0"] {
        let source = format!(
            "mut eager = 0\nfor x in range({args}) {{ eager += x }}\nmut lazy = 0\nfor x in range_iter({args}) {{ lazy += x }}\neager == lazy"
        );
        assert_eq!(evaluate(&source, 1000).unwrap(), Value::Bool(true));
    }
}

#[test]
fn invalid_steps_and_nonadvancing_ranges_report_errors() {
    assert!(
        evaluate("range_iter(0, 10, 0)", 100)
            .unwrap_err()
            .message
            .contains("nonzero")
    );
    let source = "for x in range_iter(10000000000000000, 10000000000000004) {}";
    assert!(
        evaluate(source, 100)
            .unwrap_err()
            .message
            .contains("advance")
    );
    // A consumer that stops at the first value never requests the invalid next value.
    assert!(
        evaluate(
            "for x in range_iter(10000000000000000, 10000000000000004) { break }",
            100
        )
        .is_ok()
    );
}

#[test]
fn consumption_obeys_execution_budget() {
    assert!(
        evaluate("for x in range_iter(0, 1000000000000) {}", 100)
            .unwrap_err()
            .message
            .contains("limit")
    );
}

#[test]
fn transformations_are_deferred_ordered_and_stop_at_limit() {
    let source = "mut calls = 0\nfn twice(x) { calls += 1; return x * 2 }\nlet sequence = range_iter(0, 1000000000000) | map(twice) | filter(x => x >= 4)\nassert(calls == 0)\nlet result = sequence | collect(3)\n(result, calls)";
    assert_eq!(
        evaluate(source, 500).unwrap(),
        Value::Tuple(vec![
            Value::List(vec![
                Value::Number(4.),
                Value::Number(6.),
                Value::Number(8.)
            ]),
            Value::Number(5.),
        ])
    );
    assert_eq!(
        evaluate("range_iter(0, 10) | map(x => 1 / 0) | collect(0)", 100).unwrap(),
        Value::List(vec![])
    );
}

#[test]
fn loops_and_repeated_consumers_restart_the_sequence() {
    let source = "mut calls = 0\nfn visit(x) { calls += 1; return x }\nlet seq = iter([1,2,3]) | map(visit)\nfor x in seq { break }\nassert(calls == 1)\nlet first = collect(seq, 2)\nlet second = collect(seq, 2)\n(first == second, calls)";
    assert_eq!(
        evaluate(source, 500).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Number(5.)])
    );
    assert_eq!(
        evaluate("iter([]) | collect(10)", 100).unwrap(),
        Value::List(vec![])
    );
    assert_eq!(
        evaluate("[1,2] | map(x => x + 1)", 100).unwrap(),
        Value::List(vec![Value::Number(2.), Value::Number(3.)])
    );
}

#[test]
fn rejected_candidates_consume_budget_and_arguments_are_checked() {
    assert!(
        evaluate(
            "range_iter(0, 1000000000000) | filter(x => false) | collect(1)",
            100
        )
        .unwrap_err()
        .message
        .contains("limit")
    );
    for source in [
        "iter(42)",
        "collect(iter([]), -1)",
        "collect(iter([]), 0.5)",
        "collect(iter([]))",
        "map(iter([]), 42)",
        "filter(iter([]), false)",
        "range_iter(0, 1) | filter(x => 1) | collect(1)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn deferred_errors_retain_module_and_source_span() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let source = "let seq = range_iter(0, 2) | filter(x => 1)\n{seq: seq}";
    let library = Program::compile(source).unwrap();
    let main = Program::compile("import library\ncollect(library.seq, 1)").unwrap();
    let error = main
        .run_with_modules(
            500,
            &CancellationToken::default(),
            &[],
            &[],
            &[("library", &library)],
        )
        .unwrap_err();
    assert_eq!(error.module.as_deref(), Some("library"));
    assert!(source[error.span.start..error.span.end].contains("filter"));
    assert!(error.message.contains("boolean"));
}

#[test]
fn cancellation_from_a_callback_stops_consumption() {
    use std::rc::Rc;
    use themoretheless_tokenizer_rush::{CancellationToken, HostFunction, Program, ValueType};
    fn cancel<'s>(_: &[Value<'s>], token: &CancellationToken) -> Result<Value<'s>, String> {
        token.cancel();
        Ok(Value::Bool(false))
    }
    let program =
        Program::compile("range_iter(0, 1000000000000) | filter(stop) | collect(1)").unwrap();
    let error = program
        .run_with_host(
            500,
            &CancellationToken::default(),
            &[],
            &[Rc::new(HostFunction {
                name: "stop",
                parameters: vec![ValueType::Number],
                result: ValueType::Bool,
                callback: cancel,
            })],
        )
        .unwrap_err();
    assert!(error.message.to_lowercase().contains("cancel"));
}

#[test]
fn reducers_consume_sequences_without_collecting() {
    assert_eq!(
        evaluate(
            "range_iter(0, 5) | map(x => x * 2) | fold(0, (a,b) => a+b)",
            300
        )
        .unwrap(),
        Value::Number(20.)
    );
    assert_eq!(
        evaluate("range_iter(0, 1000000000000) | any(x => x == 3)", 200).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate("range_iter(0, 1000000000000) | all(x => x < 3)", 200).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate("iter([]) | all(x => 1 / 0)", 100).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn borrowed_callback_arguments_can_escape_as_owned_values_and_closures() {
    assert_eq!(
        evaluate("[[1,2], [3]] | map(Some)", 200).unwrap(),
        Value::List(vec![
            Value::Variant(
                "Some",
                vec![Value::List(vec![Value::Number(1.), Value::Number(2.)])]
            ),
            Value::Variant("Some", vec![Value::List(vec![Value::Number(3.)])]),
        ])
    );
    let source = "let closures = iter([[1,2], [3]]) | map(x => () => x) | collect(2)\nclosures | map(f => f())";
    assert_eq!(
        evaluate(source, 300).unwrap(),
        Value::List(vec![
            Value::List(vec![Value::Number(1.), Value::Number(2.)]),
            Value::List(vec![Value::Number(3.)]),
        ])
    );
    assert_eq!(
        evaluate(
            "[1,2,3] | iter | map(Some) | collect(3) | map(x => match x { Some(n) => n, _ => 0 })",
            500
        )
        .unwrap(),
        Value::List(vec![
            Value::Number(1.),
            Value::Number(2.),
            Value::Number(3.)
        ])
    );
}

#[test]
fn range_steps_avoid_intermediate_overflow() {
    for (arguments, expected) in [
        ("-1e308, 1e308, 1e308", vec![-1e308, 0.]),
        ("1e308, -1e308, -1e308", vec![1e308, 0.]),
    ] {
        let expected = Value::List(expected.into_iter().map(Value::Number).collect());
        assert_eq!(
            evaluate(&format!("range({arguments})"), 100).unwrap(),
            expected
        );
        assert_eq!(
            evaluate(&format!("range_iter({arguments}) | collect(10)"), 100).unwrap(),
            expected
        );
    }
    assert_eq!(
        evaluate("range_iter(-1e308, 1.1e308, 1e308) | collect(3)", 100).unwrap(),
        Value::List(vec![
            Value::Number(-1e308),
            Value::Number(0.),
            Value::Number(1e308)
        ])
    );
    // A genuinely non-finite next point is still an error when requested.
    assert!(
        evaluate("range_iter(-1e308, 1.1e308, 1e308) | collect(4)", 100)
            .unwrap_err()
            .message
            .contains("advance")
    );
}
