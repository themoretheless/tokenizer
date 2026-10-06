use themoretheless_tokenizer_rush::{Value, evaluate};

#[test]
fn documented_type_aliases_are_executable() {
    for ty in ["number", "f64", "float"] {
        let source = format!("fn twice(x: {ty}) -> {ty} {{ return x * 2 }} twice(3)");
        assert_eq!(evaluate(&source, 100).unwrap(), Value::Number(6.));
    }
    for ty in ["str", "string"] {
        let source = format!("fn echo(x: {ty}) -> {ty} {{ return x }} echo('hello')");
        assert_eq!(
            evaluate(&source, 100).unwrap(),
            Value::String("hello".into())
        );
        let invalid = format!("const x: {ty} = 3");
        assert!(evaluate(&invalid, 100).is_err());
    }
}

#[test]
fn names_returns_and_missing_fields_have_distinct_semantics() {
    assert_eq!(
        evaluate("const x = 1\nconst $x = 2\nx + $x", 100).unwrap(),
        Value::Number(3.)
    );
    assert_eq!(evaluate("fn f() { 42 }\nf()", 100).unwrap(), Value::Null);
    assert!(
        evaluate("{x: 1}.missing", 100)
            .unwrap_err()
            .message
            .contains("Unknown field")
    );
}

#[test]
fn pipeline_evaluates_input_once() {
    assert_eq!(evaluate("mut calls = 0\nfn next() { calls += 1; return calls }\nfn twice(x) { return x * 2 }\nlet result = next() | twice\n(result, calls)", 300).unwrap(), Value::Tuple(vec![Value::Number(2.), Value::Number(1.)]));
}

#[test]
fn assertions_validate_types_and_preserve_failure_span() {
    assert_eq!(evaluate("assert(true)", 100).unwrap(), Value::Null);
    let source = "assert(false, 'wrong curve')";
    let error = evaluate(source, 100).unwrap_err();
    assert_eq!(error.message, "wrong curve");
    assert_eq!(&source[error.span.start..error.span.end], source);
    for source in [
        "assert(1)",
        "assert(true, 1)",
        "assert()",
        "assert(true, 'x', 3)",
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
    assert_eq!(evaluate("true | assert", 100).unwrap(), Value::Null);
}

#[test]
fn len_counts_collections_and_unicode_scalars() {
    assert_eq!(
        evaluate(
            "(len([1,2]), len((1,2,3)), len({a:1}), len('Привет🙂'), len('é'), len([]))",
            300
        )
        .unwrap(),
        Value::Tuple(vec![
            Value::Number(2.),
            Value::Number(3.),
            Value::Number(1.),
            Value::Number(7.),
            Value::Number(2.),
            Value::Number(0.)
        ])
    );
    assert!(evaluate("len(42)", 100).is_err());
    assert!(evaluate("len([1], [2])", 100).is_err());
    let source = format!("len('{}')", "a".repeat(1000));
    assert!(
        evaluate(&source, 100)
            .unwrap_err()
            .message
            .contains("Execution limit")
    );
}

#[test]
fn compilation_preserves_the_first_diagnostic_location_and_reason() {
    use themoretheless_tokenizer_rush::{Program, analyze};
    for source in ["const x =", "const x = 1\nx = 2", "fn f(x,x) { return x }"] {
        let parsed = analyze(source);
        let diagnostic = parsed.diagnostics.first().unwrap();
        let error = match Program::compile(source) {
            Ok(_) => panic!("invalid program accepted"),
            Err(error) => error,
        };
        assert_eq!(error.span, diagnostic.span);
        assert!(error.message.contains(diagnostic.code));
        assert!(error.message.contains(diagnostic.message));
    }
}

#[test]
fn record_indexing_uses_computed_string_keys_and_strict_missing_errors() {
    assert_eq!(
        evaluate(
            "const row = {radius:3, height:5}\n['height','radius'] | map(key => row[key])",
            200
        )
        .unwrap(),
        Value::List(vec![Value::Number(5.), Value::Number(3.)])
    );
    assert_eq!(evaluate("{label:null}['label']", 100).unwrap(), Value::Null);
    assert!(
        evaluate("{x:1}['y']", 100)
            .unwrap_err()
            .message
            .contains("Unknown field")
    );
    assert!(
        evaluate("{x:1}[0]", 100)
            .unwrap_err()
            .message
            .contains("must be a string")
    );
    assert!(evaluate("[1]['0']", 100).is_err());
}

#[test]
fn unicode_escapes_decode_scalars_and_reject_invalid_sequences() {
    assert_eq!(
        evaluate(r#""\u{41}\u{44f}\u{1F642}\u{0}""#, 100).unwrap(),
        Value::String("Aя🙂\0".into())
    );
    for source in [
        r#""\u{}""#,
        r#""\u{D800}""#,
        r#""\u{110000}""#,
        r#""\u{0000000}""#,
        r#""\u{gg}""#,
        r#""\u0041""#,
        r#""\u{41""#,
    ] {
        assert!(evaluate(source, 100).is_err(), "{source}");
    }
    assert_eq!(
        evaluate(r#"{"\u{61}": 3}['a']"#, 100).unwrap(),
        Value::Number(3.)
    );
}

#[test]
fn string_work_consumes_budget_before_allocating_results() {
    let literal = format!("'{}'", "a".repeat(1000));
    assert!(
        evaluate(&literal, 100)
            .unwrap_err()
            .message
            .contains("Execution limit")
    );
    assert!(evaluate(&literal, 1100).is_ok());
    let doubling = "mut text = 'x'\nfor i in range(0,20) { text = text + text }\nlen(text)";
    assert!(
        evaluate(doubling, 10000)
            .unwrap_err()
            .message
            .contains("Execution limit")
    );
    assert_eq!(
        evaluate("'ab' + '🙂'", 100).unwrap(),
        Value::String("ab🙂".into())
    );
}

#[test]
fn module_result_belongs_to_the_final_statement() {
    for source in [
        "",
        "42; if false { 7 }",
        "42; while false {}",
        "42; for x in [] {}",
    ] {
        assert_eq!(evaluate(source, 200).unwrap(), Value::Null, "{source}");
    }
    assert_eq!(evaluate("let x = 7", 200).unwrap(), Value::Number(7.));
    assert_eq!(
        evaluate("if true { let x = 7 }", 200).unwrap(),
        Value::Number(7.)
    );
    assert_eq!(
        evaluate("if false { 1 } else { 2 }", 200).unwrap(),
        Value::Number(2.)
    );
    assert_eq!(
        evaluate("fn f() { 42; if false { 7 } }\nf()", 200).unwrap(),
        Value::Null
    );
}

#[test]
fn import_has_null_result_on_initial_and_cached_loads() {
    use themoretheless_tokenizer_rush::{CancellationToken, Program};
    let library = Program::compile("{answer:42}").unwrap();
    for source in [
        "42; import library",
        "import library\nfn f() { 42; import library }\nf()",
        "import library\nif true { 42; import library }",
    ] {
        let main = Program::compile(source).unwrap();
        assert_eq!(
            main.run_with_modules(
                1000,
                &CancellationToken::default(),
                &[],
                &[],
                &[("library", &library)]
            )
            .unwrap(),
            Value::Null,
            "{source}"
        );
    }
}

#[test]
fn calls_evaluate_callee_then_arguments_from_left_to_right() {
    let source = "mut order = 0\nfn mark(n) { order = order * 10 + n; return n }\nfn choose() { mark(1); return (a,b) => a+b }\nlet value = choose()(mark(2), mark(3))\n(value, order)";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![Value::Number(5.), Value::Number(123.)])
    );
    let source = "mut order = 0\nfn mark(n) { order = order * 10 + n; return n }\nfn add(a,b) { return a+b }\nlet value = mark(1) | add(mark(2)) | add(mark(3))\n(value,order)";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![Value::Number(6.), Value::Number(123.)])
    );
}

#[test]
fn lexical_snapshots_mutable_cells_and_assignment_scope_are_explicit() {
    let source = "const x = 1\nconst f = () => x\nfn g() { const x = 2; return f() }\ng()";
    assert_eq!(evaluate(source, 300).unwrap(), Value::Number(1.));
    assert!(evaluate("fn f() { return later }\nlet later = 3\nf()", 300).is_err());
    for source in ["mut xs = [1]\nxs[0] = 2", "mut row = {x:1}\nrow.x = 2"] {
        assert!(
            evaluate(source, 300)
                .unwrap_err()
                .message
                .contains("Only variable assignment")
        );
    }
    assert_eq!(
        evaluate("mut x = 1\nlet f = () => x\nx = 2\nf()", 300).unwrap(),
        Value::Number(2.)
    );
}

#[test]
fn shared_environments_keep_call_frames_and_snapshots_independent() {
    let source = r#"
        fn factory(seed) {
            mut count = seed
            let read = () => count
            let next = () => count += 1
            return (read, next)
        }
        let (read_a, next_a) = factory(10)
        let (read_b, next_b) = factory(100)
        assert(next_a() == 11)
        assert(read_a() == 11)
        assert(read_b() == 100)
        assert(next_b() == 101)
        let saved = map([1,2,3], x => (() => x))
        assert(saved[0]() == 1)
        assert(saved[1]() == 2)
        assert(saved[2]() == 3)
        let outer = 7
        let snapshot = () => outer
        if true {
            let outer = 8
            assert(snapshot() == 7)
        }
        fn factorial(n) {
            if n == 0 { return 1 }
            return n * factorial(n - 1)
        }
        assert(factorial(6) == 720)
        read_a() + read_b()
    "#;
    let source = source
        .lines()
        .map(|line| line.strip_prefix("        ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let program = themoretheless_tokenizer_rush::Program::compile(&source).unwrap();
    for _ in 0..2 {
        assert_eq!(
            program.run(10000, &Default::default(), &[]).unwrap(),
            themoretheless_tokenizer_rush::Value::Number(112.)
        );
    }
}
