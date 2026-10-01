use themoretheless_tokenizer_rush::{LexicalBinding, analyze_editor};

fn visible(source: &str, offset: usize) -> Vec<LexicalBinding> {
    let (parsed, _, mut bindings) = analyze_editor(source);
    assert!(parsed.is_valid(), "{:?}", parsed.diagnostics);
    bindings.retain(|binding| binding.visible.start <= offset && offset < binding.visible.end);
    bindings.sort_by_key(|binding| std::cmp::Reverse((binding.depth, binding.visible.start)));
    let mut names = std::collections::HashSet::new();
    bindings.retain(|binding| names.insert(binding.name.clone()));
    bindings
}

#[test]
fn declarations_are_visible_after_initialization_and_at_eof() {
    let source = "let first = 1\nlet next = first\nnext\n";
    assert!(visible(source, source.find("1").unwrap()).is_empty());
    let names = visible(source, source.rfind("first").unwrap());
    assert_eq!(
        names.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
        ["first"]
    );
    assert_eq!(visible(source, source.len()).len(), 2);
}

#[test]
fn function_and_lambda_parameters_stay_in_their_bodies() {
    let source = "let outer = 1\nfn f({point: (x,y)}) { let local = x; return local+y+outer }\nlet g = ((a,b)) => a+b+outer\nouter";
    let names = visible(source, source.find("return").unwrap());
    for name in ["f", "x", "y", "local", "outer"] {
        assert!(names.iter().any(|b| b.name == name), "{name}");
    }
    assert!(!names.iter().any(|b| b.name == "point" || b.name == "g"));
    let names = visible(source, source.find("a+b").unwrap());
    assert!(names.iter().any(|b| b.name == "a"));
    assert!(!names.iter().any(|b| b.name == "x" || b.name == "g"));
    let names = visible(source, source.len());
    assert_eq!(names.len(), 3);
}

#[test]
fn nested_shadowing_loops_and_match_guards_have_precise_intervals() {
    let source = "let x = 1\nfor x in [x] { let y = x; y }\nmatch Some(x) { Some(value) if (value > 0) => value, _ => x }\nx";
    let names = visible(source, source.find("let y").unwrap());
    assert_eq!(
        names
            .iter()
            .find(|b| b.name == "x")
            .unwrap()
            .definition
            .start,
        source.find("x in").unwrap()
    );
    let names = visible(source, source.find("[x]").unwrap() + 1);
    assert_eq!(
        names
            .iter()
            .find(|b| b.name == "x")
            .unwrap()
            .definition
            .start,
        4
    );
    assert!(
        visible(source, source.find("value >").unwrap())
            .iter()
            .any(|b| b.name == "value")
    );
    assert_eq!(visible(source, source.len()).len(), 1);
    assert!(analyze_editor("fn broken(").2.is_empty());
}

#[test]
fn destructured_records_keep_nested_members() {
    for declaration in [
        "let (config, other) = ({camera: {position: vec3(1,2,3)}}, 0)",
        "let {settings: config} = {settings: {camera: {position: vec3(1,2,3)}}}",
    ] {
        let source = format!("{declaration}\nconfig");
        let bindings = visible(&source, source.len());
        let config = bindings.iter().find(|b| b.name == "config").unwrap();
        assert_eq!(config.members, ["camera"]);
        assert_eq!(config.member_paths["camera"], ["position"]);
        assert_eq!(config.member_paths["camera.position"], ["x", "y", "z"]);
    }
}

#[test]
fn destructured_record_unknown_field_is_diagnosed() {
    let source = "let (config, _) = ({camera: {position: vec3(1,2,3)}}, 0)\nconfig.camera.missing";
    let parsed = themoretheless_tokenizer_rush::analyze_host_calls(source, &[]);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.code == "unknown-record-field")
    );
}

#[test]
fn destructuring_known_record_aliases_preserves_nested_shapes() {
    let source = "let source = {settings:{position:vec3(1,2,3)}}\nlet alias = source\nlet {settings:config} = alias\nlet {position:point} = config\npoint";
    let bindings = visible(source, source.len());
    assert_eq!(
        bindings
            .iter()
            .find(|b| b.name == "config")
            .unwrap()
            .members,
        ["position"]
    );
    assert_eq!(
        bindings.iter().find(|b| b.name == "point").unwrap().members,
        ["x", "y", "z"]
    );
}

#[test]
fn destructuring_mutable_records_does_not_assume_initial_fields() {
    let source = "let mut source = {settings:{first:1}}\nsource = {settings:{second:2}}\nlet {settings:config} = source\nconfig";
    let bindings = visible(source, source.len());
    let config = bindings.iter().find(|b| b.name == "config").unwrap();
    assert!(config.members.is_empty());
    assert!(config.member_paths.is_empty());
}

#[test]
fn annotated_mesh_parameter_has_field_completions() {
    let source = "fn inspect(m: mesh) { return m.vertices }";
    let bindings = visible(source, source.find("m.vertices").unwrap());
    assert_eq!(
        bindings.iter().find(|b| b.name == "m").unwrap().members,
        ["triangles", "vertices"]
    );
}

#[test]
fn tuple_alias_destructuring_retains_record_members() {
    let source =
        "let pair = ({position:vec3(1,2,3)}, 0)\nlet alias = pair\nlet (config, _) = alias\nconfig";
    let bindings = visible(source, source.len());
    let config = bindings.iter().find(|b| b.name == "config").unwrap();
    assert_eq!(config.members, ["position"]);
    assert_eq!(config.member_paths["position"], ["x", "y", "z"]);
}

#[test]
fn literal_record_indices_preserve_nested_fields_and_vector_axes() {
    for key in [r#""camera""#, "'camera'", r#""\u{63}amera""#] {
        let source = format!(
            "let config = {{camera:{{position:vec3(1,2,3)}}}}; let camera = config[{key}]; let point = camera[\"position\"]; point"
        );
        let bindings = visible(&source, source.len());
        let camera = bindings
            .iter()
            .find(|binding| binding.name == "camera")
            .unwrap();
        assert_eq!(camera.members, ["position"]);
        assert_eq!(camera.member_paths["position"], ["x", "y", "z"]);
        assert_eq!(
            bindings
                .iter()
                .find(|binding| binding.name == "point")
                .unwrap()
                .members,
            ["x", "y", "z"]
        );
    }
}

#[test]
fn unknown_record_index_has_precise_span_but_dynamic_and_mutable_keys_are_not_assumed() {
    let source = r#"let config = {camera:1}; config["missing"]"#;
    let parsed = themoretheless_tokenizer_rush::analyze_host_calls(source, &[]);
    let diagnostic = parsed
        .diagnostics
        .iter()
        .find(|d| d.code == "unknown-record-field")
        .unwrap();
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "\"missing\""
    );
    for source in [
        r#"fn f(key: string) { let config={camera:1}; return config[key] }"#,
        r#"mut config={first:1}; config={second:2}; let result=config["second"]; result"#,
    ] {
        let parsed = themoretheless_tokenizer_rush::analyze_host_calls(source, &[]);
        assert!(
            !parsed
                .diagnostics
                .iter()
                .any(|d| d.code == "unknown-record-field"),
            "{source}"
        );
    }
}

#[test]
fn expression_receivers_have_member_candidates_at_the_actual_cursor() {
    use themoretheless_tokenizer_rush::analyze_editor_details;
    for (source, expected) in [
        ("vec3(1,2,3).", vec!["x", "y", "z"]),
        ("(vec2(1,2)).", vec!["x", "y"]),
        ("[vec3(1,2,3)][0].", vec!["x", "y", "z"]),
        (
            r#"let config={camera:{position:vec3(1,2,3)}}; config["camera"]."#,
            vec!["position"],
        ),
        (
            "fn point() -> vec3 { return vec3(1,2,3) }; point().",
            vec!["x", "y", "z"],
        ),
        (
            "fn vec3(x: number) -> number { return x }; vec3(1).",
            vec![],
        ),
        ("let имя={point:vec2(1,2)}; имя.point.", vec!["x", "y"]),
        ("unknown().", vec![]),
    ] {
        let result = analyze_editor_details(source);
        assert!(
            !result.parsed.is_valid(),
            "completion recovery must not permit execution"
        );
        let candidates = result
            .member_completions
            .iter()
            .find(|c| c.name_span.start == source.len())
            .unwrap();
        assert_eq!(candidates.members, expected, "{source}");
        assert_eq!(candidates.name_span.end, source.len());
    }
    assert!(
        analyze_editor_details("fn broken(\nvec3(1,2,3).")
            .member_completions
            .is_empty()
    );
}

#[test]
fn vector_math_result_shapes_follow_arguments_aliases_and_pipeline_stages() {
    use themoretheless_tokenizer_rush::analyze_editor_details;
    for (source, expected) in [
        ("normalize(vec2(1,2)).", vec!["x", "y"]),
        (
            "(vec4(1,2,3,4) | normalize | normalize).",
            vec!["x", "y", "z", "w"],
        ),
        ("let n=normalize; n(vec3(1,2,3)).", vec!["x", "y", "z"]),
        ("cross(vec3(1,0,0),vec3(0,1,0)).", vec!["x", "y", "z"]),
        ("lerp(vec2(1,2),vec2(3,4),0.5).", vec!["x", "y"]),
        (
            "(vec3(1,2,3) | lerp(vec3(4,5,6),0.5) | normalize).",
            vec!["x", "y", "z"],
        ),
        (
            "transform_point(identity(),vec3(1,2,3)).",
            vec!["x", "y", "z"],
        ),
        ("normalize(unknown).", vec![]),
        ("lerp(vec2(1,2),vec3(1,2,3),0.5).", vec![]),
        ("lerp(1,2,0.5).", vec![]),
        (
            "fn normalize(v:vec3)->number { return 1 }; normalize(vec3(1,2,3)).",
            vec![],
        ),
    ] {
        let details = analyze_editor_details(source);
        let completion = details
            .member_completions
            .iter()
            .find(|c| c.name_span.start == source.len())
            .unwrap();
        assert_eq!(completion.members, expected, "{source}");
    }
}
