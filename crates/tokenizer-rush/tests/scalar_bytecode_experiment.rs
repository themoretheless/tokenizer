#[path = "../benches/support/scalar.rs"]
mod scalar;
use scalar::{Scalar, Tree};
use themoretheless_tokenizer_rush::{CancellationToken, Value, evaluate};

thread_local! {
    static HOST_TRACE: std::cell::RefCell<Vec<f64>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn observe<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::Number(n) = args[0] else {
        panic!("host contract bypassed")
    };
    HOST_TRACE.with_borrow_mut(|trace| trace.push(n));
    Ok(Value::Number(n * 2.0))
}
fn reject<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    Err("host rejected: проверка".into())
}
fn wrong_result<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    Ok(Value::Bool(true))
}
fn cancel_call<'s>(_: &[Value<'s>], token: &CancellationToken) -> Result<Value<'s>, String> {
    token.cancel();
    Ok(Value::Number(1.0))
}
fn trace() -> Vec<f64> {
    HOST_TRACE.with_borrow_mut(std::mem::take)
}

#[test]
fn returned_immutable_closures_match_rush_after_factory_is_dropped() {
    use scalar::closures::Factory;
    for (source, outer, captured) in [
        ("factor => x => x*factor", vec![Scalar::Number(3.0)], 1),
        (
            "(factor,offset) => x => x*factor+offset",
            vec![Scalar::Number(3.0), Scalar::Number(7.0)],
            2,
        ),
        ("x => x => x*2", vec![Scalar::Number(99.0)], 0),
        (
            "(factor,unused) => x => x*factor",
            vec![Scalar::Number(3.0), Scalar::Number(99.0)],
            1,
        ),
        (
            "flag => x => if flag { x*2 } else { x/0 }",
            vec![Scalar::Bool(true)],
            1,
        ),
    ] {
        let factory = Factory::compile(source).unwrap();
        let vm_factory = factory.bytecode();
        let tree = factory.bind(&outer).unwrap();
        let vm = vm_factory.bind(&outer).unwrap();
        assert_eq!(tree.captured_count(), captured);
        assert_eq!(vm.captured_count(), captured);
        let mut tree_stack = tree.stack();
        let mut vm_stack = vm.stack();
        drop(factory);
        drop(vm_factory);
        let outer_source = outer
            .iter()
            .map(|v| match v {
                Scalar::Number(n) => n.to_string(),
                Scalar::Bool(b) => b.to_string(),
            })
            .collect::<Vec<_>>()
            .join(",");
        for x in -10..10 {
            let program = format!(
                "const factory = {source}; const bound = factory({outer_source}); bound({x})"
            );
            let Value::Number(expected) = evaluate(&program, 1000).unwrap() else {
                panic!()
            };
            let args = [Scalar::Number(f64::from(x))];
            let token = CancellationToken::default();
            assert_eq!(
                tree.run(&args, 100, &token, &mut tree_stack).unwrap(),
                Scalar::Number(expected)
            );
            assert_eq!(
                vm.run(&args, 100, &token, &mut vm_stack).unwrap(),
                Scalar::Number(expected)
            );
            for fuel in 0..20 {
                assert_eq!(
                    tree.run(&args, fuel, &token, &mut tree_stack),
                    vm.run(&args, fuel, &token, &mut vm_stack)
                );
            }
        }
    }
}

#[test]
fn closure_instances_are_independent_and_errors_allow_reuse() {
    use scalar::closures::Factory;
    let source = String::from("factor => x => x/factor");
    let factory = Factory::compile(&source).unwrap();
    let code_factory = factory.bytecode();
    let first = code_factory.bind(&[Scalar::Number(2.0)]).unwrap();
    let second = code_factory.bind(&[Scalar::Number(3.0)]).unwrap();
    let failing = code_factory.bind(&[Scalar::Number(0.0)]).unwrap();
    assert!(factory.bind(&[]).is_err());
    drop(source);
    drop(factory);
    drop(code_factory);
    let mut stack = first.stack();
    let token = CancellationToken::default();
    assert!(
        failing
            .run(&[Scalar::Number(6.0)], 100, &token, &mut stack)
            .is_err()
    );
    assert!(stack.is_empty());
    assert!(first.run(&[], 100, &token, &mut stack).is_err());
    assert_eq!(
        first
            .run(&[Scalar::Number(6.0)], 100, &token, &mut stack)
            .unwrap(),
        Scalar::Number(3.0)
    );
    assert_eq!(
        second
            .run(&[Scalar::Number(6.0)], 100, &token, &mut stack)
            .unwrap(),
        Scalar::Number(2.0)
    );
    token.cancel();
    assert_eq!(
        first
            .run(&[Scalar::Number(6.0)], 100, &token, &mut stack)
            .unwrap_err()
            .message,
        "Execution cancelled"
    );
    assert!(stack.is_empty());
    for source in ["x=>x", "x=>y=>z=>x+y+z", "x=>x=>missing", "(x,x)=>y=>x+y"] {
        assert!(Factory::compile(source).is_err(), "{source}");
    }
}

#[test]
fn host_contract_errors_order_and_cancellation_match_working_rush() {
    use std::rc::Rc;
    use themoretheless_tokenizer_rush::{HostCallback, HostFunction, Program, ValueType};
    let hosts = [
        ("observe", observe as HostCallback),
        ("reject", reject as HostCallback),
        ("wrong", wrong_result as HostCallback),
        ("cancel", cancel_call as HostCallback),
    ]
    .map(|(name, callback)| {
        Rc::new(HostFunction {
            name,
            parameters: vec![ValueType::Number],
            result: ValueType::Number,
            callback,
        })
    });
    for source in [
        "observe(3)",
        "observe(observe(2))",
        "observe(1)+observe(2)",
        "observe(true)",
        "observe()",
        "observe(1,observe(2))",
        "observe(1e308)",
        "reject(1)",
        "wrong(1)",
        "cancel(1)+observe(2)",
        "false and observe(2)",
        "(if true { observe(4) } else { reject(1) })",
        "observe(reject(1))",
        "observe(1/0)",
    ] {
        let program = Program::compile(source).unwrap();
        let expected = program
            .run_with_host(1000, &CancellationToken::default(), &[], &hosts)
            .map(|value| match value {
                Value::Number(n) => Scalar::Number(n),
                Value::Bool(b) => Scalar::Bool(b),
                _ => panic!(),
            })
            .map_err(|err| (err.span, err.message));
        let expected_trace = trace();
        let tree = Tree::compile_with_hosts(source, &[], &hosts).unwrap();
        let actual = tree
            .run(&[], 1000, &CancellationToken::default())
            .map_err(|err| (err.span, err.message));
        assert_eq!(actual, expected, "tree: {source}");
        assert_eq!(trace(), expected_trace, "tree trace: {source}");
        let vm = tree.bytecode();
        let mut stack = vm.stack();
        let capacity = stack.capacity();
        let actual = vm
            .run(&[], 1000, &CancellationToken::default(), &mut stack)
            .map_err(|err| (err.span, err.message));
        assert_eq!(actual, expected, "VM: {source}");
        assert_eq!(trace(), expected_trace, "VM trace: {source}");
        assert!(stack.is_empty());
        assert_eq!(stack.capacity(), capacity);
        for fuel in 0..25 {
            let left = tree.run(&[], fuel, &CancellationToken::default());
            let left_trace = trace();
            let right = vm.run(&[], fuel, &CancellationToken::default(), &mut stack);
            assert_eq!(left, right, "{source}; fuel={fuel}");
            assert_eq!(left_trace, trace(), "{source}; fuel={fuel}");
            assert!(stack.is_empty());
        }
    }
    assert!(Tree::compile_with_hosts("observe(1)", &["observe"], &hosts).is_err());
    assert!(Tree::compile_with_hosts("1", &[], &[hosts[0].clone(), hosts[0].clone()]).is_err());
}

#[test]
fn scalar_tree_and_bytecode_match_rush_values() {
    let token = CancellationToken::default();
    for expression in [
        "x*2",
        "x%3 == 0",
        "sum+x",
        "x/(sum+1)",
        "x<sum",
        "x==true",
        "(x+1)*(sum-2)",
        "1000.5+x",
        "true==false",
    ] {
        let tree = Tree::compile(expression, &["x", "sum"]).unwrap();
        let bytecode = tree.bytecode();
        let mut stack = bytecode.stack();
        for x in -20..20 {
            let args = [Scalar::Number(f64::from(x)), Scalar::Number(7.0)];
            let source = format!("const x={x}; const sum=7; {expression}");
            let expected = match evaluate(&source, 1000).unwrap() {
                Value::Number(value) => Scalar::Number(value),
                Value::Bool(value) => Scalar::Bool(value),
                other => panic!("Unexpected scalar result {other:?}"),
            };
            assert_eq!(tree.run(&args, 100, &token).unwrap(), expected, "{source}");
            assert_eq!(
                bytecode.run(&args, 100, &token, &mut stack).unwrap(),
                expected,
                "{source}"
            );
            assert!(stack.is_empty());
        }
    }
}

#[test]
fn errors_have_matching_order_spans_and_reusable_stack() {
    let token = CancellationToken::default();
    for source in ["1/0", "1%0", "1e308*2", "true+1", "(1/0)+(2/0)"] {
        let tree = Tree::compile(source, &[]).unwrap();
        let bytecode = tree.bytecode();
        let mut stack = bytecode.stack();
        let expected = evaluate(source, 100).unwrap_err();
        let actual = tree.run(&[], 100, &token).unwrap_err();
        assert_eq!(actual.span, expected.span, "{source}");
        assert_eq!(actual.message, expected.message, "{source}");
        assert_eq!(
            bytecode.run(&[], 100, &token, &mut stack).unwrap_err(),
            actual
        );
        assert!(stack.is_empty());
    }
    let tree = Tree::compile("1+x/y", &["x", "y"]).unwrap();
    let bytecode = tree.bytecode();
    let mut stack = bytecode.stack();
    assert!(
        bytecode
            .run(
                &[Scalar::Number(1.0), Scalar::Number(0.0)],
                100,
                &token,
                &mut stack
            )
            .is_err()
    );
    assert!(stack.is_empty());
    assert_eq!(
        bytecode
            .run(
                &[Scalar::Number(6.0), Scalar::Number(2.0)],
                100,
                &token,
                &mut stack
            )
            .unwrap(),
        Scalar::Number(4.0)
    );
}

#[test]
fn budget_and_cancellation_match_between_experimental_engines() {
    let tree = Tree::compile("(x+2)*(x%3)", &["x"]).unwrap();
    let bytecode = tree.bytecode();
    let mut stack = bytecode.stack();
    let token = CancellationToken::default();
    let args = [Scalar::Number(4.0)];
    for fuel in 0..15 {
        assert_eq!(
            tree.run(&args, fuel, &token),
            bytecode.run(&args, fuel, &token, &mut stack)
        );
        assert!(stack.is_empty());
    }
    token.cancel();
    assert_eq!(
        tree.run(&args, 100, &token),
        bytecode.run(&args, 100, &token, &mut stack)
    );
    assert_eq!(
        tree.run(&args, 100, &token).unwrap_err().message,
        "Execution cancelled"
    );
    assert_eq!(
        tree.run(&[], 100, &token),
        bytecode.run(&[], 100, &token, &mut stack)
    );
}

#[test]
fn unsupported_semantics_are_rejected_instead_of_falling_back() {
    for source in [
        "x=>x",
        "[1,2]",
        "if true { 1 } else { 2 }",
        "sin(1)",
        "missing",
        "const x=1",
        "1;2",
        "1e999",
    ] {
        assert!(Tree::compile(source, &[]).is_err(), "{source}");
    }
    assert!(Tree::compile("x", &["x", "x"]).is_err());
}

#[test]
fn branches_match_rush_and_skip_errors_and_budget() {
    let token = CancellationToken::default();
    for source in [
        "(if true { 7 } else { 1/0 })",
        "(if false { 1/0 } else { 9 })",
        "(if 1 { 7 } else { 9 })",
        "10+(if false { 1/0 } else { (if true { 3 } else { 4 }) })",
        "false and (1/0 == 0)",
        "true or (1/0 == 0)",
        "false && (1/0 == 0)",
        "true || (1/0 == 0)",
        "true and false",
        "false or true",
        "true and 3",
        "false or 3",
        "1 and (1/0 == 0)",
        "1 or true",
        "(if true { 1/0 } else { 2/0 })",
        "(if false { 1/0 } else { 2/0 })",
        "(true and false) or (true and true)",
    ] {
        let tree = Tree::compile(source, &[]).unwrap();
        let bytecode = tree.bytecode();
        let mut stack = bytecode.stack();
        let capacity = stack.capacity();
        let result = tree.run(&[], 100, &token);
        match (evaluate(source, 1000), &result) {
            (Ok(Value::Number(expected)), Ok(Scalar::Number(actual))) => {
                assert_eq!(expected, *actual)
            }
            (Ok(Value::Bool(expected)), Ok(Scalar::Bool(actual))) => assert_eq!(expected, *actual),
            (Err(expected), Err(actual)) => {
                assert_eq!(expected.span, actual.span, "{source}");
                assert_eq!(expected.message, actual.message, "{source}");
            }
            pair => panic!("Mismatch for {source}: {pair:?}"),
        }
        for fuel in 0..30 {
            assert_eq!(
                tree.run(&[], fuel, &token),
                bytecode.run(&[], fuel, &token, &mut stack),
                "{source}, fuel={fuel}"
            );
            assert!(stack.is_empty());
            assert_eq!(stack.capacity(), capacity, "stack underestimated: {source}");
        }
    }
    for (source, fuel) in [
        ("true or (1/0==0)", 2),
        ("false and (1/0==0)", 2),
        ("(if true { 7 } else { 1/0 })", 3),
    ] {
        let tree = Tree::compile(source, &[]).unwrap();
        assert!(tree.run(&[], fuel, &token).is_ok(), "{source}");
        assert!(tree.run(&[], fuel - 1, &token).is_err(), "{source}");
    }
}
