use serde_json::{Value, json};
use themoretheless_tokenizer_forma::{evaluate, parse, parse_expression, semantics};
#[test]
fn original_studio_corpus_has_identical_ast_and_editor_offsets() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/forma-corpus.json")).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 87);
    for case in cases.as_array().unwrap() {
        let got = parse(case["source"].as_str().unwrap()).unwrap();
        assert!(evaluate::equal(&got, &case["ast"]), "{}", case["file"]);
    }
}
#[test]
fn malformed_source_is_rejected_and_nesting_is_bounded() {
    for s in [
        "component A { Text { text: ; } }",
        "component A { prop x: String; }",
        "component A { Text { key: 'a'; } Text { key: 'a'; } }",
        "component A { Text { clicked -> evil.run(); } }",
        "component A { event go(x: String, x: Number); }",
    ] {
        assert!(parse(s).is_err(), "{s}");
    }
    assert!(parse_expression(&format!("{}1{}", "(".repeat(140), ")".repeat(140))).is_err());
}
#[test]
fn unicode_ranges_use_utf16_and_tokens_reconstruct_significant_source() {
    let s = "component A { Text { text: '😀'; color: #fff; } }";
    let p = parse(s).unwrap();
    let range = &p["nodes"][0]["propertyRanges"]["color"];
    let start = s.find("#fff").unwrap();
    assert_eq!(range["from"], s[..start].encode_utf16().count());
    assert_eq!(range["to"], s[..start + 4].encode_utf16().count());
}
#[test]
fn evaluation_preserves_short_circuit_optional_index_and_numeric_errors() {
    let reads = std::cell::Cell::new(0);
    let mut resolver = |_: &str, _: bool| {
        reads.set(reads.get() + 1);
        Ok(Value::Null)
    };
    assert_eq!(
        evaluate::evaluate(
            &parse_expression("false && state.bad").unwrap(),
            &mut resolver
        )
        .unwrap(),
        false
    );
    assert_eq!(
        evaluate::evaluate(
            &parse_expression("state.missing?.[state.bad]").unwrap(),
            &mut resolver
        )
        .unwrap(),
        Value::Null
    );
    assert_eq!(reads.get(), 1);
    assert!(evaluate::evaluate(&parse_expression("1 / 0").unwrap(), &mut resolver).is_err());
    assert_eq!(
        evaluate::evaluate(&parse_expression("len('😀')").unwrap(), &mut resolver).unwrap(),
        2
    );
}
#[test]
fn contracts_cycles_and_keyed_expansion_are_checked() {
    let props = json!({"a":{"expr":"props.b"},"b":{"expr":"props.a"}});
    assert!(
        semantics::evaluate_value(
            &json!({"expr":"props.a"}),
            &props,
            &json!({}),
            &[],
            false,
            &json!({})
        )
        .unwrap_err()
        .contains("Циклическая")
    );
    assert!(
        semantics::validate_contract(
            &json!({"title":{"type":"String","required":true}}),
            &json!({}),
            &json!({}),
            "A"
        )
        .is_err()
    );
    let p =
        parse("component A { for item in state.items key item.id { Text { text: item.name; } } }")
            .unwrap();
    let state = json!({"items":[{"id":"a","name":"First"},{"id":"b","name":"Second"}]});
    let expanded = semantics::expand_structure(
        &p["nodes"],
        &json!({}),
        &state,
        &json!({}),
        &json!({}),
        &mut |_, _| Ok(json!({})),
    )
    .unwrap();
    assert_eq!(expanded[0]["props"]["text"], "First");
    assert_ne!(expanded[0]["props"]["key"], expanded[1]["props"]["key"]);
    let bad = json!({"items":[{"id":"a","name":"First"},{"id":"a","name":"Again"}]});
    assert!(
        semantics::expand_structure(
            &p["nodes"],
            &json!({}),
            &bad,
            &json!({}),
            &json!({}),
            &mut |_, _| Ok(json!({}))
        )
        .unwrap_err()
        .contains("Повторный ключ")
    );
}
