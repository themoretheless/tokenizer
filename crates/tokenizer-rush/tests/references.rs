use themoretheless_tokenizer_rush::analyze_references;

#[test]
fn navigation_resolves_shadowing_and_closure_capture() {
    let source = "const outer = 10\nfn f(outer) { return x => outer + x }\nouter + host";
    let (parsed, references) = analyze_references(source);
    assert!(parsed.is_valid(), "{:?}", parsed.diagnostics);
    let uses: Vec<_> = references
        .iter()
        .map(|r| {
            (
                &source[r.usage.start..r.usage.end],
                r.definition.map(|d| d.start),
            )
        })
        .collect();
    let parameter = source.find("f(outer)").unwrap() + 2;
    assert_eq!(
        uses,
        vec![
            ("outer", Some(parameter)),
            ("x", Some(source.find("x =>").unwrap())),
            ("outer", Some(source.find("outer").unwrap())),
            ("host", None)
        ]
    );
}

#[test]
fn record_keys_and_match_bindings_are_not_external_references() {
    let source = "const x = 1\nconst r = {label: x}\nmatch Some(x) { Some(value) => value }";
    let (parsed, references) = analyze_references(source);
    assert!(parsed.is_valid());
    assert!(
        !references
            .iter()
            .any(|r| &source[r.usage.start..r.usage.end] == "label")
    );
    let value = references.last().unwrap();
    assert_eq!(
        value.definition.unwrap().start,
        source.find("value").unwrap()
    );
    assert!(analyze_references("let x = ;").1.is_empty());
}

#[test]
fn strict_names_share_runtime_catalog_and_accept_explicit_host_names() {
    use themoretheless_tokenizer_rush::{analyze_names, builtin_catalog};
    for (name, _) in builtin_catalog() {
        assert!(analyze_names(name, &[]).is_valid(), "{name}");
    }
    assert!(analyze_names("let p = vec2(radius, 0)\ncustom(p)", &["radius", "custom"]).is_valid());
    let source = "let p = vec2(1, 2)\nif false { misspelled(p) }";
    let parsed = analyze_names(source, &[]);
    assert!(!parsed.is_valid());
    let diagnostic = parsed
        .diagnostics
        .iter()
        .find(|d| d.code == "unknown-name")
        .unwrap();
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "misspelled"
    );
    assert!(!analyze_names("fn f() { return later }\nlet later = 1", &[]).is_valid());
}
