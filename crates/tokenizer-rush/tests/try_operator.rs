use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, OwnedScriptInstance, Program, StateValue, Value, evaluate,
    format_source,
};

#[test]
fn option_and_result_successes_unwrap_and_failures_return_early() {
    let source = "fn f(v: Option[number]) -> Option[number] { let x = v?; return Some(x+1) }; (f(Some(2)), f(None()))";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Some", vec![Value::Number(3.)]),
            Value::Variant("None", vec![])
        ])
    );
    let source = "fn f(v: Result[number,str]) -> Result[number,str] { return Ok(v? + 1) }; (f(Ok(2)), f(Err('missing')))";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Ok", vec![Value::Number(3.)]),
            Value::Variant("Err", vec![Value::String("missing".into())])
        ])
    );
}

#[test]
fn propagation_stops_loops_and_skips_later_effects_without_rollback() {
    let source = "mut calls = 0; fn f(v: Option[number]) -> Option[number] { for x in range_iter(0,10) { calls += 1; let y = v?; calls += y }; calls += 100; return Some(0) }; let value = f(None()); (value,calls)";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![Value::Variant("None", vec![]), Value::Number(1.)])
    );
    let source = "mut calls=0; fn side() { calls+=1; return 1 }; fn f(v: Result[number,str]) -> Result[number,str] { return Ok(v? + side()) }; let value=f(Err('missing')); (value,calls)";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Err", vec![Value::String("missing".into())]),
            Value::Number(0.)
        ])
    );
}

#[test]
fn only_the_nearest_function_returns_and_calls_can_be_reused() {
    let source = "fn inner(v: Option[number]) -> Option[number] { return Some(v?+1) }; fn outer() -> Option[number] { let ignored=inner(None()); return inner(Some(4)) }; (outer(),inner(None()),outer())";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Some", vec![Value::Number(5.)]),
            Value::Variant("None", vec![]),
            Value::Variant("Some", vec![Value::Number(5.)])
        ])
    );
}

#[test]
fn loading_rejects_incompatible_propagation_even_when_unused() {
    for source in [
        "Some(1)?",
        "fn f(v: Option[number]) -> number { return v? }",
        "fn f(v: number) -> Option[number] { return Some(v?) }",
        "fn f(v: Option[number]) -> Result[number,str] { return Ok(v?) }",
        "fn f(v: Result[number,bool]) -> Result[number,str] { return Ok(v?) }",
        "fn f(v: Option[number]) -> Option[number] { let g=() => v?; return Some(1) }",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
}

#[test]
fn postfix_chaining_works_with_user_data_and_formats_stably() {
    let source = "struct Point { x: number }; fn f(v: Option[Point]) -> Option[number] { return Some(v?.x) }; f(Some(Point({x:7})))";
    assert_eq!(
        evaluate(source, 1000).unwrap(),
        Value::Variant("Some", vec![Value::Number(7.)])
    );
    let formatted = format_source(source).unwrap();
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    assert_eq!(
        evaluate(&formatted, 1000).unwrap(),
        Value::Variant("Some", vec![Value::Number(7.)])
    );
}

#[test]
fn try_block_catches_propagation_and_rolls_back_mutations_on_failure() {
    let source = "\
mut counter = 10
mut items = [1, 2]
mut state = { hp: 100 }
fn step(v: Result[number, str]) -> Result[number, str] {
    counter += 5
    items = [1, 2, 3]
    state.hp -= 20
    let x = v?
    counter += x
    return Ok(x * 2)
}
let failed = try {
    let a = step(Ok(4))?
    let b = step(Err('boom'))?
    Ok(a + b)
}
let after_fail = (failed, counter, items, state.hp)
let succeeded = try {
    let a = step(Ok(4))?
    let b = step(Ok(6))?
    Ok(a + b)
}
(after_fail, succeeded, counter, items, state.hp)
";
    assert_eq!(
        evaluate(source, 5000).unwrap(),
        Value::Tuple(vec![
            Value::Tuple(vec![
                Value::Variant("Err", vec![Value::String("boom".into())]),
                Value::Number(10.),
                Value::List(vec![Value::Number(1.), Value::Number(2.)]),
                Value::Number(100.),
            ]),
            Value::Variant("Ok", vec![Value::Number(20.)]),
            Value::Number(30.),
            Value::List(vec![
                Value::Number(1.),
                Value::Number(2.),
                Value::Number(3.)
            ]),
            Value::Number(60.),
        ])
    );
}

#[test]
fn nested_try_blocks_isolate_inner_rollback_while_preserving_outer_progress() {
    let source = "\
mut log = 0
let outcome = try {
    log += 10
    let inner = try {
        log += 100
        let x = None()?
        Some(x)
    }
    let fallback = match inner {
        Some(v) => v,
        None() => 5,
    }
    log += fallback
    Some(log)
}
(outcome, log)
";
    assert_eq!(
        evaluate(source, 5000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("Some", vec![Value::Number(15.)]),
            Value::Number(15.),
        ])
    );
}

#[test]
fn try_block_formats_stably_and_rejects_invalid_control_or_types() {
    let source = "let x = try { let v = Some(1)?; Some(v + 2) }; x";
    let formatted = format_source(source).unwrap();
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    assert_eq!(
        evaluate(&formatted, 1000).unwrap(),
        Value::Variant("Some", vec![Value::Number(3.)])
    );

    for invalid in [
        "try { 42 }",
        "try { let x = Some(1)?; Ok(x) }",
        "try { let x = Ok(1)?; Some(x) }",
        "try { let x = 42?; Some(x) }",
        "fn f() -> Option[number] { let x = try { return Some(1) }; return x }",
        "for i in [1, 2] { let x = try { break; Some(1) } }",
    ] {
        assert!(Program::compile(invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn call_transactional_rolls_back_instance_mutations_on_err_none_or_runtime_error() {
    let source = "\
mut balance: int = 100
fn withdraw(amount: int) -> Result[int, str] {
    balance -= amount
    if balance < 0 {
        return Err('insufficient funds')
    }
    return Ok(balance)
}
fn explode(divisor: int) -> int {
    balance -= 50
    return 100 / divisor
}
";
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(5000);
    let mut instance = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();

    assert_eq!(
        instance
            .call_transactional("withdraw", &[Value::Int(30)], limits)
            .unwrap(),
        Value::Variant("Ok", vec![Value::Int(70)])
    );
    assert_eq!(instance.get("balance"), Some(Value::Int(70)));

    assert_eq!(
        instance
            .call_transactional("withdraw", &[Value::Int(90)], limits)
            .unwrap(),
        Value::Variant("Err", vec![Value::String("insufficient funds".into())])
    );
    assert_eq!(instance.get("balance"), Some(Value::Int(70)));

    assert!(
        instance
            .call_transactional("explode", &[Value::Int(0)], limits)
            .is_err()
    );
    assert_eq!(instance.get("balance"), Some(Value::Int(70)));

    let mut owned = OwnedScriptInstance::new(source, limits).unwrap();
    assert!(
        owned
            .call_transactional("explode", &[StateValue::Int(0)], limits)
            .is_err()
    );
    assert_eq!(
        owned
            .call_values_transactional("withdraw", &[Value::Int(150)], limits)
            .unwrap(),
        Value::Variant("Err", vec![Value::String("insufficient funds".into())])
    );
    assert_eq!(
        owned.export_state(&["balance"]).unwrap().get("balance"),
        Some(&StateValue::Int(100))
    );
}
