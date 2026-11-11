use std::{cell::RefCell, rc::Rc};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostFunction, HostRegistration, OwnedScriptInstance,
    Program, StateValue, Value, ValueType, analyze_host_calls, evaluate, evaluate_bytecode,
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

#[test]
fn undo_log_preserves_untouched_cells_restores_move_and_rolls_back_committed_inner_try() {
    let source = "\
mut untouched = [10, 20, 30]
mut box_val = [99]
mut stage = 1

let res = try {
    let taken = move(box_val)
    let inner = try {
        stage = 2
        Some(taken[0] + 1)
    }
    let v = inner?
    stage = v
    let abort: Option[int] = None()
    Some(abort?)
}
box_val = box_val
(res, untouched, box_val, stage)
";
    assert_eq!(
        evaluate(source, 5000).unwrap(),
        Value::Tuple(vec![
            Value::Variant("None", vec![]),
            Value::List(vec![
                Value::Number(10.),
                Value::Number(20.),
                Value::Number(30.)
            ]),
            Value::List(vec![Value::Number(99.)]),
            Value::Number(1.),
        ])
    );
}

#[test]
fn aborted_try_block_rolls_back_spawned_async_tasks() {
    let source = "\
async fn worker() -> int { return 42 }
mut leaked = worker()
let before = leaked
let outcome = try {
    leaked = worker()
    let fail: Option[int] = None()
    Some(fail?)
}
let good = worker()
(before, outcome, leaked, good)
";
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(5000);
    let mut instance = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let Value::Tuple(items) = instance.initial_value().clone() else {
        panic!("expected tuple");
    };
    let before_id = items[0].task_id().expect("expected before task id");
    assert_eq!(items[1], Value::Variant("None", vec![]));
    assert_eq!(items[2].task_id(), Some(before_id));
    let good_id = items[3].task_id().expect("expected good task id");
    // `leaked` got an ID between `before_id` and `good_id` and was rolled back on abort.
    assert!(good_id.get() == before_id.get() + 2);
    assert!(instance.resume_coroutine(before_id, limits).is_ok());
    assert!(instance.resume_coroutine(good_id, limits).is_ok());
}

#[test]
fn try_block_enforces_boundary_diagnostics_type_inference_formatter_and_bytecode_rejection() {
    for invalid in [
        "async fn f() -> Option[int] { let x = try { let v = await 1; Some(v) }; return x }",
        "let x = try { import math; Some(1) }",
        "let y = 1; let x = try { export y; Some(y) }",
    ] {
        assert!(Program::compile(invalid).is_err(), "accepted {invalid}");
    }

    // Parameter type inference propagates constraints through `try { ... }` and `?`.
    let infer_src = "fn bump(x) -> Option[int] { return try { Some(x + 1) } }; bump(41)";
    assert_eq!(
        evaluate(infer_src, 1000).unwrap(),
        Value::Variant("Some", vec![Value::Int(42)])
    );
    assert!(
        Program::compile("fn bump(x) -> Option[int] { return try { Some(x + 1) } }; bump(true)")
            .is_err()
    );

    // Multiline comments inside `try { ... }` format cleanly without spurious outer parens.
    let commented =
        "let r = try {\n    // step one\n    let a = Some(10)?\n    Some(a + 5)\n}\nr\n";
    let formatted = format_source(commented).unwrap();
    assert!(!formatted.contains("(try {"), "{formatted}");
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    assert_eq!(
        evaluate(&formatted, 1000).unwrap(),
        Value::Variant("Some", vec![Value::Number(15.)])
    );

    // Bytecode VM explicitly rejects `try { ... }` instead of executing non-transactionally.
    let err = evaluate_bytecode("try { Some(1) }", 1000).unwrap_err();
    assert!(
        err.message
            .contains("try blocks are not supported in bytecode compilation"),
        "{}",
        err.message
    );
}

#[test]
fn host_transactional_effect_contract_blocks_impure_callbacks_and_allows_transactional_ones() {
    let side_effects = Rc::new(RefCell::new(0usize));
    let effects_ref = side_effects.clone();
    let impure = HostRegistration::new("impure_write", vec![], ValueType::Int, move |_, _| {
        *effects_ref.borrow_mut() += 1;
        Ok(Value::Int(1))
    });
    assert!(!impure.is_transactional());

    let pure_reads = Rc::new(RefCell::new(0usize));
    let reads_ref = pure_reads.clone();
    let pure_ctx = HostRegistration::new("pure_read", vec![], ValueType::Int, move |_, _| {
        *reads_ref.borrow_mut() += 1;
        Ok(Value::Int(40))
    })
    .with_transactional(true);
    assert!(pure_ctx.is_transactional());

    fn plain_double_cb<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        let Value::Int(n) = args[0] else {
            unreachable!()
        };
        Ok(Value::Int(n * 2))
    }
    let plain_host = Rc::new(HostFunction {
        name: "plain_double",
        parameters: vec![ValueType::Int],
        result: ValueType::Int,
        callback: plain_double_cb,
    });

    // Static rejection on direct impure host call inside `try { ... }` at instantiate time.
    let direct_prog = Program::compile("try { let x = impure_write(); Some(x) }").unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(5000);
    let static_err =
        match direct_prog.instantiate(limits, &token, &[], &[], std::slice::from_ref(&impure), &[])
        {
            Ok(_) => panic!("expected static non-transactional-host error"),
            Err(err) => err,
        };
    assert!(
        static_err.message.contains("non-transactional-host"),
        "{}",
        static_err.message
    );
    assert_eq!(*side_effects.borrow(), 0);

    // Transactional contextual host + plain HostFunction succeed inside `try { ... }`.
    let ok_analysis = analyze_host_calls(
        "try { Some(pure_read() + plain_double(1)) }",
        &[pure_ctx.function.clone(), plain_host.clone()],
    );
    assert!(ok_analysis.is_valid(), "{:?}", ok_analysis.diagnostics);

    // Runtime rejection when impure host is called indirectly (via closure/alias) inside
    // `try { ... }` or inside `call_transactional`, rolling back mutations with 0 side effects.
    let source = "\
mut counter: int = 10
fn call_indirect(cb) -> Option[int] {
    return try {
        counter += 5
        Some(cb())
    }
}
fn tx_entry() -> Result[int, str] {
    counter += 7
    return Ok(impure_write())
}
fn tx_pure() -> Result[int, str] {
    counter += 2
    return Ok(pure_read() + plain_double(1))
}
";
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(5000);
    let mut instance = program
        .instantiate(limits, &token, &[], &[plain_host], &[impure, pure_ctx], &[])
        .unwrap();

    let impure_val = instance.get("impure_write").unwrap();
    let err = instance
        .call("call_indirect", &[impure_val], limits)
        .unwrap_err();
    assert!(
        err.message
            .contains("cannot be called inside a transaction"),
        "{}",
        err.message
    );
    assert_eq!(*side_effects.borrow(), 0);
    assert_eq!(instance.get("counter"), Some(Value::Int(10)));

    let err = instance
        .call_transactional("tx_entry", &[], limits)
        .unwrap_err();
    assert!(
        err.message
            .contains("cannot be called inside a transaction"),
        "{}",
        err.message
    );
    assert_eq!(*side_effects.borrow(), 0);
    assert_eq!(instance.get("counter"), Some(Value::Int(10)));

    assert_eq!(
        instance.call_transactional("tx_pure", &[], limits).unwrap(),
        Value::Variant("Ok", vec![Value::Int(42)])
    );
    assert_eq!(*pure_reads.borrow(), 1);
    assert_eq!(instance.get("counter"), Some(Value::Int(12)));
}
