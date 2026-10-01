use std::rc::Rc;
use themoretheless_tokenizer_rush::{CancellationToken, HostFunction, Program, Value, ValueType};
fn double<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::Number(value) = args[0] else {
        panic!("contract must be checked first")
    };
    Ok(Value::Number(value * 2.0))
}
fn host() -> Rc<HostFunction> {
    Rc::new(HostFunction {
        name: "double",
        parameters: vec![ValueType::Number],
        result: ValueType::Number,
        callback: double,
    })
}
#[test]
fn registered_functions_work_in_pipelines_and_callbacks() {
    let program = Program::compile("[1, 2] | map(double)").unwrap();
    assert_eq!(
        program
            .run_with_host(100, &CancellationToken::default(), &[], &[host()])
            .unwrap(),
        Value::List(vec![Value::Number(2.0), Value::Number(4.0)])
    );
}
#[test]
fn host_contracts_are_enforced_before_and_after_calls() {
    for source in ["double(true)", "double()", "double(1e308)"] {
        let program = Program::compile(source).unwrap();
        assert!(
            program
                .run_with_host(100, &CancellationToken::default(), &[], &[host()])
                .is_err()
        );
    }
    let program = Program::compile("double(1)").unwrap();
    assert!(
        program
            .run_with_host(100, &CancellationToken::default(), &[], &[host(), host()])
            .is_err()
    );
}

#[test]
fn analyzer_reuses_host_arity_with_pipeline_aliases_and_shadowing() {
    use themoretheless_tokenizer_rush::analyze_host_calls;
    for source in [
        "double(1)",
        "1 | double",
        "const f = double\nf(2)",
        "const double = (a,b) => a+b\ndouble(1,2)",
    ] {
        assert!(analyze_host_calls(source, &[host()]).is_valid(), "{source}");
    }
    for source in [
        "double()",
        "1 | double(2)",
        "const f = double\nf(1,2)",
        "fn later() { double(1,2) }",
    ] {
        assert!(
            !analyze_host_calls(source, &[host()]).is_valid(),
            "{source}"
        );
    }
    // Obvious argument types are checked from the same registration.
    assert!(!analyze_host_calls("double(true)", &[host()]).is_valid());
}
