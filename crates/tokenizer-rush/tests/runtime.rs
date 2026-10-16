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
        Value::vec2(5.0, 7.0)
    );
    assert_eq!(
        evaluate("normalize(vec2(0, 4))", 100).unwrap(),
        Value::vec2(0.0, 1.0)
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
        Value::vec3(0.0, 0.0, 1.0)
    );
    assert_eq!(
        evaluate("lerp(vec2(0, 2), vec2(4, 6), 0.5)", 100).unwrap(),
        Value::vec2(2.0, 4.0)
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
        Value::vec2(2.0, 0.0)
    );
    assert_eq!(
        program
            .run(100, &cancellation, &[("time", 0.0), ("radius", 3.0)])
            .unwrap(),
        Value::vec2(3.0, 0.0)
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
        Value::vec2(3.0, -2.0)
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
        Value::vec2(3.0, 6.0)
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
            Value::vec2(1., 10.),
            Value::vec2(2., 20.)
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
    assert_eq!(evaluate(source, 100).unwrap(), Value::vec2(2., 3.));
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
    use themoretheless_tokenizer_rush::Program;
    for source in ["null.missing", "null.x", "true.missing", "[1].x"] {
        let error = Program::compile(source).err().unwrap();
        assert_eq!(
            error.message, "member-object: Value does not support member access",
            "{source}"
        );
    }
    let error = Program::compile("vec2(1,2).missing").err().unwrap();
    assert_eq!(
        error.message,
        "vector-component: Unknown or out-of-bounds vector component"
    );
}

#[test]
fn failed_calls_keep_builtin_names_and_unicode_source_coordinates() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let source = "fn fail() {\n let text = 'я'; assert(false)\n}\nfail()";
    let program = Program::compile(source).unwrap();
    let error = program
        .run(1000, &CancellationToken::default(), &[])
        .unwrap_err();
    assert_eq!(error.stack.len(), 2);
    assert_eq!(
        (
            &*error.stack[0].function,
            error.stack[0].line,
            error.stack[0].column
        ),
        ("Assert", 2, 18)
    );
    assert_eq!(
        (
            &*error.stack[1].function,
            error.stack[1].line,
            error.stack[1].column
        ),
        ("fail", 4, 1)
    );
    assert!(error.stack.iter().all(|frame| frame.module.is_none()));
    let location = error.location.unwrap();
    assert_eq!((location.line, location.column), (2, 18));
}

#[test]
fn compound_assignment_reads_left_before_effectful_right() {
    assert_eq!(
        evaluate("mut x=1; fn rhs(){x=10;return 2}; x+=rhs(); x", 1000).unwrap(),
        Value::Number(3.)
    );
    assert_eq!(
        evaluate("mut v=vec2(1,2); v+=vec2(3,4); v*=2; v", 1000).unwrap(),
        Value::vec2(8., 12.)
    );
    assert_eq!(
        evaluate("mut text='a'; text+='б'; text", 1000).unwrap(),
        Value::String("aб".into())
    );
}

#[test]
fn compound_assignment_keeps_expanded_expression_step_and_depth_limits() {
    use themoretheless_tokenizer_rush::{CancellationToken, ExecutionLimits, Program};
    for (compound, expanded) in [
        ("mut x=1;x+=2;x", "mut x=1;x=x+2;x"),
        ("mut x=7;x%=3;x", "mut x=7;x=x%3;x"),
        ("mut x=1;x/=0;x", "mut x=1;x=x/0;x"),
    ] {
        let a = Program::compile(compound).unwrap();
        let b = Program::compile(expanded).unwrap();
        for steps in 0..40 {
            for depth in 0..9 {
                let mut limits = ExecutionLimits::new(steps);
                limits.max_depth = depth;
                let token = CancellationToken::default();
                let a = a
                    .run_with_limits(limits, &token, &[], &[], &[])
                    .map_err(|e| e.message);
                let b = b
                    .run_with_limits(limits, &token, &[], &[], &[])
                    .map_err(|e| e.message);
                assert_eq!(a, b, "steps={steps}, depth={depth}, {compound}");
            }
        }
    }
}

#[test]
fn mutual_recursion_and_forward_declarations_work_across_evaluators() {
    use themoretheless_tokenizer_rush::{
        CancellationToken, CoroutineState, ExecutionLimits, Program, evaluate_bytecode,
    };

    let mutual_src = "\
let res = (is_even(10), is_odd(10), is_even(7), is_odd(7))
fn is_even(n) {
    if n == 0 { return true }
    return is_odd(n - 1)
}
fn is_odd(n) {
    if n == 0 { return false }
    return is_even(n - 1)
}
res";
    let expected = Value::Tuple(vec![
        Value::Bool(true),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(true),
    ]);
    let prog = Program::compile_strict(mutual_src).unwrap();
    let token = CancellationToken::default();
    assert_eq!(prog.run(10_000, &token, &[]).unwrap(), expected);
    assert_eq!(evaluate_bytecode(mutual_src, 10_000).unwrap(), expected);

    let forward_types_and_fns = "\
fn build_outer(v: number) -> Outer {
    return make_outer(Inner({ x: v }))
}
struct Outer { inner: Inner, tag: Tag }
let result = unwrap_outer(build_outer(21))
fn make_outer(inner: Inner) -> Outer {
    return Outer({ inner: inner, tag: Tag.Scaled(2) })
}
fn unwrap_outer(o: Outer) -> number {
    return match o.tag {
        Tag.Scaled(k) => o.inner.x * k,
        Tag.Empty() => 0,
    }
}
enum Tag { Scaled(number), Empty }
struct Inner { x: number }
result";
    let prog2 = Program::compile_strict(forward_types_and_fns).unwrap();
    assert_eq!(prog2.run(10_000, &token, &[]).unwrap(), Value::Number(42.0));
    assert_eq!(
        evaluate_bytecode(forward_types_and_fns, 10_000).unwrap(),
        Value::Number(42.0)
    );

    let coroutine_forward = "\
fn worker(n: number) {
    yield step_a(n)
    let bonus = 10
    fn local_add(x: number) -> number {
        return x + bonus
    }
    yield local_add(step_b(n))
}
fn step_a(x: number) -> number {
    return step_b(x) + 1
}
fn step_b(x: number) -> number {
    return x * 3
}";
    let prog3 = Program::compile(coroutine_forward).unwrap();
    let limits = ExecutionLimits::new(10_000);
    let mut instance = prog3
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let co = instance
        .spawn_coroutine("worker", &[Value::Number(4.0)], limits)
        .unwrap();
    assert_eq!(
        instance.resume_coroutine(co, limits).unwrap(),
        CoroutineState::Yielded(Value::Number(13.0))
    );
    assert_eq!(
        instance.resume_coroutine(co, limits).unwrap(),
        CoroutineState::Yielded(Value::Number(22.0))
    );
    assert_eq!(
        instance.resume_coroutine(co, limits).unwrap(),
        CoroutineState::Complete(Value::Null)
    );
}

#[test]
fn dedicated_int_type_exact_64bit_bitwise_and_indexing() {
    use themoretheless_tokenizer_rush::{
        CancellationToken, Program, evaluate, evaluate_bytecode, format_source,
    };

    let token = CancellationToken::default();

    // 1. Exact 64-bit integer precision beyond 2^53 (9007199254740992) and i64::MAX / i64::MIN
    let exact_src = "\
let big: int = 9007199254740993i
let next = big + 1i
let max_val: i64 = 9223372036854775807i64
let min_val: int = -9223372036854775808i
let bases = 0xFF_00 + 0b1010_0101 + 0o77 + 1_000i
(big, next, next - big, max_val, min_val, bases, big > 9007199254740992.0, big != 9007199254740992.0)";
    let expected_exact = Value::Tuple(vec![
        Value::Int(9_007_199_254_740_993),
        Value::Int(9_007_199_254_740_994),
        Value::Int(1),
        Value::Int(i64::MAX),
        Value::Int(i64::MIN),
        Value::Int(0xFF00 + 0b1010_0101 + 0o77 + 1_000),
        Value::Bool(true),
        Value::Bool(true),
    ]);
    let prog = Program::compile_strict(exact_src).unwrap();
    assert_eq!(prog.run(10_000, &token, &[]).unwrap(), expected_exact);
    assert_eq!(evaluate_bytecode(exact_src, 10_000).unwrap(), expected_exact);

    // 2. Integer arithmetic (truncating division, remainder, power) & bitwise ops
    let bitwise_src = "\
let mut mask: int = 0b1100_1010
mask &= 0b1111_0000
mask ^= 0b0101_0101
mask <<= 2i
mask >>= 1i
let mut state = { flags: 0x0Fi, items: [1i, 2i, 3i] }
state.flags &= 0x03i
state.items[1i] <<= 4i
let bw = (
    7i / 2i,
    -7i / 2i,
    -7i % 4i,
    2i ** 60i,
    ~0i,
    0xF0 & 0x3C,
    0xF0 ^ 0x3C,
    1i << 40i,
    1024i >> 3i,
    bit_or(0xF0, 0x0F),
    bit_and(0xFF, 0x0F),
    bit_xor(0xFF, 0x0F),
    bit_not(0i),
    shl(3i, 4i),
    shr(64i, 2i),
    mask,
    state.flags,
    state.items[1i],
)
bw";
    let expected_bw = Value::Tuple(vec![
        Value::Int(3),
        Value::Int(-3),
        Value::Int(-3),
        Value::Int(1_i64 << 60),
        Value::Int(-1),
        Value::Int(0xF0 & 0x3C),
        Value::Int(0xF0 ^ 0x3C),
        Value::Int(1_i64 << 40),
        Value::Int(128),
        Value::Int(0xFF),
        Value::Int(0x0F),
        Value::Int(0xF0),
        Value::Int(-1),
        Value::Int(48),
        Value::Int(16),
        Value::Int((((0b1100_1010_i64 & 0b1111_0000) ^ 0b0101_0101) << 2) >> 1),
        Value::Int(0x03),
        Value::Int(32),
    ]);
    let prog_bw = Program::compile_strict(bitwise_src).unwrap();
    assert_eq!(prog_bw.run(10_000, &token, &[]).unwrap(), expected_bw);
    assert_eq!(evaluate_bytecode(bitwise_src, 10_000).unwrap(), expected_bw);

    // 3. Type annotations, conversions, structs, and indexing with int
    let typed_src = "\
struct Counter { count: int }
fn step_counter(c: Counter, delta: i64) -> int {
    return c.count + delta
}
let c = Counter({ count: 40 })
let idx: int = 2
let xs: list[int] = [10, 20, 30]
let v = vec3(4, 5, 6)
(
    step_counter(c, 2),
    xs[idx],
    v[1i],
    int(3.9),
    int(-3.9),
    int('12345'),
    float(42i),
)";
    let expected_typed = Value::Tuple(vec![
        Value::Int(42),
        Value::Int(30),
        Value::Number(5.0),
        Value::Int(3),
        Value::Int(-3),
        Value::Int(12345),
        Value::Number(42.0),
    ]);
    let prog_typed = Program::compile_strict(typed_src).unwrap();
    assert_eq!(prog_typed.run(10_000, &token, &[]).unwrap(), expected_typed);
    assert_eq!(
        evaluate_bytecode(typed_src, 10_000).unwrap(),
        expected_typed
    );

    // 4. Overflow and invalid shift/division errors in both evaluators
    for bad in [
        "9223372036854775807i + 1i",
        "-9223372036854775808i - 1i",
        "let m = -9223372036854775808i\n-m",
        "1i / 0i",
        "1i % 0i",
        "2i ** -1i",
        "1i << 64i",
        "1i >> -1i",
    ] {
        assert!(evaluate(bad, 1_000).is_err(), "expected AST error for {bad}");
        assert!(
            evaluate_bytecode(bad, 1_000).is_err(),
            "expected VM error for {bad}"
        );
    }

    // 5. Formatter preserves bitwise operators and compound assignments
    let formatted = format_source("mut x: int = ~1i & 3i ^ 2i << 1i >> 1i\nx &= 1i\n").unwrap();
    assert_eq!(
        formatted,
        "mut x: int = ~ 1i & 3i ^ 2i << 1i >> 1i;\n(x &= 1i);\n"
    );
}

#[test]
fn inline_vector_fast_paths_and_constant_folding() {
    use themoretheless_tokenizer_rush::{InlineVec, evaluate_bytecode};

    let src = "\
mut v2 = vec2(1, 2) + vec2(3, 4) * 2
v2 -= vec2(1, 2)
v2 /= 2
v2.x += 0.5
mut v3 = cross(vec3(1, 0, 0), vec3(0, 1, 0))
v3 = -v3 + vec3(0, 0, 3)
let v4 = lerp(vec4(0, 0, 0, 0), vec4(2, 4, 6, 8), 0.5)
let d = dot(vec3(1, 2, 3), vec3(4, 5, 6))
let n = normalize(vec2(3, 4))
(v2, v3, v4, d, n)";
    let expected = Value::Tuple(vec![
        Value::vec2(3.5, 4.0),
        Value::vec3(0.0, 0.0, 2.0),
        Value::vec4(1.0, 2.0, 3.0, 4.0),
        Value::Number(32.0),
        Value::vec2(0.6, 0.8),
    ]);
    assert_eq!(evaluate(src, 10_000).unwrap(), expected);
    assert_eq!(evaluate_bytecode(src, 10_000).unwrap(), expected);

    let inline = InlineVec::vec3(1.0, 2.0, 3.0);
    assert_eq!(inline.len(), 3);
    assert_eq!(inline.capacity(), 3);
    assert_eq!(inline, [1.0, 2.0, 3.0]);
    assert_eq!(inline.into_iter().collect::<Vec<_>>(), vec![1.0, 2.0, 3.0]);
}

#[test]
fn slotted_record_map_and_shared_shapes() {
    use std::rc::Rc;
    use themoretheless_tokenizer_rush::{RecordMap, evaluate_bytecode};

    let src = "\
struct Point { y: number, x: number }
mut p1 = Point({ y: 20, x: 10 })
mut p2 = Point({ x: 10, y: 20 })
let eq_before = p1 == p2
p1.x += 5
p2.y -= 3
let r_const = { z: 3, a: 1, m: 2 }
mut seq = 0
let r_dyn = { b: (seq += 1), a: (seq += 10) }
(p1, p2, eq_before, r_const, r_dyn)";

    let ast_val = evaluate(src, 10_000).unwrap();
    let vm_val = evaluate_bytecode(src, 10_000).unwrap();

    for (eval_idx, val) in [&ast_val, &vm_val].into_iter().enumerate() {
        let Value::Tuple(items) = val else {
            panic!("expected tuple");
        };
        let (Value::UserData(u1), Value::UserData(u2)) = (&items[0], &items[1]) else {
            panic!("expected UserData structs");
        };
        let (Some(Value::Record(r1)), Some(Value::Record(r2))) =
            (u1.values.first(), u2.values.first())
        else {
            panic!("expected struct payload records");
        };
        if eval_idx == 0 {
            // In AST, struct constructor assigns the shared struct RecordShape to every instance
            assert!(Rc::ptr_eq(r1.shape(), r2.shape()));
        }
        assert_eq!(r1.keys_slice(), &["x".to_string(), "y".to_string()]);
        assert_eq!(r1.values_slice(), &[Value::Number(15.0), Value::Number(20.0)]);
        assert_eq!(r2.values_slice(), &[Value::Number(10.0), Value::Number(17.0)]);
        assert_eq!(items[2], Value::Bool(true));

        let expected_const = Value::Record(RecordMap::from([
            ("a".to_string(), Value::Number(1.0)),
            ("m".to_string(), Value::Number(2.0)),
            ("z".to_string(), Value::Number(3.0)),
        ]));
        assert_eq!(items[3], expected_const);

        // Unsorted source fields { b: (seq += 1), a: (seq += 10) } evaluate left-to-right (b=1, a=11)
        // and store in sorted slot order ("a": 11, "b": 1).
        let expected_dyn = Value::Record(RecordMap::from([
            ("a".to_string(), Value::Number(11.0)),
            ("b".to_string(), Value::Number(1.0)),
        ]));
        assert_eq!(items[4], expected_dyn);
    }
}

#[test]
fn zero_clone_local_access_and_int_superinstructions() {
    use themoretheless_tokenizer_rush::bytecode::{BytecodeProgram, Opcode};
    use themoretheless_tokenizer_rush::evaluate_bytecode;

    let src = "\
struct State { count: int, acc: int }
mut st = State({ count: 0i64, acc: 0i64 })
mut arr = [10i64, 20i64, 30i64, 40i64]
fn step(k: int) -> int {
    st.count = st.count + 1i64
    arr[0i64] = arr[0i64] + (k & 0xff)
    return st.acc + arr[k % 4i64]
}
mut i = 0i64
mut hash = 0i64
while i < 32i64 {
    hash += ((i * 3i64 - 1i64) ^ 0x5a) << 2i64 >> 1i64
    st.acc = step(i)
    i += 1i64
}
(st.count, st.acc, arr[0i64], hash)";

    let prog = BytecodeProgram::compile(src).unwrap();
    // Verify zero-clone local member/index and int superinstructions are emitted in top-level chunk
    assert!(prog
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::LocalLessIntImm(_, 32))));
    assert!(prog
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::StoreAddLocalIntImm(_, 1))));
    assert!(prog
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::StoreMemberSet(_, _))));
    assert!(prog
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::LocalMemberGet(_, _))));
    assert!(prog
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::LocalIndexGet(_))));

    // Verify upvalue member/index superinstructions inside closure `step`
    let step_fn = &prog.chunk.functions[1];
    assert!(step_fn
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::UpvalueMemberGet(_, _))));
    assert!(step_fn
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::UpvalueIndexGet(_))));
    assert!(step_fn
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::LocalBitAndIntImm(_, 0xff))));
    assert!(step_fn
        .chunk
        .code
        .iter()
        .any(|op| matches!(op, Opcode::LocalModIntImm(_, 4))));

    let ast_val = evaluate(src, 100_000).unwrap();
    let vm_val = evaluate_bytecode(src, 100_000).unwrap();
    assert_eq!(ast_val, vm_val);
}
