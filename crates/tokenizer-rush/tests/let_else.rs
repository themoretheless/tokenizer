use std::time::Duration;
use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineScheduler, ExecutionLimits, Program, ScheduledState, Value,
    analyze_host_calls, evaluate, evaluate_bytecode, format_source,
};

#[test]
fn let_else_unwraps_or_diverges_across_return_break_continue_and_try() {
    let source = "\
enum Status { Ready(number), Pending }

fn parse_pair(v: Result[tuple[number, number], str]) -> number {
    let Ok((a, b)) = v else {
        return -1
    }
    return a + b
}

fn sum_ready(items) -> number {
    mut acc = 0
    for item in items {
        let Status.Ready(n) = item else {
            continue
        }
        if n < 0 {
            let Some(pos) = None() else {
                break
            }
            acc += pos
        }
        acc += n
    }
    return acc
}

fn make_none() -> Option[number] {
    return None()
}

let in_try = try {
    let Some(x) = Some(10) else {
        None()?
    }
    let Some(y) = make_none() else {
        None()?
    }
    Some(x + y)
}

(
    parse_pair(Ok((20, 22))),
    parse_pair(Err('bad')),
    sum_ready([Status.Pending, Status.Ready(4), Status.Ready(6), Status.Ready(-1), Status.Ready(100)]),
    in_try,
)
";
    assert_eq!(
        evaluate(source, 5000).unwrap(),
        Value::Tuple(vec![
            Value::Number(42.),
            Value::Number(-1.),
            Value::Number(10.),
            Value::Variant("None", vec![]),
        ])
    );
}

#[test]
fn let_else_isolates_partial_matches_and_preserves_outer_shadowed_bindings_in_else() {
    let source = "\
fn check(pair) -> number {
    let x = 99
    if true {
        let (x, Some(y)) = pair else {
            return x
        }
        return x + y
    }
    return 0
}
(check((1, None())), check((10, Some(5))))
";
    let expected = Value::Tuple(vec![Value::Number(99.), Value::Number(15.)]);
    assert_eq!(evaluate(source, 2000).unwrap(), expected);
    assert_eq!(evaluate_bytecode(source, 2000).unwrap(), expected);
}

#[test]
fn let_else_rejects_non_diverging_else_and_bare_refutable_let() {
    let non_diverging =
        "fn f(v: Option[number]) -> number { let Some(x) = v else { 0 }; return x }";
    let parsed = analyze_host_calls(non_diverging, &[]);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "let-else-must-diverge"),
        "{:?}",
        parsed.diagnostics
    );
    assert!(Program::compile(non_diverging).is_err());
    let bc_err = evaluate_bytecode(non_diverging, 1000).unwrap_err();
    assert!(
        bc_err.message.contains("must diverge"),
        "{}",
        bc_err.message
    );

    let bare_refutable = "let Some(x) = Some(1)";
    let parsed_bare = analyze_host_calls(bare_refutable, &[]);
    assert!(
        parsed_bare
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid-binding-pattern"),
        "{:?}",
        parsed_bare.diagnostics
    );
    assert!(Program::compile(bare_refutable).is_err());
}

#[test]
fn if_let_statements_expressions_chains_and_static_type_narrowing() {
    let source = "\
fn classify(a: Option[number], b: Result[number, str]) -> number {
    mut bonus = 0
    if let Some(x) = a {
        bonus += x * 2
    } else if let Ok(y) = b {
        bonus += y + 1
    } else {
        bonus = -5
    }
    let expr_val = if let Ok(v) = b { v * 10 } else { 3 }
    return bonus + expr_val
}
(
    classify(Some(4), Err('no')),
    classify(None(), Ok(6)),
    classify(None(), Err('no')),
)
";
    let expected = Value::Tuple(vec![
        Value::Number(11.),
        Value::Number(67.),
        Value::Number(-2.),
    ]);
    assert_eq!(evaluate(source, 5000).unwrap(), expected);
    assert_eq!(evaluate_bytecode(source, 5000).unwrap(), expected);

    // Payload type narrowing for Option[vec3] inside `if let` and `let ... else`.
    let valid_narrow = "\
fn z_or_zero(p: Option[vec3]) -> number {
    if let Some(v) = p {
        return v.z
    }
    return 0
}
z_or_zero(Some(vec3(1, 2, 7)))
";
    assert_eq!(evaluate(valid_narrow, 1000).unwrap(), Value::Number(7.));

    let invalid_narrow =
        "fn bad(p: Option[vec3]) -> number { if let Some(v) = p { return v.w }; return 0 }";
    assert!(Program::compile(invalid_narrow).is_err());

    // Parameter type inference through `if let` and `let ... else`.
    let infer =
        "fn inc(opt) -> number { let Some(n) = opt else { return 0 }; return n + 1 }; inc(Some(8))";
    assert_eq!(evaluate(infer, 1000).unwrap(), Value::Number(9.));
}

#[test]
fn let_else_and_if_let_format_idempotently_and_run_in_async_coroutines() {
    let source = "\
fn demo(v: Option[number]) -> number {
    let Some(x) = v else {
        return 0
    }
    let doubled = if let Some(y) = Some(x + 1) { y * 2 } else { 0 }
    if let Some(z) = Some(doubled) {
        return z
    }
    return -1
}
demo(Some(4))
";
    let formatted = format_source(source).unwrap();
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    assert_eq!(evaluate(&formatted, 2000).unwrap(), Value::Number(10.));
    assert_eq!(
        evaluate_bytecode(&formatted, 2000).unwrap(),
        Value::Number(10.)
    );

    // Async coroutine support for `let Pattern = await ... else { ... }` and `if let`.
    let async_source = "\
enum Wait { After(number), Event(str) }
async fn fetch(ok: bool) -> Option[number] {
    await Wait.After(1)
    if ok {
        return Some(21)
    }
    return None()
}
async fn pipeline(ok: bool) -> number {
    let Some(base) = await fetch(ok) else {
        return -1
    }
    if let Some(extra) = Some(base) {
        await Wait.After(1)
        return base + extra
    }
    return 0
}
";
    let program = Program::compile(async_source).unwrap();
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(5000);
    let mut instance = program
        .instantiate(limits, &token, &[], &[], &[], &[])
        .unwrap();
    let mut scheduler = CoroutineScheduler::new(&mut instance);
    let t_ok = scheduler
        .spawn("pipeline", &[Value::Bool(true)], limits)
        .unwrap();
    let t_err = scheduler
        .spawn("pipeline", &[Value::Bool(false)], limits)
        .unwrap();

    let mut results = std::collections::HashMap::new();
    for tick in 0..=5 {
        for step in scheduler.poll(Duration::from_secs(tick), limits).unwrap() {
            if let ScheduledState::Complete(val) = step.state {
                results.insert(step.id, val);
            }
        }
        if scheduler.is_empty() {
            break;
        }
    }
    assert_eq!(results.get(&t_ok), Some(&Value::Number(42.)));
    assert_eq!(results.get(&t_err), Some(&Value::Number(-1.)));
}
