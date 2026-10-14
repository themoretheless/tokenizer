use themoretheless_tokenizer_rush::{Value, evaluate, evaluate_bytecode};

#[test]
fn test_destructure_tuple_basic() {
    let code = "const (a, b) = (10, 20)\na + b";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(30.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_destructure_tuple_nested() {
    let code = "const (x, (y, z)) = (1, (2, 3))\nx * 100 + y * 10 + z";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(123.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_destructure_tuple_wildcard() {
    let code = "const (first, _) = (42, 99)\nfirst";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(42.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_destructure_record_basic() {
    let code = "const { x: a, y: b } = { x: 5, y: 7 }\na * b";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(35.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_destructure_tuple_mismatch_error() {
    let code = "const (a, b) = (1, 2, 3)\na + b";
    assert!(evaluate_bytecode(code, 1000).is_err());
}

#[test]
fn test_destructure_record_missing_field_error() {
    let code = "const { x: a, z: c } = { x: 1 }\na";
    assert!(evaluate_bytecode(code, 1000).is_err());
}

#[test]
fn test_match_literal() {
    let code = "match 2 { 1 => \"one\", 2 => \"two\", _ => \"other\" }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::String("two".into()));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_wildcard() {
    let code = "match 99 { 1 => \"one\", 2 => \"two\", _ => \"other\" }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::String("other".into()));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_variable_binding() {
    let code = "match 5 { 0 => 0, n => n * 10 }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(50.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_guard() {
    let code = "match 3 { x if (x > 5) => 0, y if (y > 1) => y * 10, _ => -1 }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(30.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_option_variants() {
    let code_some = "match Some(42) { Some(x) => x + 1, None() => 0 }";
    let expected_some = evaluate(code_some, 1000).unwrap();
    let actual_some = evaluate_bytecode(code_some, 1000).unwrap();
    assert_eq!(actual_some, Value::Number(43.0));
    assert_eq!(actual_some, expected_some);

    let code_none = "match None() { Some(x) => x, None() => -1 }";
    let expected_none = evaluate(code_none, 1000).unwrap();
    let actual_none = evaluate_bytecode(code_none, 1000).unwrap();
    assert_eq!(actual_none, Value::Number(-1.0));
    assert_eq!(actual_none, expected_none);
}

#[test]
fn test_match_result_variants() {
    let code_ok = "match Ok(100) { Ok(v) => v, Err(e) => 0 }";
    let expected_ok = evaluate(code_ok, 1000).unwrap();
    let actual_ok = evaluate_bytecode(code_ok, 1000).unwrap();
    assert_eq!(actual_ok, Value::Number(100.0));
    assert_eq!(actual_ok, expected_ok);

    let code_err = "match Err(\"failure\") { Ok(v) => \"success\", Err(e) => e }";
    let expected_err = evaluate(code_err, 1000).unwrap();
    let actual_err = evaluate_bytecode(code_err, 1000).unwrap();
    assert_eq!(actual_err, Value::String("failure".into()));
    assert_eq!(actual_err, expected_err);
}

#[test]
fn test_match_nested_variant_tuple() {
    let code = "match Some((10, 20)) { Some((x, y)) => x + y, _ => 0 }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(30.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_tuple() {
    let code = "match (15, 27) { (x, y) => x + y }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(42.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_user_struct() {
    let code = "struct Point { x: number }\nmatch Point({x: 5}) { {x: val} => val }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(5.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_match_user_enum() {
    let code = "enum State { Idle, Moving(number) }\nmatch State.Moving(7) { State.Idle() => 0, State.Moving(n) => n * 2 }";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(14.0));
    assert_eq!(actual, expected);

    let code_idle = "enum State { Idle, Moving(number) }\nmatch State.Idle() { State.Idle() => 42, State.Moving(n) => n }";
    let expected_idle = evaluate(code_idle, 1000).unwrap();
    let actual_idle = evaluate_bytecode(code_idle, 1000).unwrap();
    assert_eq!(actual_idle, Value::Number(42.0));
    assert_eq!(actual_idle, expected_idle);
}

#[test]
fn test_match_no_matching_pattern_error() {
    let code = "match 100 { 1 => 1, 2 => 2 }";
    assert!(evaluate_bytecode(code, 1000).is_err());
}

#[test]
fn test_match_in_function_body() {
    let code = "fn classify(n) { return match n { 0 => \"zero\", x if (x > 0) => \"positive\", _ => \"negative\" } }\n[classify(10), classify(-5), classify(0)]";
    let expected = evaluate(code, 2000).unwrap();
    let actual = evaluate_bytecode(code, 2000).unwrap();
    assert_eq!(
        actual,
        Value::List(vec![
            Value::String("positive".into()),
            Value::String("negative".into()),
            Value::String("zero".into()),
        ])
    );
    assert_eq!(actual, expected);
}
