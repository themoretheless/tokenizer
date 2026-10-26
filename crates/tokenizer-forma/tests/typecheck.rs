use serde_json::{Value, json};
use themoretheless_tokenizer_forma::lint_document_with_context;
use themoretheless_tokenizer_forma::typecheck::{TypeContext, analyze_source};

fn report(source: &str) -> themoretheless_tokenizer_forma::typecheck::TypecheckReport {
    analyze_source(source, &TypeContext::default()).unwrap()
}

fn incompatible(source: &str, code: &str) -> Value {
    let diagnostics = report(source).diagnostics;
    let matches = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "{source}\n{diagnostics:#?}");
    matches[0].clone()
}

fn utf16_range(source: &str, needle: &str) -> (usize, usize) {
    let byte = source.find(needle).unwrap();
    let from = source[..byte].encode_utf16().count();
    (from, from + needle.encode_utf16().count())
}

fn assert_range(source: &str, code: &str, needle: &str) -> Value {
    let diagnostic = incompatible(source, code);
    let (from, to) = utf16_range(source, needle);
    assert_eq!(diagnostic["from"], from, "{diagnostic:#?}");
    assert_eq!(diagnostic["to"], to, "{diagnostic:#?}");
    assert_eq!(diagnostic["severity"], "error");
    assert!(diagnostic["expected"].is_string());
    assert!(diagnostic["actual"].is_string());
    diagnostic
}

#[test]
fn bindings_arguments_conditions_operators_and_units_are_checked() {
    assert_range(
        "component A { state name: String = '😀'; Text { width: state.name; } }",
        "incompatible-binding",
        "state.name",
    );
    assert_range(
        "component A { state n: Number = 1; Text { text: trim(state.n); } }",
        "incompatible-argument",
        "state.n",
    );
    assert_range(
        "component A { state name: String = 'x'; if state.name { Text {} } }",
        "incompatible-condition",
        "state.name",
    );
    assert_range(
        "component A { state name: String = 'x'; Text { width: state.name + 1; } }",
        "incompatible-operand",
        "state.name + 1",
    );
    assert_range(
        "component A { Text { transitionDuration: 1px; } }",
        "incompatible-binding",
        "1px",
    );
    assert_range(
        "component A { state n: Number = 1; TextField { value <-> state.n; } }",
        "incompatible-binding",
        "value <-> state.n;",
    );
}

#[test]
fn match_patterns_branches_and_guards_have_expression_ranges() {
    assert_range(
        "component A { state n: Number = 1; Text { text: match state.n { '😀' => 'x'; _ => 'y'; }; } }",
        "incompatible-match-pattern",
        "'😀'",
    );
    assert_range(
        "component A { state flag: Bool = true; Text { width: match state.flag { true => 1px; _ => 'bad'; }; } }",
        "incompatible-match-branch",
        "'bad'",
    );
    let source = "component A { state n: Number = 1; Text { text: match state.n { 1 if state.n => 'x'; _ => 'y'; }; } }";
    let diagnostic = incompatible(source, "incompatible-condition");
    let byte = source.rfind("state.n").unwrap();
    let from = source[..byte].encode_utf16().count();
    assert_eq!(diagnostic["from"], from);
    assert_eq!(diagnostic["to"], from + "state.n".len());
}

#[test]
fn nested_interpolation_offsets_are_absolute_utf16() {
    let source = "component A { state n: Number = 1; Text { text: '😀 ${trim(state.n)}'; } }";
    let diagnostic = incompatible(source, "incompatible-argument");
    let byte = source.rfind("state.n").unwrap();
    let from = source[..byte].encode_utf16().count();
    assert_eq!(diagnostic["from"], from);
    assert_eq!(diagnostic["to"], from + "state.n".encode_utf16().count());
}

#[test]
fn unknown_any_and_missing_schemas_suppress_diagnostics() {
    for source in [
        "component A { state value: Unknown = external.value; Text { width: state.value; } }",
        "component A { state value: String | Any = 'x'; Text { width: state.value; } }",
        "component A { state value: Any | String = 'x'; Text { width: state.value; } }",
        "component A { state name: String = 'x'; Text { width: state.name; } }",
        "component A { state values: List<Number> = []; Text { width: sum(state.values); } }",
        "component A { text: 1; Text { text: props.text; } }",
        "component A { Text { color: invert(#fff); } }",
    ] {
        let context = if source.contains("width: state.name") {
            TypeContext::empty()
        } else {
            TypeContext::default()
        };
        let diagnostics = analyze_source(source, &context).unwrap().diagnostics;
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
    }
}

#[test]
fn known_union_members_still_prove_incompatibility() {
    assert_range(
        "component A { state value: String | Int = 1; Text { width: state.value; } }",
        "incompatible-binding",
        "state.value",
    );
    let source = "component A { state n: Number = 1; Text { width: state.n; } }";
    assert!(report(source).diagnostics.is_empty());
}

#[test]
fn host_context_overrides_builtins_and_types_child_components_and_actions() {
    let context = TypeContext::from_json(&json!({
        "properties": {"TextField": {"value": "Number"}},
        "components": {"Child": {"title": "String"}},
        "actions": {"save": ["String"]}
    }))
    .unwrap();
    let good = "component A { state n: Number = 1; TextField { value <-> state.n; } Child { title: 'ok'; clicked -> actions.save('ok'); } }";
    assert!(
        analyze_source(good, &context)
            .unwrap()
            .diagnostics
            .is_empty()
    );
    let bad = "component A { state n: Number = 1; Child { title: state.n; clicked -> actions.save(state.n); } }";
    let diagnostics = analyze_source(bad, &context).unwrap().diagnostics;
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "incompatible-binding")
            .count(),
        1
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d["code"] == "incompatible-argument")
            .count(),
        1
    );
}

#[test]
fn public_lint_adds_only_additive_diagnostics() {
    let source = "component A { state name: String = 'x'; Text { width: state.name; } }";
    let diagnostics = lint_document_with_context(source, &TypeContext::default()).unwrap();
    let diagnostic = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "incompatible-binding")
        .unwrap();
    assert_eq!(diagnostic["severity"], "error");
    assert!(TypeContext::from_json(&json!([])).is_err());
}

#[test]
fn corpus_has_no_unreviewed_type_diagnostics_and_has_real_coverage() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/forma-corpus.json")).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 87);
    let context = TypeContext::default();
    let mut total = 0;
    let mut typed = 0;
    let mut diagnostics = Vec::new();
    for case in cases.as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let report = analyze_source(case["source"].as_str().unwrap(), &context).unwrap();
        total += report.expressions_total;
        typed += report.expressions_typed;
        for diagnostic in report.diagnostics {
            if diagnostic["code"]
                .as_str()
                .unwrap_or("")
                .starts_with("incompatible-")
            {
                eprintln!(
                    "CORPUS {file} {} {}..{} expected={} actual={}",
                    diagnostic["code"],
                    diagnostic["from"],
                    diagnostic["to"],
                    diagnostic["expected"],
                    diagnostic["actual"]
                );
                diagnostics.push((file, diagnostic));
            }
        }
    }
    eprintln!("CORPUS COVERAGE typed={typed} total={total}");
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    assert_eq!(
        (typed, total),
        (1724, 2247),
        "Reviewed corpus coverage baseline"
    );
}
