use serde_json::{Value, json};
use themoretheless_tokenizer_forma::{evaluate, parse, parse_expression, semantics};
#[test]
fn original_studio_corpus_preserves_existing_ast_fields_and_editor_offsets() {
    let cases: Value = serde_json::from_str(include_str!("fixtures/forma-corpus.json")).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 87);
    for case in cases.as_array().unwrap() {
        let got = parse(case["source"].as_str().unwrap()).unwrap();
        let mut expected = case["ast"].clone();
        // The 0.8 document schema adds explicit empty declaration collections.
        // Preserve exact comparison of every existing field and source range.
        for (key, value) in [
            ("uses", json!([])),
            ("stateDefinitions", json!({})),
            ("structs", json!({})),
            ("structConstraints", json!({})),
            ("lets", json!({})),
            ("tokens", json!({})),
            ("i18n", json!({})),
            ("styles", json!({})),
            ("slotDefinitions", json!({})),
            ("shortcuts", json!([])),
        ] {
            expected
                .as_object_mut()
                .unwrap()
                .entry(key)
                .or_insert(value);
        }
        assert!(evaluate::equal(&got, &expected), "{}", case["file"]);
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
#[test]
fn extended_pure_functions_evaluate_correctly() {
    let eval = |src: &str| {
        evaluate::evaluate(&parse_expression(src).unwrap(), &mut |_, _| Ok(Value::Null)).unwrap()
    };
    let check = |src: &str, expected: Value| {
        let got = eval(src);
        assert!(
            evaluate::equal(&got, &expected),
            "{src}: got {got}, expected {expected}"
        );
    };
    // Math & interpolation
    check("pow(2, 10)", json!(1024.0));
    check("sqrt(81)", json!(9.0));
    check("sign(-42)", json!(-1));
    check("sign(0)", json!(0));
    check("sign(42)", json!(1));
    check("lerp(10, 20, 0.25)", json!(12.5));
    check("remap(5, 0, 10, 100, 200)", json!(150.0));
    check("avg([10, 20, 30])", json!(20.0));
    check("avg([])", json!(0));

    // Strings
    check("capitalize('fORMA')", json!("Forma"));
    check("'7'.padStart(3, '0')", json!("007"));
    check("'ab'.padEnd(4, '-')", json!("ab--"));
    check("truncate('Hello World', 6)", json!("Hello…"));
    check("truncate('Hello', 10)", json!("Hello"));
    check("repeat('ha', 3)", json!("hahaha"));

    // Collections & records
    check("includes([1, 2, 3], 2)", json!(true));
    check("indexOf(['a', 'b', 'c'], 'b')", json!(1));
    check("indexOf('hello world', 'world')", json!(6));
    check("flatten([[1, 2], [3, [4]], 5])", json!([1, 2, 3, [4], 5]));
    check("chunk([1, 2, 3, 4, 5], 2)", json!([[1, 2], [3, 4], [5]]));
    check("zip(['a', 'b'], [1, 2, 3])", json!([["a", 1], ["b", 2]]));
    check("keys({b: 2, a: 1})", json!(["b", "a"]));
    check("values({a: 10})", json!([10]));
    check(
        "fromEntries(entries({a: 1, b: 2}))",
        json!({"a": 1, "b": 2}),
    );
    check(
        "merge({a: 1, b: 2}, {b: 3, c: 4})",
        json!({"a": 1, "b": 3, "c": 4}),
    );
    check(
        "pick({a: 1, b: 2, c: 3}, ['a', 'c'])",
        json!({"a": 1, "c": 3}),
    );
    check("omit({a: 1, b: 2, c: 3}, ['b'])", json!({"a": 1, "c": 3}));
    check("len({a: 1, b: 2, c: 3})", json!(3));

    // Predicates & null-handling
    check("isEmpty(null)", json!(true));
    check("isEmpty('   ')", json!(true));
    check("isEmpty([])", json!(true));
    check("isEmpty({})", json!(true));
    check("isEmpty([1])", json!(false));
    check("coalesce(null, null, 'fallback', 1 / 0)", json!("fallback"));

    // Color & WCAG accessibility
    let ratio = eval("contrast('#000000', '#ffffff')").as_f64().unwrap();
    assert!((ratio - 21.0).abs() < 1e-6);
    check("invert('#00ff80')", json!("#ff007f"));
    check("desaturate('#ff0000', 1)", json!("#808080"));
    check("saturate('#804040', 0.5)", json!("#a02020"));
}
#[test]
fn extended_types_units_lint_and_formatter_work() {
    use themoretheless_tokenizer_forma::frontend::{format_source, lint_document};

    let src = "component Card {\n  prop spacing: Length = 1.5rem;\n  prop angle: Angle = 45deg;\n  prop metrics: Map<Number> = {a: 1, b: 2};\n  prop id: String | Int = 42;\n  prop forwarded: String = 'ok';\n  prop unusedProp: String = 'unused';\n  let unusedLet = 10;\n  style unusedStyle { color: #fff; }\n  shortcut 'Ctrl+S' -> actions.save();\n  shortcut 'Ctrl+S' -> actions.saveAgain();\n  Row {\n    forward props { forwarded }\n    width: 50vw;\n    height: 100vh;\n    text: '${props.spacing} ${props.angle} ${props.id}';\n  }\n}";
    let doc = parse(src).unwrap();
    assert_eq!(doc["defaults"]["spacing"]["expr"], "1.5rem");
    assert_eq!(doc["defaults"]["angle"]["expr"], "45deg");
    assert!(
        semantics::validate_contract(
            &doc["propDefinitions"],
            &doc["defaults"],
            &doc["enums"],
            "Card"
        )
        .is_ok()
    );
    assert!(semantics::matches_type(
        &json!("hello"),
        "String | Int",
        &json!({})
    ));
    assert!(semantics::matches_type(
        &json!(42),
        "String | Int",
        &json!({})
    ));
    assert!(!semantics::matches_type(
        &json!(true),
        "String | Int",
        &json!({})
    ));
    assert!(semantics::matches_type(
        &json!({"x": "a", "y": "b"}),
        "Record<String, String>",
        &json!({})
    ));

    let warnings = lint_document(src).unwrap();
    let codes: Vec<&str> = warnings
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"unused-prop"));
    assert!(codes.contains(&"unused-let"));
    assert!(codes.contains(&"unused-style"));
    assert!(codes.contains(&"duplicate-shortcut"));
    // `forwarded` was used via `forward props { forwarded }`, so only `unusedProp`, `metrics` are unused props
    assert!(
        !warnings
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["message"].as_str().unwrap().contains("forwarded"))
    );

    let formatted = format_source("component A { Row { width: 10px; } }").unwrap();
    assert!(formatted.contains("component A {\n  Row {\n    width: 10px;\n  }\n}\n"));
}
