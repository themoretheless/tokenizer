use themoretheless_tokenizer_rush::{Value, evaluate, evaluate_bytecode};

#[test]
fn test_named_function_basic() {
    let code = "fn add(a, b) { return a + b }\nadd(15, 27)";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(42.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_named_function_explicit_return() {
    let code = "fn check_sign(x) { if x > 0 { return \"positive\" }; if x < 0 { return \"negative\" }; return \"zero\" }\n[check_sign(5), check_sign(-3), check_sign(0)]";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
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

#[test]
fn test_recursion_factorial() {
    let code =
        "fn factorial(n) { if n <= 1 { return 1 }; return n * factorial(n - 1) }\nfactorial(6)";
    let expected = evaluate(code, 5000).unwrap();
    let actual = evaluate_bytecode(code, 5000).unwrap();
    assert_eq!(actual, Value::Number(720.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_recursion_fibonacci() {
    let code = "fn fib(n) { if n <= 1 { return n }; return fib(n - 1) + fib(n - 2) }\nfib(10)";
    let expected = evaluate(code, 10000).unwrap();
    let actual = evaluate_bytecode(code, 10000).unwrap();
    assert_eq!(actual, Value::Number(55.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_mutual_recursion() {
    let code = "fn is_even(n, odd_fn) { if n == 0 { return true }; return odd_fn(n - 1, is_even) }\nfn is_odd(n, even_fn) { if n == 0 { return false }; return even_fn(n - 1, is_odd) }\n[is_even(8, is_odd), is_even(9, is_odd), is_odd(7, is_even), is_odd(4, is_even)]";
    let expected = evaluate(code, 5000).unwrap();
    let actual = evaluate_bytecode(code, 5000).unwrap();
    assert_eq!(
        actual,
        Value::List(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(false),
        ])
    );
    assert_eq!(actual, expected);
}

#[test]
fn test_anonymous_lambda_immediate_call() {
    let code = "((a, b) => a * 10 + b)(3, 7)";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(37.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_single_param_lambda_without_parens() {
    let code = "const double = x => x * 2; double(21)";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(42.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_closure_immutable_capture() {
    let code = "fn make_adder(x) { return y => x + y }\nconst add10 = make_adder(10)\nconst add20 = make_adder(20)\n[add10(5), add20(5), add10(15)]";
    let expected = evaluate(code, 2000).unwrap();
    let actual = evaluate_bytecode(code, 2000).unwrap();
    assert_eq!(
        actual,
        Value::List(vec![
            Value::Number(15.0),
            Value::Number(25.0),
            Value::Number(25.0),
        ])
    );
    assert_eq!(actual, expected);
}

#[test]
fn test_nested_multi_level_closures() {
    let code = "const curry3 = a => b => c => a * 100 + b * 10 + c; curry3(4)(5)(6)";
    let expected = evaluate(code, 2000).unwrap();
    let actual = evaluate_bytecode(code, 2000).unwrap();
    assert_eq!(actual, Value::Number(456.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_closure_mutable_capture_escaping() {
    let code = "fn create_counter(start) { mut count = start; return () => count += 1 }\nconst c1 = create_counter(0)\nconst c2 = create_counter(100)\n[c1(), c1(), c2(), c1(), c2()]";
    let expected = evaluate(code, 2000).unwrap();
    let actual = evaluate_bytecode(code, 2000).unwrap();
    assert_eq!(
        actual,
        Value::List(vec![
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Number(101.0),
            Value::Number(3.0),
            Value::Number(102.0),
        ])
    );
    assert_eq!(actual, expected);
}

#[test]
fn test_closures_sharing_same_mutable_upvalue() {
    let code = "fn make_box(initial) { mut val = initial; const getter = () => val; const setter = x => val = x; const adder = delta => val += delta; return [getter, setter, adder] }\nconst b = make_box(10)\nconst get = b[0]\nconst set = b[1]\nconst add = b[2]\nconst v1 = get()\nadd(5)\nconst v2 = get()\nset(100)\nconst v3 = get()\n[v1, v2, v3]";
    let expected = evaluate(code, 3000).unwrap();
    let actual = evaluate_bytecode(code, 3000).unwrap();
    assert_eq!(
        actual,
        Value::List(vec![
            Value::Number(10.0),
            Value::Number(15.0),
            Value::Number(100.0),
        ])
    );
    assert_eq!(actual, expected);
}

#[test]
fn test_higher_order_builtins_with_closures() {
    let code = "const nums = [1, 2, 3, 4, 5]\nconst doubled = nums | map(x => x * 2)\nconst evens = doubled | filter(x => x > 4)\nconst sum = evens | fold(0, (acc, x) => acc + x)\n[doubled, evens, sum]";
    let expected = evaluate(code, 3000).unwrap();
    let actual = evaluate_bytecode(code, 3000).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn test_pipeline_with_user_functions() {
    let code =
        "fn square(x) { return x * x }\nfn plus_one(x) { return x + 1 }\n5 | square | plus_one";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(26.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_closure_mutating_captured_container() {
    let code = "mut list = [10, 20]\nconst modify = () => list[0] = 99\nmodify()\nlist[0]";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(99.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_infinite_recursion_hits_budget() {
    let code = "fn infinite() { return infinite() }\ninfinite()";
    assert!(evaluate_bytecode(code, 500).is_err());
}

#[test]
fn test_closures_in_records() {
    let code = "fn make_obj(x) { return { val: x, add: y => x + y } }\nconst obj = make_obj(10)\nobj.add(5)";
    let expected = evaluate(code, 1000).unwrap();
    let actual = evaluate_bytecode(code, 1000).unwrap();
    assert_eq!(actual, Value::Number(15.0));
    assert_eq!(actual, expected);
}

#[test]
fn test_cross_engine_interop() {
    use themoretheless_tokenizer_rush::{
        CancellationToken, ExecutionLimits, HostRegistration, Program, ValueType,
    };
    let bc_closure = evaluate_bytecode("x => x * 10", 1000).unwrap();
    let host = HostRegistration::new(
        "get_multiplier",
        vec![],
        ValueType::Function(vec![ValueType::Number], Box::new(ValueType::Number)),
        move |_, _| Ok(bc_closure.clone()),
    );
    let program = Program::compile("const f = get_multiplier(); [1, 2, 3] | map(f)").unwrap();
    let token = CancellationToken::default();
    let instance = program
        .instantiate(ExecutionLimits::new(5000), &token, &[], &[], &[host], &[])
        .unwrap();
    assert_eq!(
        instance.initial_value(),
        &Value::List(vec![
            Value::Number(10.0),
            Value::Number(20.0),
            Value::Number(30.0),
        ])
    );
}
