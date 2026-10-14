use themoretheless_tokenizer_rush::{Value, evaluate_bytecode};

#[test]
fn test_for_list_basic() {
    let source = "mut sum = 0\nfor x in [1, 2, 3, 4] {\n    sum += x\n}\nsum";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Number(10.0)
    );
}

#[test]
fn test_for_list_break() {
    let source =
        "mut sum = 0\nfor x in [10, 20, 30, 40] {\n    if x == 30 { break }\n    sum += x\n}\nsum";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Number(30.0)
    );
}

#[test]
fn test_for_list_continue() {
    let source = "mut sum = 0\nfor x in [1, 2, 3, 4, 5] {\n    if x % 2 == 0 { continue }\n    sum += x\n}\nsum";
    assert_eq!(evaluate_bytecode(source, 1000).unwrap(), Value::Number(9.0));
}

#[test]
fn test_for_nested() {
    let source = "mut pairs = []\nfor i in [1, 2] {\n    for j in [10, 20] {\n        pairs += [[i, j]]\n    }\n}\npairs";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::List(vec![
            Value::List(vec![Value::Number(1.0), Value::Number(10.0)]),
            Value::List(vec![Value::Number(1.0), Value::Number(20.0)]),
            Value::List(vec![Value::Number(2.0), Value::Number(10.0)]),
            Value::List(vec![Value::Number(2.0), Value::Number(20.0)]),
        ])
    );
}

#[test]
fn test_for_in_function_return() {
    let source = "fn find_first_even(xs) {\n    for x in xs {\n        if x % 2 == 0 {\n            return x\n        }\n    }\n    return -1\n}\n(find_first_even([1, 3, 4, 5]), find_first_even([1, 3, 5]))";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Tuple(vec![Value::Number(4.0), Value::Number(-1.0)])
    );
}

#[test]
fn test_range_eager_and_lazy() {
    for args in ["0, 4", "4, 0, -1", "0, 1, 0.25", "4, 0", "0, 0"] {
        let source = format!(
            "mut eager = 0\nfor x in range({args}) {{ eager += x }}\nmut lazy = 0\nfor x in range_iter({args}) {{ lazy += x }}\neager == lazy"
        );
        assert_eq!(
            evaluate_bytecode(&source, 1000).unwrap(),
            Value::Bool(true),
            "Failed for range({args})"
        );
    }
}

#[test]
fn test_range_step_negative() {
    let source = "mut sum = 0\nfor x in range_iter(10, 0, -2) {\n    sum += x\n}\nsum";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Number(30.0)
    );
}

#[test]
fn test_range_lazy_repeatable() {
    let source = "let numbers = range_iter(0, 1000000000000)\nmut sum = 0\nfor x in numbers { sum += x; if x == 3 { break } }\nfor x in numbers { sum += x; if x == 3 { break } }\nsum";
    assert_eq!(evaluate_bytecode(source, 300).unwrap(), Value::Number(12.0));
}

#[test]
fn test_range_invalid_steps_and_nonadvancing() {
    assert!(
        evaluate_bytecode("range_iter(0, 10, 0)", 100)
            .unwrap_err()
            .message
            .contains("nonzero")
    );
    let source = "for x in range_iter(10000000000000000, 10000000000000004) {}";
    assert!(
        evaluate_bytecode(source, 100)
            .unwrap_err()
            .message
            .contains("advance")
    );
    assert!(
        evaluate_bytecode(
            "for x in range_iter(10000000000000000, 10000000000000004) { break }",
            100
        )
        .is_ok()
    );
}

#[test]
fn test_for_execution_budget_limit() {
    assert!(
        evaluate_bytecode("for x in range_iter(0, 1000000000000) {}", 100)
            .unwrap_err()
            .message
            .contains("limit")
    );
}

#[test]
fn test_sequence_pipeline_map_filter_collect() {
    let source = "mut calls = 0\nfn twice(x) { calls += 1; return x * 2 }\nlet sequence = range_iter(0, 1000000000000) | map(twice) | filter(x => x >= 4)\nassert(calls == 0)\nlet result = sequence | collect(3)\n(result, calls)";
    assert_eq!(
        evaluate_bytecode(source, 500).unwrap(),
        Value::Tuple(vec![
            Value::List(vec![
                Value::Number(4.0),
                Value::Number(6.0),
                Value::Number(8.0)
            ]),
            Value::Number(5.0),
        ])
    );
}

#[test]
fn test_sequence_iter_list_and_repeated_consumption() {
    let source = "mut calls = 0\nfn visit(x) { calls += 1; return x }\nlet seq = iter([1,2,3]) | map(visit)\nfor x in seq { break }\nassert(calls == 1)\nlet first = collect(seq, 2)\nlet second = collect(seq, 2)\n(first == second, calls)";
    assert_eq!(
        evaluate_bytecode(source, 500).unwrap(),
        Value::Tuple(vec![Value::Bool(true), Value::Number(5.0)])
    );
}

#[test]
fn test_try_option_basic() {
    let source = "fn f(v) { let x = v?; return Some(x + 1) }\n(f(Some(2)), f(None()))";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Some", vec![Value::Number(3.0)]),
            Value::Variant("None", vec![])
        ])
    );
}

#[test]
fn test_try_result_basic() {
    let source = "fn f(v) { return Ok(v? + 10) }\n(f(Ok(5)), f(Err('missing')))";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Ok", vec![Value::Number(15.0)]),
            Value::Variant("Err", vec![Value::String("missing".into())])
        ])
    );
}

#[test]
fn test_try_inside_loop_early_return() {
    let source = "mut calls = 0\nfn f(v) {\n    for x in range_iter(0, 10) {\n        calls += 1\n        let y = v?\n        calls += y\n    }\n    calls += 100\n    return Some(0)\n}\nlet value = f(None())\n(value, calls)";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Tuple(vec![Value::Variant("None", vec![]), Value::Number(1.0)])
    );
}

#[test]
fn test_try_nearest_function_only() {
    let source = "fn inner(v) { return Some(v? + 1) }\nfn outer() { let ignored = inner(None()); return inner(Some(4)) }\n(outer(), inner(None()), outer())";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Some", vec![Value::Number(5.0)]),
            Value::Variant("None", vec![]),
            Value::Variant("Some", vec![Value::Number(5.0)])
        ])
    );
}

#[test]
fn test_try_postfix_member_access() {
    let source =
        "struct Point { x: number }\nfn f(v) { return Some(v?.x) }\nf(Some(Point({x: 7})))";
    assert_eq!(
        evaluate_bytecode(source, 1000).unwrap(),
        Value::Variant("Some", vec![Value::Number(7.0)])
    );
}

#[test]
fn test_try_error_on_invalid_type() {
    let source = "fn f(x) { return x? }\nf(42)";
    let err = evaluate_bytecode(source, 1000).unwrap_err();
    assert!(err.message.contains("? requires Option or Result"));
}

#[test]
fn test_for_invalid_iterable_error() {
    let source = "for x in 42 {}";
    let err = evaluate_bytecode(source, 1000).unwrap_err();
    assert!(err.message.contains("Expected a list or sequence"));
}

#[test]
fn test_for_show_stmt() {
    let source = "show 123";
    assert_eq!(
        evaluate_bytecode(source, 100).unwrap(),
        Value::Number(123.0)
    );
}

#[test]
fn test_region_basic() {
    let source = "region scratch { mut t = 40; t + 2 }";
    assert_eq!(evaluate_bytecode(source, 100).unwrap(), Value::Number(42.0));
}

#[test]
fn test_region_with_budget() {
    let source = "region scratch (1024) { 10 + 20 }";
    assert_eq!(evaluate_bytecode(source, 100).unwrap(), Value::Number(30.0));
}

#[test]
fn test_region_budget_error() {
    let source = "region scratch (-5) { 10 }";
    let err = evaluate_bytecode(source, 100).unwrap_err();
    assert!(
        err.message
            .contains("Region budget must be a non-negative integer")
    );
}

#[test]
fn test_region_escaping_closure() {
    let source = "mut g = () => 0\nregion r { mut n = 40\ng = () => n += 2 }\ng()\ng()\n";
    assert_eq!(evaluate_bytecode(source, 500).unwrap(), Value::Number(44.0));
}

#[test]
fn test_constant_folding_and_store_local() {
    use themoretheless_tokenizer_rush::bytecode::{BytecodeProgram, Opcode};
    let source = "let x = 1 + 2 * 3\nx";
    let prog = BytecodeProgram::compile(source).unwrap();
    // 1 + 2 * 3 is folded into a single constant (7.0)
    // and storing x uses StoreLocal instead of SetLocal + Pop!
    assert!(
        prog.chunk
            .code
            .iter()
            .any(|op| matches!(op, Opcode::StoreLocal(_)))
    );
    assert_eq!(prog.execute(100).unwrap(), Value::Number(7.0));
}
