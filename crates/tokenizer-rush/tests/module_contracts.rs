use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, Program, Value, analyze_editor_modules, format_source,
};
const LIMITS: ExecutionLimits = ExecutionLimits::new(10_000);
#[test]
fn imported_signatures_reject_dead_calls_before_initialization() {
    let library =
        Program::compile("fn twice(n:number)->number {return n*2}; export twice").unwrap();
    for source in [
        "import model; fn unused(){return model.twice(true)}",
        "import model; let f=model.twice; if false { f('bad') }",
        "import model; let n:bool=model.twice(2)",
        "import model; model.twice()",
    ] {
        let main = Program::compile(source).unwrap();
        assert!(
            main.validate_modules(&[("model", &library)]).is_err(),
            "{source}"
        );
    }
}
#[test]
fn qualified_types_work_for_functions_and_nested_contracts() {
    let model=Program::compile("struct Settings {speed:number}; enum State {Idle, Moving(Settings)}; fn make()->Settings {return Settings({speed:3})}; export Settings,State,make").unwrap();
    let source = "import model; fn read(s:model.Settings)->number {return s.speed}; let items:list[model.Settings]=[model.make()]; read(items[0])";
    let main = Program::compile(source).unwrap();
    assert_eq!(
        main.run_with_modules(
            10_000,
            &CancellationToken::default(),
            &[],
            &[],
            &[("model", &model)]
        )
        .unwrap(),
        Value::Number(3.)
    );
    let formatted = format_source(source).unwrap();
    assert!(Program::compile(&formatted).is_ok());
    assert!(
        analyze_editor_modules(source, &[("model", &model)])
            .parsed
            .is_valid()
    );
    for source in [
        "import model; fn f(s:model.Settings)->number {return s.missing}",
        "import model; let s:model.Settings=model.Settings({speed:true})",
    ] {
        let main = Program::compile(source).unwrap();
        assert!(main.validate_modules(&[("model", &model)]).is_err());
    }
}
#[test]
fn transitive_interfaces_and_nominal_identity_survive_reexports() {
    let model = Program::compile(
        "struct Point{x:number}; fn make()->Point{return Point({x:4})}; export Point,make",
    )
    .unwrap();
    let facade = Program::compile(
        "import model; let Point=model.Point; let make=model.make; export Point,make",
    )
    .unwrap();
    let main =
        Program::compile("import api; fn read(p:api.Point)->number{return p.x}; read(api.make())")
            .unwrap();
    assert_eq!(
        main.run_with_modules(
            10_000,
            &CancellationToken::default(),
            &[],
            &[],
            &[("api", &facade), ("model", &model)]
        )
        .unwrap(),
        Value::Number(4.)
    );
}
#[test]
fn private_types_are_not_qualified_public_names() {
    let model = Program::compile("struct Secret{x:number}; let public=1; export public").unwrap();
    let main = Program::compile("import model; fn f(s:model.Secret)->number{return s.x}").unwrap();
    assert!(
        main.validate_modules(&[("model", &model)])
            .unwrap_err()
            .message
            .contains("unsupported-annotation")
    );
}
#[test]
fn strict_contracts_infer_parameters_recursion_and_require_dynamic_annotations() {
    for source in [
        "fn next(n){return n+1}; next(2)",
        "fn fact(n){if n<=1{return 1}; return n*fact(n-1)}; fact(5)",
        "fn use(flag)->number {if flag {return 1} else {return 2}}; use(true)",
    ] {
        assert!(Program::compile_strict(source).is_ok(), "{source}");
    }
    for source in [
        "fn id(x){return x}",
        "fn bad(n){if n<=1{return 1}; return bad(true)}",
        "fn bad(n:number)->number {if n>0{return n}}",
        "fn mixed(flag:bool){if flag{return 1};return 'x'}",
        "fn dynamic(){return input}",
    ] {
        assert!(Program::compile_strict(source).is_err(), "{source}");
    }
    assert!(Program::compile_strict("fn id(x:number)->number{return x}").is_ok());
}
#[test]
fn dynamic_public_function_requires_explicit_contract() {
    let model = Program::compile("fn identity(x){return x}; export identity").unwrap();
    let main = Program::compile("import model; 1").unwrap();
    assert!(
        main.validate_modules(&[("model", &model)])
            .unwrap_err()
            .message
            .contains("dynamic-contract")
    );
    let model = Program::compile("fn next(x){return x+1}; export next").unwrap();
    let main = Program::compile("import model; model.next(true)").unwrap();
    assert!(
        main.validate_modules(&[("model", &model)])
            .err()
            .unwrap()
            .message
            .contains("argument-type")
    );
    let token = CancellationToken::default();
    assert!(
        main.instantiate(LIMITS, &token, &[], &[], &[], &[("model", &model)])
            .is_err()
    );
}
#[test]
fn inferred_runtime_contracts_reject_host_arguments_before_effects() {
    let program =
        Program::compile_strict("mut effects=0; fn next(n){effects+=1;return n+1}").unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    assert!(
        instance
            .call("next", &[Value::Bool(true)], LIMITS)
            .unwrap_err()
            .message
            .contains("Argument")
    );
    assert_eq!(instance.get("effects"), Some(Value::Number(0.)));
    let model = Program::compile(
        "struct Point{x:number}; fn make(){return Point({x:1})}; export Point,make",
    )
    .unwrap();
    let main = Program::compile("import model; model.make().x").unwrap();
    assert_eq!(
        main.run_with_modules(10_000, &token, &[], &[], &[("model", &model)])
            .unwrap(),
        Value::Number(1.)
    );
}
#[test]
fn parameter_inference_respects_local_shadowing() {
    let program =
        Program::compile("fn f(x){if true {let x=true; if x{return 1}};return 2}; f(7)").unwrap();
    assert_eq!(
        program
            .run(1000, &CancellationToken::default(), &[])
            .unwrap(),
        Value::Number(1.)
    );
    assert!(
        Program::compile_strict("fn f(x){if true {let x=true; if x{return 1}};return 2}").is_err()
    );
}
#[test]
fn imported_enum_patterns_are_deferred_until_interfaces_are_registered() {
    let model = Program::compile("enum State{Idle,Moving(number)}; export State").unwrap();
    let main=Program::compile("import model; fn read(s:model.State)->number{return match s {model.State.Idle()=>0,model.State.Moving(n)=>n}}; read(model.State.Moving(3))").unwrap();
    assert_eq!(
        main.run_with_modules(
            10_000,
            &CancellationToken::default(),
            &[],
            &[],
            &[("model", &model)]
        )
        .unwrap(),
        Value::Number(3.)
    );
}
#[test]
fn strict_registered_contracts_infer_from_imported_calls() {
    let model = Program::compile("fn twice(n:number)->number{return n*2}; export twice").unwrap();
    let program = Program::compile(
        "import model; mut effects=0; fn work(n){effects+=1;return model.twice(n)}",
    )
    .unwrap();
    program
        .validate_strict_modules(&[("model", &model)])
        .unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[("model", &model)])
        .unwrap();
    assert!(instance.call("work", &[Value::Bool(true)], LIMITS).is_err());
    assert_eq!(instance.get("effects"), Some(Value::Number(0.)));
}
#[test]
fn each_instance_revalidates_replaced_interfaces_under_the_same_module_name() {
    let numeric = Program::compile("fn read(n:number)->number{return n}; export read").unwrap();
    let boolean =
        Program::compile("fn read(n:bool)->number{return if n {1} else {0}}; export read").unwrap();
    let program = Program::compile("import model; model.read(42)").unwrap();
    let token = CancellationToken::default();
    assert_eq!(
        program
            .instantiate(LIMITS, &token, &[], &[], &[], &[("model", &numeric)])
            .unwrap()
            .initial_value(),
        &Value::Number(42.)
    );
    assert!(
        program
            .instantiate(LIMITS, &token, &[], &[], &[], &[("model", &boolean)])
            .err()
            .unwrap()
            .message
            .contains("argument-type")
    );
}
#[test]
fn parameter_constraint_convergence_preserves_tuple_inference() {
    let program =
        Program::compile("fn f((x,y)){x+y; x+1; return y}; let n:number=f((1,2)); n").unwrap();
    assert_eq!(
        program
            .run(1000, &CancellationToken::default(), &[])
            .unwrap(),
        Value::Number(2.)
    );
    assert!(Program::compile("fn f((x,y)){x+y; x+1; return y}; let n:bool=f((1,2))").is_err());
}

#[test]
fn qualified_annotations_preserve_trivia_and_resolve_the_same_type() {
    let model = Program::compile("struct Settings {speed:number}; export Settings").unwrap();
    for annotation in [
        "model /* note */ . Settings",
        "model . /* note */ Settings",
        "model /* first */ . /* second */ Settings",
        "model // note\n . Settings",
    ] {
        let source = format!(
            "import model;
fn read(s:{annotation})->number {{
    return s.speed
}}
read(model.Settings({{speed:3}}))"
        );
        let formatted = format_source(&source).unwrap();
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        for source in [&source, &formatted] {
            let main = Program::compile(source).unwrap();
            assert_eq!(
                main.run_with_modules(
                    10_000,
                    &CancellationToken::default(),
                    &[],
                    &[],
                    &[("model", &model)]
                )
                .unwrap(),
                Value::Number(3.)
            );
        }
    }
}
