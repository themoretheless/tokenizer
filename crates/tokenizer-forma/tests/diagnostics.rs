//! Structured diagnostics (T0.4), Record<K, V> key check (T0.2) and
//! pattern-matcher degradation (T0.3).
use serde_json::{Value, json};
use themoretheless_tokenizer_forma::{Diagnostic, evaluate, parse, parse_expression, semantics};

fn eval_err(src: &str) -> Diagnostic {
    evaluate::evaluate(&parse_expression(src).unwrap(), &mut |_, _| Ok(Value::Null)).unwrap_err()
}

fn eval_unknown_function_err() -> Diagnostic {
    // The parser rejects unknown function names early, so build the AST directly.
    let ast = json!({"expression": {"kind": "call", "name": "nosuchfn", "args": [1]}});
    evaluate::evaluate(&ast, &mut |_, _| Ok(Value::Null)).unwrap_err()
}

#[test]
fn record_key_must_be_string() {
    // K != String is rejected with a coded diagnostic
    let err = semantics::validate_type_name("Record<Int, Number>", &json!({})).unwrap_err();
    assert_eq!(err.code, "unsupported-record-key");
    assert!(err.message.contains("Ключи Record"));
    // K = String works as before
    assert!(semantics::validate_type_name("Record<String, Int>", &json!({})).is_ok());
    // nested and union forms are still validated recursively
    assert!(
        semantics::validate_type_name("Record<String, List<Record<String, Int>>>", &json!({}))
            .is_ok()
    );
    assert_eq!(
        semantics::validate_type_name("Record<String, Record<Int, Number>>", &json!({}))
            .unwrap_err()
            .code,
        "unsupported-record-key"
    );
    assert!(semantics::validate_type_name("Record<Int, Number> | String", &json!({})).is_err());
    assert!(semantics::validate_type_name("Record<String, Int> | String", &json!({})).is_ok());
    // malformed Record arity is still an unknown-type error
    assert_eq!(
        semantics::validate_type_name("Record<String>", &json!({}))
            .unwrap_err()
            .code,
        "unknown-type"
    );
    // matches_type behaviour is unchanged: objects are matched by values
    assert!(semantics::matches_type(
        &json!({"a": 1}),
        "Record<String, Int>",
        &json!({})
    ));
    // contracts surface the same code
    let defs = json!({"m": {"type": "Record<Int, Number>"}});
    let err = semantics::validate_contract(&defs, &json!({}), &json!({}), "A").unwrap_err();
    assert_eq!(err.code, "unsupported-record-key");
}

#[test]
fn catastrophic_pattern_reports_code_instead_of_false() {
    let pat = "^a*a*a*a*a*a*a*a*b$";
    let s = "a".repeat(48);
    let err = evaluate::matches_constraint_pattern(&s, pat).unwrap_err();
    assert_eq!(err.code, "pattern-too-complex");
    // the same code propagates through contract constraints
    let defs = json!({"code": {"type": "String", "constraints": {"pattern": pat}}});
    let err =
        semantics::validate_contract(&defs, &json!({"code": s}), &json!({}), "A").unwrap_err();
    assert_eq!(err.code, "pattern-too-complex");
    // and through the isValid pure function
    let err = eval_err(&format!("isValid('{s}', '{pat}')"));
    assert_eq!(err.code, "pattern-too-complex");
    // supported patterns keep working
    assert!(evaluate::matches_constraint_pattern("abc123", "^[a-z]+\\d+$").unwrap());
    assert!(evaluate::matches_constraint_pattern("x@y.z", "email").unwrap());
    assert!(!evaluate::matches_constraint_pattern("nope", "digits").unwrap());
    // a pattern mismatch (not complexity) keeps its own code
    let defs = json!({"code": {"type": "String", "constraints": {"pattern": "^\\d+$"}}});
    let err =
        semantics::validate_contract(&defs, &json!({"code": "ab"}), &json!({}), "A").unwrap_err();
    assert_eq!(err.code, "pattern-mismatch");
}

#[test]
fn diagnostics_carry_codes_and_spans() {
    // evaluate: unknown pure function
    let err = eval_unknown_function_err();
    assert_eq!(err.code, "unknown-function");
    assert!(err.message.contains("nosuchfn"));
    // semantics: contract type mismatch and unknown type
    let defs = json!({"title": {"type": "String", "required": true}});
    let err =
        semantics::validate_contract(&defs, &json!({"title": 5}), &json!({}), "A").unwrap_err();
    assert_eq!(err.code, "type-mismatch");
    assert_eq!(
        semantics::validate_type_name("Nope", &json!({}))
            .unwrap_err()
            .code,
        "unknown-type"
    );
    // cyclic props reference keeps its message and gains a code
    let props = json!({"a":{"expr":"props.b"},"b":{"expr":"props.a"}});
    let err = semantics::evaluate_value(
        &json!({"expr":"props.a"}),
        &props,
        &json!({}),
        &[],
        false,
        &json!({}),
    )
    .unwrap_err();
    assert_eq!(err.code, "cyclic-reference");
    // expansion errors carry the UTF-16 span of the offending node
    let p =
        parse("component A { for item in state.items key item.id { Text { text: item.name; } } }")
            .unwrap();
    let bad = json!({"items":[{"id":"a","name":"First"},{"id":"a","name":"Again"}]});
    let err = semantics::expand_structure(
        &p["nodes"],
        &json!({}),
        &bad,
        &json!({}),
        &json!({}),
        &mut |_, _| Ok(json!({})),
    )
    .unwrap_err();
    assert_eq!(err.code, "duplicate-key");
    assert!(err.span.is_some());
    // JSON form: message preserved, code/span additive
    let j = err.to_json();
    assert_eq!(j["code"], "duplicate-key");
    assert!(j["message"].as_str().unwrap().contains("Повторный ключ"));
    assert_eq!(j["span"], err.span.unwrap() as u64);
    let no_span = eval_unknown_function_err().to_json();
    assert!(no_span.get("span").is_none());
    assert_eq!(no_span["code"], "unknown-function");
}
