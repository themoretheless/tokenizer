use themoretheless_tokenizer_rush::{Program, Value, evaluate, format_source};

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
