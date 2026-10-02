use themoretheless_tokenizer_rush::{
    Program, analyze_editor_project, import_completions, rename_project_symbol,
};
#[test]
fn imported_fields_variants_and_annotations_point_to_their_document() {
    let a = "struct Settings{speed:number}; enum State{Idle,Moving(Settings)}; fn make()->Settings{return Settings({speed:2})}; export Settings,State,make";
    let b = "import model; fn read(s:model.Settings)->number{return s.speed}; let state=model.State.Idle(); read(model.make())";
    let model = Program::compile(a).unwrap();
    let main = Program::compile(b).unwrap();
    let docs = analyze_editor_project(&[("model", &model), ("main", &main)]);
    let editor = &docs[1].analysis;
    for (usage, definition) in [
        ("speed", "speed"),
        ("Idle", "Idle"),
        ("Settings", "Settings"),
        ("make", "make"),
    ] {
        let pos = b.rfind(usage).unwrap();
        let reference = editor
            .references
            .iter()
            .find(|r| r.usage.start == pos)
            .unwrap();
        assert_eq!(reference.definition_module.as_deref(), Some("model"));
        assert_eq!(
            &a[reference.definition.unwrap().start..reference.definition.unwrap().end],
            definition
        );
    }
    assert_eq!(
        import_completions(&[("model", &model), ("main", &main)], "mo"),
        vec!["model"]
    );
}
#[test]
fn rename_changes_only_the_nominal_field_across_documents() {
    let a = "struct Settings{speed:number}; struct Other{speed:number}; fn make()->Settings{return Settings({speed:2})}; export Settings,Other,make";
    let b = "import model; fn read(s:model.Settings)->number{return s.speed}; let other=model.Other({speed:3}); read(model.make())";
    let model = Program::compile(a).unwrap();
    let main = Program::compile(b).unwrap();
    let registry = [("model", &model), ("main", &main)];
    let edits = rename_project_symbol(
        &registry,
        "main",
        b.find("s.speed").unwrap() + 2,
        "velocity",
    )
    .unwrap();
    assert_eq!(edits.len(), 3);
    assert_eq!(edits.iter().filter(|e| e.module == "main").count(), 1);
    assert!(
        rename_project_symbol(&registry, "main", b.find("s.speed").unwrap() + 2, "number").is_ok()
    );
    assert!(
        rename_project_symbol(&registry, "main", b.find("s.speed").unwrap() + 2, "if").is_err()
    );
}
#[test]
fn variants_are_renamed_in_constructors_and_patterns() {
    let source = "enum State{Idle,Moving(number)}; let state=State.Idle(); match state {State.Idle()=>0,State.Moving(n)=>n}";
    let p = Program::compile(source).unwrap();
    let edits = rename_project_symbol(
        &[("main", &p)],
        "main",
        source.find("Idle").unwrap(),
        "Stopped",
    )
    .unwrap();
    assert_eq!(edits.len(), 3);
}
#[test]
fn rename_rejects_capture_and_duplicate_fields() {
    let source = "let x=1; fn f(y:number)->number{return x+y}; f(2)";
    let p = Program::compile(source).unwrap();
    assert!(
        rename_project_symbol(&[("main", &p)], "main", source.find("x=").unwrap(), "y").is_err()
    );
    let source = "struct S{x:number,y:number}; let s=S({x:1,y:2}); s.x";
    let p = Program::compile(source).unwrap();
    assert!(
        rename_project_symbol(&[("main", &p)], "main", source.find("x:").unwrap(), "y").is_err()
    );
}
#[test]
fn rename_includes_nominal_destructuring_and_parameter_patterns() {
    let source = "struct S{x:number}; let s=S({x:1}); let {x:value}=s; fn read({x:item}:S)->number{return item}; read(s)+value";
    let program = Program::compile(source).unwrap();
    let edits = rename_project_symbol(
        &[("main", &program)],
        "main",
        source.find("x:").unwrap(),
        "coordinate",
    )
    .unwrap();
    assert_eq!(edits.len(), 4);
}
#[test]
fn renaming_a_type_preserves_differently_named_public_aliases() {
    let source = "struct Point{x:number}; export Point";
    let model = Program::compile(source).unwrap();
    let api = Program::compile("import model; let Alias=model.Point; export Alias").unwrap();
    let main = Program::compile("import api; fn read(p:api.Alias)->number{return p.x}").unwrap();
    let registry = [("model", &model), ("api", &api), ("main", &main)];
    let edits = rename_project_symbol(
        &registry,
        "model",
        source.find("Point").unwrap(),
        "Position",
    )
    .unwrap();
    assert_eq!(edits.len(), 3);
    assert!(!edits.iter().any(|e| e.module == "main"));
}

#[test]
fn project_import_cycles_and_missing_modules_match_load_rejection() {
    let a = Program::compile("import b; let value=1; export value").unwrap();
    let b = Program::compile("import a; let value=2; export value").unwrap();
    let docs = analyze_editor_project(&[("a", &a), ("b", &b)]);
    assert!(a.validate_modules(&[("a", &a), ("b", &b)]).is_err());
    assert!(docs.iter().any(|doc| {
        doc.analysis
            .parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "cyclic-import")
    }));
    let missing = Program::compile("if false {import absent}; 1").unwrap();
    let docs = analyze_editor_project(&[("main", &missing)]);
    assert!(missing.validate_modules(&[]).is_err());
    assert!(
        docs[0]
            .analysis
            .parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "unknown-module")
    );
}

#[test]
fn project_graph_allows_shared_dependencies_and_rejects_duplicate_registrations() {
    let leaf = Program::compile("let value=1;export value").unwrap();
    let left = Program::compile("import leaf;let value=leaf.value;export value").unwrap();
    let right = Program::compile("import leaf;let value=leaf.value;export value").unwrap();
    let root = Program::compile("import left;import right;left.value+right.value").unwrap();
    let registry = [
        ("root", &root),
        ("left", &left),
        ("right", &right),
        ("leaf", &leaf),
    ];
    assert!(root.validate_modules(&registry).is_ok());
    assert!(
        analyze_editor_project(&registry)
            .iter()
            .all(|doc| doc.analysis.parsed.is_valid())
    );
    let docs = analyze_editor_project(&[("leaf", &leaf), ("leaf", &leaf)]);
    assert!(docs.iter().all(|doc| {
        doc.analysis
            .parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "duplicate-module")
    }));
}
