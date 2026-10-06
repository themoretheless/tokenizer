use themoretheless_tokenizer_rush::{ExprKind, StmtKind, analyze, parse};

#[test]
fn nested_lambdas_preserve_parameters_and_body() {
    let parsed = parse("const scale = factor => (x, y) => x * factor + y\n");
    assert!(parsed.is_valid(), "{:?}", parsed.diagnostics);
    let StmtKind::Declaration { value, .. } = &parsed.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Lambda { parameters, body } = &value.kind else {
        panic!()
    };
    assert!(matches!(&parameters[0].kind, ExprKind::Name(name) if name.text == "factor"));
    let ExprKind::Lambda { parameters, body } = &body.kind else {
        panic!()
    };
    assert_eq!(
        parameters
            .iter()
            .map(|p| match &p.kind {
                ExprKind::Name(name) => name.text,
                _ => panic!("expected a name"),
            })
            .collect::<Vec<_>>(),
        ["x", "y"]
    );
    assert!(matches!(body.kind, ExprKind::Binary { operator: "+", .. }));
}

#[test]
fn lambdas_compose_with_calls_lists_and_grouping() {
    for source in [
        "const xs = map([1, 2], x => x * 2)",
        "const f = () => 42",
        "const f = (x) => (x + 1) * 2",
        "const f = [(x, y) => x + y, z => z]",
        "const xs = [1, 2] | map(x => x * 2)",
    ] {
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{source}: {:?}", parsed.diagnostics);
    }
}

#[test]
fn lambda_parameters_are_immutable_and_scoped() {
    assert!(analyze("const x = 1\nconst f = x => x + 1").is_valid());
    assert!(!analyze("const f = x => x = 2").is_valid());
    assert!(!analyze("const x = 1\nconst f = y => x = y").is_valid());
    assert!(analyze("const f = x => x\nlet mut x = 2\nx = 3").is_valid());
}

#[test]
fn malformed_lambdas_are_rejected() {
    for source in [
        "const f = (x, x) => x",
        "const f = x =>",
        "const f = (x, 1) => x",
    ] {
        assert!(!parse(source).is_valid(), "{source}");
    }
}

#[test]
fn multiline_pipeline_continues_only_when_indented() {
    let source = "const points = [1, 2]\n    | map(x => x * 2)\n    // Continue after a comment.\n    | filter(x => x > 2)\npoints";
    let parsed = parse(source);
    assert!(parsed.is_valid(), "{:?}", parsed.diagnostics);
    assert_eq!(parsed.module.items.len(), 2);
    let StmtKind::Declaration { value, .. } = &parsed.module.items[0].kind else {
        panic!()
    };
    let ExprKind::Pipeline { stages, .. } = &value.kind else {
        panic!()
    };
    assert_eq!(stages.len(), 2);
    assert!(!parse("const x = [1]\n| map(x => x)").is_valid());
}

#[test]
fn multiline_pipeline_preserves_function_dedent() {
    let source = "fn f(xs)\n    return xs\n        | map(x => x + 1)\n        | fold(0, (a, b) => a + b)\nconst result = f([1, 2])\nresult";
    assert_eq!(
        themoretheless_tokenizer_rush::evaluate(source, 200).unwrap(),
        themoretheless_tokenizer_rush::Value::Number(5.0)
    );
}

#[test]
fn flat_map_flattens_one_level_in_source_order() {
    use themoretheless_tokenizer_rush::{Value, evaluate};
    assert_eq!(
        evaluate("[1,2] | flat_map(x => [x, x * 10])", 200).unwrap(),
        Value::List(vec![
            Value::Number(1.0),
            Value::Number(10.0),
            Value::Number(2.0),
            Value::Number(20.0)
        ])
    );
    assert_eq!(
        evaluate("[1,2] | flat_map(x => [])", 200).unwrap(),
        Value::List(vec![])
    );
    assert_eq!(
        evaluate("[1] | flat_map(x => [[x]])", 200).unwrap(),
        Value::List(vec![Value::List(vec![Value::Number(1.0)])])
    );
    assert!(
        evaluate("[1] | flat_map(x => x)", 200)
            .unwrap_err()
            .message
            .contains("must return a list")
    );
}

#[test]
fn flat_map_charges_for_expanding_existing_lists() {
    use themoretheless_tokenizer_rush::evaluate;
    let source = "const xs = range(0, 50)\nxs | flat_map(x => xs)";
    assert!(
        evaluate(source, 500)
            .unwrap_err()
            .message
            .contains("Execution limit")
    );
    let themoretheless_tokenizer_rush::Value::List(values) = evaluate(source, 10000).unwrap()
    else {
        panic!()
    };
    assert_eq!(values.len(), 2500);
    assert!(
        !themoretheless_tokenizer_rush::analyze_calls("flat_map([1])")
            .diagnostics
            .is_empty()
    );
}

#[test]
fn optional_lookup_distinguishes_missing_null_and_bad_keys() {
    use themoretheless_tokenizer_rush::{Value, evaluate};
    assert_eq!(
        evaluate("get({x:null}, 'x')", 100).unwrap(),
        Value::Variant("Some", vec![Value::Null])
    );
    for source in [
        "get({x:1}, 'y')",
        "get([], 0)",
        "get((1,), 100000000000000000000)",
    ] {
        assert_eq!(
            evaluate(source, 100).unwrap(),
            Value::Variant("None", vec![])
        );
    }
    assert_eq!(
        evaluate("match get([42],0) { Some(x) => x, None => 0 }", 100).unwrap(),
        Value::Number(42.)
    );
    for source in [
        "get([1], -1)",
        "get([1], 0.5)",
        "get([1], '0')",
        "get({}, 0)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn predicates_short_circuit_and_define_empty_results() {
    use themoretheless_tokenizer_rush::{Value, evaluate};
    assert_eq!(
        evaluate("(any([], x => true), all([], x => false))", 100).unwrap(),
        Value::Tuple(vec![Value::Bool(false), Value::Bool(true)])
    );
    assert_eq!(
        evaluate("[0,1] | any(x => x == 0 or 1 / 0 > 0)", 100).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate("[0,1] | all(x => x != 0 and 1 / 0 > 0)", 100).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(evaluate("mut calls = 0\nfn predicate(x) { calls += 1; return x > 1 }\nany([0,1,2,3], predicate)\ncalls", 300).unwrap(), Value::Number(3.));
    for source in ["any([1], x => x)", "all([], 42)", "any(1, x => true)"] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn guarded_match_uses_bindings_and_skips_unmatched_conditions() {
    use themoretheless_tokenizer_rush::{Value, analyze_names, evaluate};
    assert_eq!(
        evaluate(
            "match Some(3) { Some(x) if (x > 5) => 0, Some(y) if (y > 1) => y, _ => -1 }",
            200
        )
        .unwrap(),
        Value::Number(3.)
    );
    assert_eq!(
        evaluate(
            "match 2 { 1 if (1 / 0 > 0) => 0, x if (x > 1) => x, _ => 0 }",
            200
        )
        .unwrap(),
        Value::Number(2.)
    );
    assert!(
        evaluate("match 1 { x if (x) => x, _ => 0 }", 100)
            .unwrap_err()
            .message
            .contains("Condition must be a boolean")
    );
    assert!(
        !analyze_names(
            "match 1 { x if (x > 2) => x, y if (x > 0) => y, _ => 0 }",
            &[]
        )
        .diagnostics
        .is_empty()
    );
    assert!(
        !parse("match 1 { _ => 0, x if (true) => x }")
            .diagnostics
            .is_empty()
    );
}
