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
        .message
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
        .message
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
#[test]
fn formatter_preserves_comments_and_round_trips() {
    use themoretheless_tokenizer_forma::frontend::format_source;

    // line comments before/inside a block, trailing line comment, block comment
    let src = "// Шапка компонента\ncomponent A {\n  // перед Row\n  Row {\n    width: 10px; // trailing\n    /* блочный */\n    height: 20px;\n  }\n}";
    let formatted = format_source(src).unwrap();
    assert_eq!(
        formatted,
        "// Шапка компонента\ncomponent A {\n  // перед Row\n  Row {\n    width: 10px; // trailing\n    /* блочный */\n    height: 20px;\n  }\n}\n"
    );
    // round-trip: parse → format → parse → format is a fixed point
    let again = format_source(&formatted).unwrap();
    assert_eq!(formatted, again);
    assert!(parse(&formatted).is_ok());

    // inline block comment stays inside the output line, multi-line block
    // comment text is preserved verbatim
    let src = "component A { Text { width: /* hint */ 10px; } /* много\n   строк */ }";
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("width: /* hint */ 10px;"));
    assert!(formatted.contains("/* много\n   строк */"));
    assert_eq!(formatted, format_source(&formatted).unwrap());

    // comments inside string literals are not treated as trivia
    let src = "component A { Text { text: 'a // b /* c */'; } }";
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("text: 'a // b /* c */';"));
    assert_eq!(formatted, format_source(&formatted).unwrap());

    // a comment between } and else keeps the else-branch attached
    let src = "component A { if state.a { Text { width: 1px; } } /* mid */ else { Text { width: 2px; } } }";
    let formatted = format_source(src).unwrap();
    assert!(formatted.contains("} /* mid */ else {"));
    assert_eq!(formatted, format_source(&formatted).unwrap());

    // UTF-16 positions of trivia match the source
    let src = "component A { Text { width: 10px; } } // 😀 финал";
    let (_, trivia) = themoretheless_tokenizer_forma::tokenize_with_trivia(src).unwrap();
    assert_eq!(trivia.len(), 1);
    let at = src.find("// 😀 финал").unwrap();
    assert_eq!(trivia[0].start, src[..at].encode_utf16().count());
    assert_eq!(trivia[0].end, src.encode_utf16().count());
}

fn lint_codes(src: &str) -> Vec<(String, String)> {
    use themoretheless_tokenizer_forma::frontend::lint_document;
    let warnings = lint_document(src).unwrap();
    warnings
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["code"].as_str().unwrap().to_string(),
                w["name"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn lint_usage_analysis_follows_ast_references() {
    // prop used only through a call inside a tokens value: not unused
    let src = "component A { prop gap: Number = 8; tokens { spacing: max(props.gap, 4); } Text { width: tokens.spacing; } }";
    assert_eq!(lint_codes(src), []);
    // prop used only through a string interpolation in a state default
    let src = "component A { prop label: String = 'x'; state s: String = 'Label: ${props.label}'; Text { text: s; } }";
    assert_eq!(lint_codes(src), []);
    // state used only through a call in another state's default
    let src =
        "component A { state a: Number = 1; state b: Number = max(state.a, 2); Text { text: b; } }";
    assert_eq!(lint_codes(src), []);
    // prop used only inside a component-level slot
    let src = "component A { prop hint: String = 'h'; slot header { Text { text: hint; } } }";
    assert_eq!(lint_codes(src), []);
    // a plain string literal that merely looks like a path is not a reference
    let src = "component A { prop title: String = 'x'; Text { text: 'props.title'; } }";
    assert_eq!(
        lint_codes(src),
        [("unused-prop".to_string(), "title".to_string())]
    );
    // an escaped interpolation is literal text, not a reference
    let src = "component A { prop name: String = 'x'; Text { text: '\\${name}'; } }";
    assert_eq!(
        lint_codes(src),
        [("unused-prop".to_string(), "name".to_string())]
    );
}

#[test]
fn unused_token_and_i18n_key_lints() {
    use themoretheless_tokenizer_forma::frontend::lint_document;

    let src = "component A {\n  tokens { usedGap: 8px; deadGap: 4px; }\n  tokens color { brand: #fff; dead: #000; }\n  i18n { title: 'T'; deadKey: 'D'; }\n  i18n en { hello: 'H'; bye: 'B'; }\n  i18n ru { hello: 'П'; }\n  Text { width: tokens.usedGap; color: tokens.color.brand; text: i18n.title; }\n  Text { text: i18n.hello; }\n}";
    let warnings = lint_document(src).unwrap();
    let mut got: Vec<(&str, &str)> = warnings
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["code"].as_str().unwrap(), w["name"].as_str().unwrap()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            ("unused-i18n-key", "deadKey"),
            ("unused-i18n-key", "en.bye"),
            ("unused-token", "color.dead"),
            ("unused-token", "deadGap"),
        ]
    );
    // ranges are UTF-16 offsets of the key itself
    let w = warnings
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "deadGap")
        .unwrap();
    let at = src.find("deadGap").unwrap();
    assert_eq!(w["from"], src[..at].encode_utf16().count());
    assert_eq!(w["to"], src[..at].encode_utf16().count() + "deadGap".len());
    assert!(
        w["message"]
            .as_str()
            .unwrap()
            .contains("Неиспользуемый токен tokens deadGap")
    );
    let w = warnings
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["name"] == "en.bye")
        .unwrap();
    let at = src.find("bye").unwrap();
    assert_eq!(w["from"], src[..at].encode_utf16().count());
    assert!(
        w["message"]
            .as_str()
            .unwrap()
            .contains("Неиспользуемый ключ i18n en.bye")
    );

    // group-head reference keeps every key of the group alive
    let src =
        "component A { tokens color { brand: #fff; deep: #111; } Text { color: tokens.color; } }";
    assert_eq!(lint_codes(src), []);
    // bare reference reaches a grouped key (parser mirrors it under the bare name)
    let src = "component A { tokens color { brand: #fff; } Text { color: tokens.brand; } }";
    assert_eq!(lint_codes(src), []);
    // token referenced from a style, style applied: nothing unused
    let src =
        "component A { tokens { brand: #fff; } style s { color: tokens.brand; } Row { apply s; } }";
    assert_eq!(lint_codes(src), []);
    // token referenced from another token: the chain keeps both alive
    let src = "component A { tokens { base: 4px; gap: tokens.base; } Text { width: tokens.gap; } }";
    assert_eq!(lint_codes(src), []);
    // shared module files (no component/design body) expose keys as their API
    let src = "tokens { a: 1px; b: 2px; }\ni18n { hi: 'H'; }\ni18n en { hello: 'H'; }";
    assert_eq!(lint_codes(src), []);
}
