use themoretheless_tokenizer_rush::{
    Program, Value, analyze_calls, analyze_editor_details, evaluate, format_source,
};

#[test]
fn structs_validate_fields_and_preserve_nominal_identity() {
    let source = "struct Settings { enabled: bool, speed: number }\nfn speed(settings: Settings) -> number { return settings.speed }\nlet settings = Settings({enabled: true, speed: 3}); speed(settings)";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(3.));
    for body in [
        "Settings({enabled:true})",
        "Settings({enabled:true,speed:1,extra:0})",
        "Settings({enabled:1,speed:2})",
        "Settings(1)",
        "Settings({enabled:true,speed:2}).missing",
        "speed({enabled:true,speed:2})",
    ] {
        let source = format!(
            "struct Settings {{ enabled: bool, speed: number }}; fn speed(s: Settings) -> number {{ return s.speed }}; if false {{ {body} }}"
        );
        assert!(Program::compile(&source).is_err(), "{source}");
    }
    assert_eq!(
        evaluate(
            "struct A { x: number }; struct B { x: number }; A({x:1}) == B({x:1})",
            1000
        )
        .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        evaluate("struct A { x: number }; A({x:1}) == A({x:1})", 1000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn enum_constructors_match_payloads_and_check_all_variants() {
    let source = "enum State { Idle, Moving(vec3) }; fn label(s: State) -> number { return match s { State.Idle() => 0, State.Moving(v) => v.x } }; label(State.Moving(vec3(7,2,3)))";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(7.));
    assert_eq!(evaluate("enum State { Idle, Moving(number) }; match State.Idle() { State.Idle() => 1, State.Moving(x) => x }",1000).unwrap(),Value::Number(1.));
    for body in [
        "State.Moving(true)",
        "State.Idle(1)",
        "State.Missing()",
        "match State.Idle() { State.Moving(x) => x }",
        "match State.Idle() { State.Idle() if (true) => 0, State.Moving(x) => x }",
        "match State.Idle() { State.Idle() => 0, State.Moving(0) => 1 }",
    ] {
        let source = format!("enum State {{ Idle, Moving(number) }}; if false {{ {body} }}");
        assert!(Program::compile(&source).is_err(), "{source}");
    }
    let source =
        "enum State { Idle, Moving(number) }; match State.Idle() { State.Moving(0) => 1, _ => 2 }";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(2.));
}

#[test]
fn nominal_annotations_work_inside_collections_and_aliases() {
    let source = "struct Point { x: number }; let make = Point; let points: list[Point] = [make({x:4})]; points[0].x";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(4.));
    let source = "enum Event { Tick, Change(number) }; let change = Event.Change; let e: Event = change(3); match e { Event.Tick() => 0, Event.Change(x) => x+1 }";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(4.));
    assert!(
        Program::compile("struct Point { x: number }; let points: list[Point] = [{x:1}]").is_err()
    );
    assert!(
        Program::compile("enum A { X }; enum B { X }; fn f(v: A) { return v }; f(B.X())").is_err()
    );
}

#[test]
fn patterns_keep_payload_contracts_and_constructor_references() {
    assert!(
        Program::compile("enum E { A(number), B }; match E.B() { E.A(x) => x + true, E.B() => 0 }")
            .is_err()
    );
    assert!(
        Program::compile("enum E { A(number), B }; match E.B() { E.A() => 0, E.B() => 1 }")
            .is_err()
    );
    assert!(
        Program::compile("enum E { A }; enum F { A }; match E.A() { F.A() => 0, _ => 1 }").is_err()
    );
    let source = "enum E { A, B(number) }; fn f(e: E) { return match e { E.A() => 0, E.B(x) => x } }; f(E.B(9))";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(9.));
    let source = "struct Point { x: number }; match Point({x:5}) { {x:value} => value }";
    assert_eq!(evaluate(source, 1000).unwrap(), Value::Number(5.));
}

#[test]
fn finite_builtin_domains_require_exhaustive_unguarded_patterns() {
    for source in [
        "match true { true => 1 }",
        "fn f(x: Option[number]) { return match x { Some(v) => v } }",
        "fn f(x: Result[number,str]) { return match x { Ok(v) => v } }",
    ] {
        assert!(
            analyze_calls(source)
                .diagnostics
                .iter()
                .any(|d| d.code == "non-exhaustive-match"),
            "{source}"
        );
    }
    assert!(Program::compile("match true { true => 1, false => 2 }").is_ok());
}

#[test]
fn type_syntax_formats_stably_and_editor_completes_fields() {
    let source = "// types\nstruct Point { x:number, y:number }\nenum State {Idle, Moving(Point)}\nlet p=Point({x:1,y:2}); p.x";
    let formatted = format_source(source).unwrap();
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    assert_eq!(evaluate(&formatted, 1000).unwrap(), Value::Number(1.));
    let details =
        analyze_editor_details("struct Point { x:number, y:number }; let p=Point({x:1,y:2}); p.");
    assert_eq!(
        details.member_completions.last().unwrap().members,
        vec!["x", "y"]
    );
}

#[test]
fn type_declarations_reject_duplicates_and_reserved_names() {
    for source in [
        "struct A {x:number,x:bool}",
        "enum E {A,A}",
        "enum E {}",
        "struct number {}",
        "fn f() { struct Local {} }",
    ] {
        assert!(Program::compile(source).is_err(), "{source}");
    }
}

#[test]
fn struct_destructuring_checks_fields_and_preserves_their_types() {
    assert_eq!(
        evaluate(
            "struct P { x:number }; let {x:value}=P({x:4}); value+1",
            1000
        )
        .unwrap(),
        Value::Number(5.)
    );
    assert!(Program::compile("struct P { x:number }; let {x:value}=P({x:4}); value+true").is_err());
    assert!(Program::compile("struct P { x:number }; match P({x:4}) { {x:0} => 1 }").is_err());
    assert!(Program::compile("enum E { A(number) }; match E.A(4) { E.A((a,b)) => 1 }").is_err());
}

#[test]
fn editor_resolves_user_type_annotations_for_navigation_and_rename() {
    let source = "struct P { x:number }; fn f(v: list[P]) -> P { return v[0] }; f([P({x:1})])";
    let details = analyze_editor_details(source);
    let definition = details
        .bindings
        .iter()
        .find(|binding| binding.name == "P")
        .unwrap()
        .definition;
    let uses: Vec<_> = details
        .references
        .iter()
        .filter(|reference| reference.definition == Some(definition))
        .collect();
    assert_eq!(uses.len(), 3);
}
