use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};

#[test]
fn modules_export_functions_and_keep_initialization_state_per_run() {
    let library =
        Program::compile("mut count = 0\nfn next() { count += 1; return count }\n{next: next}")
            .unwrap();
    let main = Program::compile("import counters\nlet first = counters.next()\nfn second() { import counters; return counters.next() }\n(first, second())").unwrap();
    for _ in 0..2 {
        assert_eq!(
            main.run_with_modules(
                500,
                &CancellationToken::default(),
                &[],
                &[],
                &[("counters", &library)]
            )
            .unwrap(),
            Value::Tuple(vec![Value::Number(1.), Value::Number(2.)])
        );
    }
}

#[test]
fn cyclic_missing_and_invalid_modules_fail() {
    let a = Program::compile("import b\n{}").unwrap();
    let b = Program::compile("import a\n{}").unwrap();
    let main = Program::compile("import a\na").unwrap();
    assert!(
        main.run_with_modules(
            500,
            &CancellationToken::default(),
            &[],
            &[],
            &[("a", &a), ("b", &b)]
        )
        .unwrap_err()
        .message
        .contains("Cyclic")
    );
    assert!(main.run(100, &CancellationToken::default(), &[]).is_err());
    let invalid = Program::compile("42").unwrap();
    assert!(
        main.run_with_modules(
            100,
            &CancellationToken::default(),
            &[],
            &[],
            &[("a", &invalid)]
        )
        .is_err()
    );
}

#[test]
fn errors_retain_source_module_through_exported_closures() {
    let source = "fn make() { return x => x / 0 }\n{fail: make()}";
    let library = Program::compile(source).unwrap();
    let main = Program::compile("import broken\nbroken.fail(2)").unwrap();
    let error = main
        .run_with_modules(
            500,
            &CancellationToken::default(),
            &[],
            &[],
            &[("broken", &library)],
        )
        .unwrap_err();
    assert_eq!(error.module.as_deref(), Some("broken"));
    assert_eq!(&source[error.span.start..error.span.end], "x / 0");
    let main = Program::compile("import broken\n1 / 0").unwrap();
    assert_eq!(
        main.run_with_modules(
            500,
            &CancellationToken::default(),
            &[],
            &[],
            &[("broken", &library)]
        )
        .unwrap_err()
        .module,
        None
    );
}

#[test]
fn module_graph_validation_finds_imports_in_unexecuted_functions() {
    let main = Program::compile("fn later() { import missing }\n42").unwrap();
    assert!(
        main.validate_modules(&[])
            .unwrap_err()
            .message
            .contains("missing")
    );
    let a = Program::compile("fn later() { import b }\n{}").unwrap();
    let b = Program::compile("import a\n{}").unwrap();
    let main = Program::compile("import a").unwrap();
    let error = main.validate_modules(&[("a", &a), ("b", &b)]).unwrap_err();
    assert!(error.message.contains("Cyclic"));
    assert_eq!(error.module.as_deref(), Some("b"));
    let b = Program::compile("{}").unwrap();
    assert!(main.validate_modules(&[("a", &a), ("b", &b)]).is_ok());
}

#[test]
fn mesh_library_offsets_indices_when_joining_components() {
    let library = Program::compile(include_str!("../examples/scripts/meshes.r")).unwrap();
    let main = Program::compile("import meshes\nconst a = mesh([vec3(0,0,0),vec3(1,0,0),vec3(0,1,0)], [[0,1,2]])\nmeshes.join(a, transform(a, translation(vec3(0,0,2))))").unwrap();
    let Value::Mesh(mesh) = main
        .run_with_modules(
            2000,
            &CancellationToken::default(),
            &[],
            &[],
            &[("meshes", &library)],
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(mesh.vertices().len(), 6);
    assert_eq!(mesh.triangles(), &[[0, 1, 2], [3, 4, 5]]);
    assert_eq!(mesh.vertices()[3], [0., 0., 2.]);
}
