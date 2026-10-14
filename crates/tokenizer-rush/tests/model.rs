use themoretheless_tokenizer_rush::{CancellationToken, ExecutionLimits, ModelProgram, Value};
const LIMITS: ExecutionLimits = ExecutionLimits::new(10_000);

#[test]
fn forward_dependencies_and_function_captures_recompute_selectively() {
    let source = "node answer = scale(3); node independent = 99; fn scale(x: number)->number { return x * width }; param width: number = 2; show answer";
    let program = ModelProgram::compile(source).unwrap();
    let token = CancellationToken::default();
    let mut model = program.instantiate(LIMITS, &token, &[]).unwrap();
    assert_eq!(model.output(), &Value::Number(6.0));
    assert_eq!(
        model
            .set_parameter("width", Value::Number(4.0), LIMITS)
            .unwrap(),
        vec!["scale", "answer"]
    );
    assert_eq!(model.output(), &Value::Number(12.0));
    assert_eq!(model.get("independent"), Some(Value::Number(99.0)));
    assert!(
        model
            .set_parameter("width", Value::Number(4.0), LIMITS)
            .unwrap()
            .is_empty()
    );
    assert!(
        model
            .set_parameter("width", Value::String("bad".into()), LIMITS)
            .is_err()
    );
    assert_eq!(model.output(), &Value::Number(12.0));
}
#[test]
fn cycles_and_dead_branch_type_errors_fail_at_compile_time() {
    assert!(
        ModelProgram::compile("node a = b; node b = a; show a")
            .err()
            .unwrap()
            .message
            .contains("Cyclic")
    );
    assert!(
        ModelProgram::compile(
            "param n = 2; fn bad()->number { if false {return 'bad'}; return n }; show bad()"
        )
        .is_err()
    );
    assert!(ModelProgram::compile("param a = 2; param b = a; show b").is_err());
}
#[test]
fn failed_recompute_restores_parameter_derived_values_and_output() {
    let program =
        ModelProgram::compile("param n = 2; node x = n * 2; fn f()->number { return x }; show f()")
            .unwrap();
    let token = CancellationToken::default();
    let mut model = program.instantiate(LIMITS, &token, &[]).unwrap();
    assert!(
        model
            .set_parameter("n", Value::Number(5.0), ExecutionLimits::new(1))
            .is_err()
    );
    assert_eq!(model.get("n"), Some(Value::Number(2.0)));
    assert_eq!(model.get("x"), Some(Value::Number(4.0)));
    assert_eq!(model.output(), &Value::Number(4.0));
    model
        .set_parameter("n", Value::Number(3.0), LIMITS)
        .unwrap();
    assert_eq!(model.output(), &Value::Number(6.0));
}
#[test]
fn unknown_names_and_invalid_outputs_are_rejected() {
    assert!(ModelProgram::compile("node a = absent(1); show a").is_err());
    assert!(ModelProgram::compile("param a = 1").is_err());
    assert!(ModelProgram::compile("show 1; show 2").is_err());
}
#[test]
fn typed_records_and_comments_use_the_regular_formatter() {
    let source = "struct Settings { size: number }; param size: number = 3; node settings: Settings = Settings({size: size}); // derived output\nshow settings.size";
    let program = ModelProgram::compile(source).unwrap();
    let token = CancellationToken::default();
    let mut model = program.instantiate(LIMITS, &token, &[]).unwrap();
    model
        .set_parameter("size", Value::Number(7.0), LIMITS)
        .unwrap();
    assert_eq!(model.output(), &Value::Number(7.0));
}
#[test]
fn rollback_restores_mutable_captures() {
    let source = "param n = 1; fn make()->Fn[tuple[], number] { mut count = 0; fn next()->number { count += 1; return count }; return next }; node counter = make(); fn calculate()->number { let x = counter(); assert(n > 0); return x * n }; show calculate()";
    let program = ModelProgram::compile(source).unwrap();
    let token = CancellationToken::default();
    let mut model = program.instantiate(LIMITS, &token, &[]).unwrap();
    assert_eq!(model.output(), &Value::Number(1.0));
    let error = model
        .set_parameter("n", Value::Number(-1.0), LIMITS)
        .unwrap_err();
    assert!(error.location.is_some());
    assert!(!error.stack.is_empty());
    model
        .set_parameter("n", Value::Number(2.0), LIMITS)
        .unwrap();
    assert_eq!(model.output(), &Value::Number(4.0));
}
#[test]
fn formatter_preserves_model_roles_and_comments() {
    let source = "param n:number=2; // input\nnode x=n*3; show x";
    let formatted = themoretheless_tokenizer_rush::format_source(source).unwrap();
    assert!(formatted.contains("param n"));
    assert!(formatted.contains("node x"));
    assert!(formatted.contains("// input"));
    assert!(ModelProgram::compile(&formatted).is_ok());
}
#[test]
fn imported_nominal_contracts_are_checked_before_model_execution() {
    use themoretheless_tokenizer_rush::Program;
    let module = Program::compile("struct Settings { size: number }; export Settings").unwrap();
    let program = ModelProgram::compile("import model; param n = 2; node settings: model.Settings = model.Settings({size: n}); show settings.size").unwrap();
    let token = CancellationToken::default();
    let mut model = program
        .instantiate(LIMITS, &token, &[("model", &module)])
        .unwrap();
    model
        .set_parameter("n", Value::Number(5.0), LIMITS)
        .unwrap();
    assert_eq!(model.output(), &Value::Number(5.0));
}
#[test]
fn imported_failures_keep_the_module_position_and_call_frame() {
    use themoretheless_tokenizer_rush::Program;
    let module = Program::compile(
        "fn checked(x: number)->number {\nassert(x > 0); return x\n}; export checked",
    )
    .unwrap();
    let program = ModelProgram::compile(
        "import model; param n = 2; node result = model.checked(n); show result",
    )
    .unwrap();
    let token = CancellationToken::default();
    let mut model = program
        .instantiate(LIMITS, &token, &[("model", &module)])
        .unwrap();
    let error = model
        .set_parameter("n", Value::Number(-1.0), LIMITS)
        .unwrap_err();
    assert_eq!(error.module.as_deref(), Some("model"));
    assert_eq!(error.location.unwrap().line, 2);
    assert!(!error.stack.is_empty());
    assert_eq!(model.output(), &Value::Number(2.0));
}
