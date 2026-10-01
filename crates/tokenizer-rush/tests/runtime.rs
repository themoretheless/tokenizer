use themoretheless_tokenizer_rush::{Value, evaluate};

#[test]
fn numeric_literals_preserve_f64_edges_and_lazy_errors() {
    for (source, expected) in [
        ("-0.0", -0.0_f64),
        ("5e-324", f64::from_bits(1)),
        ("1.7976931348623157e308", f64::MAX),
        ("1.25e-2", 0.0125),
        ("1e-999", 0.0),
    ] {
        let Value::Number(actual) = evaluate(source, 100).unwrap() else {
            panic!()
        };
        assert_eq!(actual.to_bits(), expected.to_bits(), "{source}");
    }
    let failure = evaluate("1e309", 100).unwrap_err();
    assert_eq!(failure.message, "Unsupported or non-finite number");
    assert_eq!((failure.span.start, failure.span.end), (0, 5));
    assert_eq!(
        evaluate("if false { 1e309 } else { 2 }", 100).unwrap(),
        Value::Number(2.0)
    );
}

#[test]
fn shared_literal_decoder_preserves_unicode_and_escape_failures() {
    for (source, expected) in [
        (r#""\u{1f642}""#, "🙂"),
        (r#""\n\r\t\\\"\'""#, "\n\r\t\\\"'"),
        ("'камера'", "камера"),
        (r#""\u{0}""#, "\0"),
    ] {
        assert_eq!(
            evaluate(source, 100).unwrap(),
            Value::String(expected.into())
        );
    }
    for source in [
        r#""\u{}""#,
        r#""\u{d800}""#,
        r#""\u{110000}""#,
        r#""\u1234""#,
        r#""\q""#,
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn escaping_closure_captures_lexical_arguments() {
    let source = "const scale = factor => x => factor * x\nconst double = scale(2)\ndouble(21)";
    assert_eq!(evaluate(source, 100).unwrap(), Value::Number(42.0));
}

#[test]
fn lexical_capture_survives_parameter_shadowing() {
    assert_eq!(
        evaluate(
            "const x = 10\nconst f = y => x + y\nconst g = x => f(x)\ng(2)",
            100
        )
        .unwrap(),
        Value::Number(12.0)
    );
}

#[test]
fn limits_errors_and_short_circuit_are_observable() {
    assert!(evaluate("const f = x => x\nf(1)", 0).is_err());
    assert!(evaluate("(x => x)()", 100).is_err());
    assert!(evaluate("1 / 0", 100).is_err());
    assert!(evaluate("unknown", 100).is_err());
    assert_eq!(
        evaluate("false and unknown", 100).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(evaluate("true or unknown", 100).unwrap(), Value::Bool(true));
}

#[test]
fn functional_pipeline_filters_maps_and_reduces() {
    let source =
        "[1, 2, 3, 4] | filter(x => x % 2 == 0) | map(x => x * x) | fold(0, (sum, x) => sum + x)";
    assert_eq!(evaluate(source, 300).unwrap(), Value::Number(20.0));
    assert_eq!(
        evaluate("[] | fold(42, (sum, x) => sum + x)", 100).unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn pipeline_prepends_input_and_respects_lexical_bindings() {
    let source = "const map = (x, y) => x - y\n10 | map(3)";
    assert_eq!(evaluate(source, 100).unwrap(), Value::Number(7.0));
    assert_eq!(
        evaluate("const twice = x => x * 2\n21 | twice", 100).unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn collection_failures_and_budget_are_propagated() {
    for source in [
        "[1] | filter(x => 1)",
        "1 | map(x => x)",
        "[] | map(1)",
        "[1] | map(x => missing)",
        "[1] | fold(0)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
    assert!(evaluate("[1, 2, 3, 4] | map(x => x * x)", 10).is_err());
}

#[test]
fn vector_geometry_and_scalar_math() {
    assert_eq!(
        evaluate("length(vec2(3, 4))", 100).unwrap(),
        Value::Number(5.0)
    );
    assert_eq!(
        evaluate("dot(vec3(1, 2, 3), vec3(4, 5, 6))", 100).unwrap(),
        Value::Number(32.0)
    );
    assert_eq!(
        evaluate("vec2(1, 2) * 3 + vec2(2, 1)", 100).unwrap(),
        Value::Vector(vec![5.0, 7.0])
    );
    assert_eq!(
        evaluate("normalize(vec2(0, 4))", 100).unwrap(),
        Value::Vector(vec![0.0, 1.0])
    );
    let Value::Number(sine) = evaluate("sin(deg(90))", 100).unwrap() else {
        panic!()
    };
    assert!((sine - 1.0).abs() < 1e-12);
    assert_eq!(evaluate("sqrt(9)", 100).unwrap(), Value::Number(3.0));
}

#[test]
fn invalid_math_is_reported() {
    for source in [
        "vec2(1)",
        "dot(vec2(1, 2), vec3(1, 2, 3))",
        "normalize(vec2(0, 0))",
        "sqrt(-1)",
        "vec2(1, 2) / 0",
        "vec2(1, 2) + vec3(1, 2, 3)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn named_functions_recurse_and_return_from_nested_blocks() {
    let source =
        "fn factorial(n) { if n <= 1 { return 1 }; return n * factorial(n - 1) }\nfactorial(6)";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(720.0));
    assert!(evaluate("fn forever() { return forever() }\nforever()", 10000).is_err());
}

#[test]
fn function_local_values_escape_in_closures() {
    let source = "fn make(a) { const b = a * 2; return x => b + x }\nconst f = make(10)\nf(3)";
    assert_eq!(evaluate(source, 100).unwrap(), Value::Number(23.0));
}

#[test]
fn match_only_evaluates_selected_arm_and_scopes_binding() {
    assert_eq!(
        evaluate("match 2 { 1 => missing, n => n * 3 }", 100).unwrap(),
        Value::Number(6.0)
    );
    assert!(evaluate("match 2 { 1 => 0 }", 100).is_err());
    assert!(evaluate("const a = match 2 { n => n }\nn", 100).is_err());
}

#[test]
fn generates_circle_points_with_functional_math() {
    let source = "range(0, 360, 90) | map(a => vec2(cos(deg(a)), sin(deg(a))) * 2)";
    let Value::List(points) = evaluate(source, 1000).unwrap() else {
        panic!()
    };
    assert_eq!(points.len(), 4);
    for (point, expected) in points
        .iter()
        .zip([[2.0, 0.0], [0.0, 2.0], [-2.0, 0.0], [0.0, -2.0]])
    {
        let Value::Vector(point) = point else {
            panic!()
        };
        assert!(
            point
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-12)
        );
    }
}

#[test]
fn interpolation_cross_product_and_range_edges() {
    assert_eq!(
        evaluate("cross(vec3(1, 0, 0), vec3(0, 1, 0))", 100).unwrap(),
        Value::Vector(vec![0.0, 0.0, 1.0])
    );
    assert_eq!(
        evaluate("lerp(vec2(0, 2), vec2(4, 6), 0.5)", 100).unwrap(),
        Value::Vector(vec![2.0, 4.0])
    );
    assert_eq!(
        evaluate("smoothstep(0, 1, 0.5)", 100).unwrap(),
        Value::Number(0.5)
    );
    assert_eq!(
        evaluate("clamp(10, 0, 1)", 100).unwrap(),
        Value::Number(1.0)
    );
    assert_eq!(
        evaluate("range(3, 0, -1)", 100).unwrap(),
        Value::List(vec![
            Value::Number(3.0),
            Value::Number(2.0),
            Value::Number(1.0)
        ])
    );
    for source in [
        "range(0, 10, 0)",
        "range(0, 1000)",
        "clamp(1, 2, 0)",
        "smoothstep(1, 1, 0)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn prepared_program_recomputes_with_fresh_inputs() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let program = Program::compile("vec2(cos(time), sin(time)) * radius").unwrap();
    let cancellation = CancellationToken::default();
    assert_eq!(
        program
            .run(100, &cancellation, &[("time", 0.0), ("radius", 2.0)])
            .unwrap(),
        Value::Vector(vec![2.0, 0.0])
    );
    assert_eq!(
        program
            .run(100, &cancellation, &[("time", 0.0), ("radius", 3.0)])
            .unwrap(),
        Value::Vector(vec![3.0, 0.0])
    );
    assert!(
        program
            .run(100, &cancellation, &[("time", f64::NAN)])
            .is_err()
    );
    cancellation.cancel();
    assert_eq!(
        program.run(100, &cancellation, &[]).unwrap_err().message,
        "Execution cancelled"
    );
}

#[test]
fn execution_can_be_cancelled_from_another_thread() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let cancellation = CancellationToken::default();
    let worker_token = cancellation.clone();
    let (ready, started) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let program = Program::compile("range(0, 1000000000)").unwrap();
        ready.send(()).unwrap();
        program
            .run(usize::MAX, &worker_token, &[])
            .unwrap_err()
            .message
    });
    started.recv().unwrap();
    cancellation.cancel();
    assert_eq!(worker.join().unwrap(), "Execution cancelled");
}

#[test]
fn piecewise_expressions_only_evaluate_selected_branch() {
    let source = "const abs = x => if x < 0 { -x } else { x }\n[-3, 2] | map(abs)";
    assert_eq!(
        evaluate(source, 200).unwrap(),
        Value::List(vec![Value::Number(3.0), Value::Number(2.0)])
    );
    assert_eq!(
        evaluate(
            "const v = if false { missing } else if true { 42 } else { missing }\nv",
            100
        )
        .unwrap(),
        Value::Number(42.0)
    );
    assert!(evaluate("const v = if 1 { 2 } else { 3 }", 100).is_err());
}

#[test]
fn vector_components_and_collection_indices() {
    assert_eq!(
        evaluate("const p = vec2(2, 3)\nvec2(p.y, -p.x)", 100).unwrap(),
        Value::Vector(vec![3.0, -2.0])
    );
    assert_eq!(
        evaluate("[vec3(1, 2, 3)][0][2]", 100).unwrap(),
        Value::Number(3.0)
    );
    for source in ["vec2(1, 2).z", "[1][-1]", "[1][0.5]", "[1][2]", "[1][true]"] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn annotations_enforce_function_and_binding_contracts() {
    let source = "fn scale(p: vec2, factor: number) -> vec2 { return p * factor }\nconst result: list[vec2] = [scale(vec2(1, 2), 3)]\nresult[0]";
    assert_eq!(
        evaluate(source, 100).unwrap(),
        Value::Vector(vec![3.0, 6.0])
    );
    for source in [
        "const x: number = true",
        "const x: list[number] = [1, false]",
        "const x: unknown = 1",
        "fn f(x: vec3) { return x }\nf(vec2(1, 2))",
        "fn f() -> number { return false }\nf()",
        "fn f() -> number { const x = 1 }\nf()",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn tuples_and_zip_combine_coordinates() {
    let source = "zip([1, 2, 3], [10, 20]) | map(pair => vec2(pair[0], pair[1]))";
    assert_eq!(
        evaluate(source, 200).unwrap(),
        Value::List(vec![
            Value::Vector(vec![1., 10.]),
            Value::Vector(vec![2., 20.])
        ])
    );
    assert_eq!(evaluate("(1, true)[1]", 100).unwrap(), Value::Bool(true));
    assert_eq!(
        evaluate("(42,)", 100).unwrap(),
        Value::Tuple(vec![Value::Number(42.)])
    );
    assert_eq!(evaluate("()", 100).unwrap(), Value::Tuple(vec![]));
    assert_eq!(evaluate("(42)", 100).unwrap(), Value::Number(42.));
    assert_eq!(evaluate("zip([], [1])", 100).unwrap(), Value::List(vec![]));
    assert!(evaluate("zip(1, [2])", 100).is_err());
}

#[test]
fn nested_tuple_bindings_are_immutable_and_shape_checked() {
    let source = "const (x, (y, _)) = (2, (3, 100))\nvec2(x, y)";
    assert_eq!(evaluate(source, 100).unwrap(), Value::Vector(vec![2., 3.]));
    for source in [
        "const (x, y) = (1,)\nx",
        "const (x, x) = (1, 2)",
        "const (x, y) = (1, 2)\nx = 3",
        "const (x, 1) = (1, 2)",
        "const (x, y) = [1, 2]",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn records_and_strings_describe_graphics_parameters() {
    let source = "const config = {radius: 20, center: vec2(1, 2), name: 'flower'}\n(config.radius, config.center.y, config.name + ' demo')";
    assert_eq!(
        evaluate(source, 100).unwrap(),
        Value::Tuple(vec![
            Value::Number(20.),
            Value::Number(2.),
            Value::String("flower demo".into())
        ])
    );
    assert_eq!(
        evaluate(r#""a\nб""#, 100).unwrap(),
        Value::String("a\nб".into())
    );
    assert_eq!(
        evaluate("{x: 1} == {x: 1}", 100).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(evaluate("true != false", 100).unwrap(), Value::Bool(true));
    for source in ["{x: 1, x: 2}", "{x: 1}.missing", r#""\q""#] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn structural_comparison_obeys_budget_and_rejects_nested_functions() {
    assert!(evaluate("const f = x => x\n[f] == [f]", 100).is_err());
    assert!(evaluate("const f = x => x\n{f: f} == {f: f}", 100).is_err());
    assert!(evaluate("const xs = range(0, 100)\nxs == xs", 150).is_err());
    assert_eq!(
        evaluate("const xs = range(0, 100)\nxs == xs", 1000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate("(1, [2, 3]) == (1, [2, 4])", 100).unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn mutable_captures_share_cells_and_runs_are_independent() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let source = "fn counter() { mut n: number = 0; return () => n += 1 }\nlet next = counter()\n(next(), next(), next())";
    let program = Program::compile(source).unwrap();
    for _ in 0..2 {
        assert_eq!(
            program
                .run(200, &CancellationToken::default(), &[])
                .unwrap(),
            Value::Tuple(vec![
                Value::Number(1.),
                Value::Number(2.),
                Value::Number(3.)
            ])
        );
    }
    assert_eq!(
        evaluate("mut x = 1\nif true { x += 2 }\nx", 100).unwrap(),
        Value::Number(3.)
    );
    assert!(evaluate("mut x: number = 1\nx = false", 100).is_err());
}

#[test]
fn loops_propagate_break_continue_and_return_correctly() {
    let source = "mut sum = 0\nfor x in range(0, 10) { if x == 2 { continue }; if x == 5 { break }; sum += x }\nsum";
    assert_eq!(evaluate(source, 500).unwrap(), Value::Number(8.));
    assert_eq!(
        evaluate("mut n = 0\nwhile n < 4 { n += 1 }\nn", 100).unwrap(),
        Value::Number(4.)
    );
    assert_eq!(
        evaluate(
            "fn find() { for x in [1, 2, 3] { if x == 2 { return x } }; return 0 }\nfind()",
            200
        )
        .unwrap(),
        Value::Number(2.)
    );
    assert!(evaluate("while true {}", 100).is_err());
    assert!(evaluate("for x in [1] {}\nx", 100).is_err());
}

#[test]
fn procedural_randomness_is_reproducible_and_noise_is_continuous() {
    let source = "range(0,20) | map(i => random(42,i))";
    let first = evaluate(source, 500).unwrap();
    assert_eq!(first, evaluate(source, 500).unwrap());
    let Value::List(samples) = first else {
        panic!()
    };
    assert!(
        samples
            .iter()
            .all(|v| matches!(v,Value::Number(x) if (0.0..1.0).contains(x)))
    );
    assert_ne!(samples[0], samples[1]);
    let Value::Number(anchor) = evaluate("random(0,0)", 100).unwrap() else {
        panic!()
    };
    assert_eq!(anchor, 0.8833108082136426);
    let Value::Number(a) = evaluate("noise(-0.000001,42)", 100).unwrap() else {
        panic!()
    };
    let Value::Number(b) = evaluate("noise(0.000001,42)", 100).unwrap() else {
        panic!()
    };
    assert!((a - b).abs() < 1e-10);
    for source in [
        "random(-1,0)",
        "random(1,0.5)",
        "noise(0,-2)",
        "noise(1e20,1)",
    ] {
        assert!(evaluate(source, 100).is_err());
    }
}

#[test]
fn option_and_result_variants_match_nested_payloads() {
    let source = "fn divide(a,b) { return if b == 0 { Err('zero') } else { Ok(a/b) } }\nmatch divide(6,2) { Ok(value) => value, Err(message) => 0 }";
    assert_eq!(evaluate(source, 200).unwrap(), Value::Number(3.));
    assert_eq!(
        evaluate("match Some(Ok(42)) { Some(Ok(n)) => n, _ => 0 }", 100).unwrap(),
        Value::Number(42.)
    );
    assert_eq!(
        evaluate("match None() { Some(n) => n, None() => 5 }", 100).unwrap(),
        Value::Number(5.)
    );
    assert!(evaluate("match Ok(1) { Err(e) => e }", 100).is_err());
    assert!(evaluate("match Some(1) { Some(x) => x }\nx", 100).is_err());
}

#[test]
fn generic_variant_and_tuple_contracts_validate_payloads() {
    let source = "fn divide(a: number,b: number) -> Result[number,string] { return if b == 0 { Err('zero') } else { Ok(a/b) } }\nconst values: tuple[Option[vec2], Result[number,string]] = (Some(vec2(1,2)), divide(6,2))\nvalues";
    assert!(evaluate(source, 200).is_ok());
    for source in [
        "const x: Option[number] = Some(false)",
        "const x: Result[number,string] = Err(1)",
        "const x: tuple[number,bool] = (1,2)",
        "const x: Option[number] = Ok(1)",
        "const x: Result[number] = Ok(1)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
    assert!(evaluate("let f = x => x\nSome(f) == Some(f)", 100).is_err());
    assert_eq!(
        evaluate("Some(1) == Some(1)", 100).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn record_patterns_select_and_rename_nested_fields() {
    let source =
        "const {radius: r, center: (x, y)} = {radius: 20, center: (1, 2), extra: true}\n(r, x, y)";
    assert_eq!(
        evaluate(source, 100).unwrap(),
        Value::Tuple(vec![
            Value::Number(20.),
            Value::Number(1.),
            Value::Number(2.)
        ])
    );
    for source in [
        "const {missing: x} = {a: 1}",
        "const {a: x, b: x} = {a: 1, b: 2}",
        "const {a: x, a: y} = {a: 1}",
        "const {a: x} = {a: 1}\nx = 2",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
}

#[test]
fn structural_match_selects_records_and_tuples_without_binding_leaks() {
    assert_eq!(
        evaluate("match (2, 3) { (0, y) => y, (x, y) => x+y }", 100).unwrap(),
        Value::Number(5.)
    );
    assert_eq!(evaluate("match {kind: 'circle', radius: 4} { {kind: 'square', side: s} => s, {kind: 'circle', radius: r} => r*r }",200).unwrap(),Value::Number(16.));
    assert_eq!(
        evaluate("match Some((1, 2)) { Some((x,y)) => x+y, _ => 0 }", 100).unwrap(),
        Value::Number(3.)
    );
    assert!(evaluate("match (1,2) { (x,x) => x }", 100).is_err());
    assert!(evaluate("match (1,2) { (x,3) => x, _ => x }", 100).is_err());
}

#[test]
fn member_errors_distinguish_unsupported_values_from_vector_fields() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    for source in ["null.missing", "null.x", "true.missing", "[1].x"] {
        let error = Program::compile(source)
            .unwrap()
            .run(100, &CancellationToken::default(), &[])
            .unwrap_err();
        assert_eq!(
            error.message, "Value does not support member access",
            "{source}"
        );
    }
    let error = Program::compile("vec2(1,2).missing")
        .unwrap()
        .run(100, &CancellationToken::default(), &[])
        .unwrap_err();
    assert_eq!(error.message, "Unknown vector component");
}
