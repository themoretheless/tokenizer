use themoretheless_tokenizer_rush::{Program, analyze_calls};

#[test]
fn loading_checks_unexecuted_code() {
    for (source, code) in [
        ("if false { let x: bool = 1 }", "annotation-type"),
        ("fn unused() -> bool { return 1 }", "return-type"),
        (
            "fn f(x: bool) { return x }; if false { f(1) }",
            "argument-type",
        ),
        ("fn unused() { return 'text' - 1 }", "arithmetic-operands"),
        ("false and (1 + true == 2)", "arithmetic-operands"),
        ("if false { [1] + [2] }", "arithmetic-operands"),
    ] {
        let analysis = analyze_calls(source);
        assert!(
            analysis.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
        let error = Program::compile(source)
            .err()
            .expect("must reject at load time");
        assert!(error.message.starts_with(code), "{source}: {error:?}");
    }
}

#[test]
fn known_locals_propagate_into_operations() {
    assert!(Program::compile("let text = 'hello'; fn unused() { return text / 2 }").is_err());
    for source in [
        "let x = 1; let y = x + 2; fn f(n: number) -> number { return n }; f(y)",
        "'hello' + ' world'",
        "vec3(1,2,3) * 2",
        "degrees(90) + degrees(45)",
        "fn f(x) { return x + 1 }; f(2)",
        "external_value + 1",
    ] {
        assert!(Program::compile(source).is_ok(), "{source}");
    }
}

#[test]
fn annotated_mutable_bindings_keep_their_contract_in_dead_code() {
    for source in [
        "mut n: number = 1; if false { n = true }",
        "mut n: number = 1; fn unused() { return n + false }",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
    assert!(Program::compile("mut n: number = 1; n = 2; n + 3").is_ok());
}

#[test]
fn mutable_scalar_types_are_inferred_and_compound_assignments_are_checked() {
    for source in [
        "mut n=1; if false {n=true}",
        "mut n=1; fn unused() {n+=false}",
        "mut n=1; n*=vec3(1,2,3)",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
    let program = Program::compile("mut n=1; n=external").unwrap();
    let error = program
        .run_with_values(
            themoretheless_tokenizer_rush::ExecutionLimits::new(100),
            &Default::default(),
            &[("external", themoretheless_tokenizer_rush::Value::Bool(true))],
            &[],
            &[],
        )
        .unwrap_err();
    assert!(error.message.contains("type"));
}

#[test]
fn all_conditional_results_are_checked_even_when_the_bad_branch_is_dead() {
    for source in [
        "fn unused() -> number { return if true {1} else {'wrong'} }",
        "enum E { A,B }; fn unused(e:E) -> number { return match e { E.A()=>1,E.B()=>'wrong' } }",
        "enum E { A,B }; let v=match E.A() {E.A()=>1,E.B()=>'wrong'}; fn f(x:number) {return x}; f(v)",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
}

#[test]
fn matching_result_metadata_does_not_override_following_member_access() {
    let source =
        "enum E {A,B}; let x:number=(match E.A() {E.A()=>vec3(1,2,3),E.B()=>vec3(4,5,6)}).x; x";
    assert_eq!(
        themoretheless_tokenizer_rush::evaluate(source, 1000).unwrap(),
        themoretheless_tokenizer_rush::Value::Number(1.)
    );
}

#[test]
fn known_function_returns_are_inferred_without_executing_the_function() {
    for source in [
        "fn make() { return 'wrong' }; fn unused() -> number { return make() }",
        "fn make() { return {x:1} }; fn unused() { return make().missing }",
        "fn make() { return vec2(1,2) }; let alias=make; fn unused() { return alias().z }",
        "fn make() { 1 }; let n:number=make()",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
    assert!(Program::compile("fn make() { return vec3(1,2,3) }; let v:vec3=make(); v.z").is_ok());
}
