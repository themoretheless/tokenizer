use themoretheless_tokenizer_rush::{Value, evaluate, json_stringify};

#[test]
fn test_json_parse_primitives() {
    let script = "let n = match json_parse('123.5') { Ok(v) => v, Err(e) => 0 }; \
                  let s = match json_parse('\"hello\\\\nworld\"') { Ok(v) => v, Err(e) => '' }; \
                  let b = match json_parse('true') { Ok(v) => v, Err(e) => false }; \
                  let null_val = match json_parse('null') { Ok(v) => v, Err(e) => false }; \
                  (n, s, b, null_val)";
    let result = evaluate(script, 1000).expect("evaluated");

    match result {
        Value::Tuple(items) => {
            assert_eq!(items.len(), 4);
            assert_eq!(items[0], Value::Number(123.5));
            assert_eq!(items[1], Value::String("hello\nworld".into()));
            assert_eq!(items[2], Value::Bool(true));
            assert_eq!(items[3], Value::Null);
        }
        other => panic!("expected tuple, got {other:?}"),
    }
}

#[test]
fn test_json_parse_collections_and_try() {
    let script = "fn get_name() -> Result[str, str] {\n\
    let data = json_parse('{\"users\": [{\"name\": \"Alice\", \"score\": 100}]}')?\n\
    return Ok(data.users[0].name)\n\
}\n\
get_name()";
    let result = evaluate(script, 1000).expect("evaluated");
    assert_eq!(
        result,
        Value::Variant("Ok", vec![Value::String("Alice".into())])
    );
}

#[test]
fn test_json_parse_error_handling() {
    let script = "let res = json_parse('invalid json here {'); \
                  match res { Ok(v) => 'unexpected ok', Err(e) => 'caught error' }";
    let result = evaluate(script, 1000).expect("evaluated");
    assert_eq!(result, Value::String("caught error".into()));
}

#[test]
fn test_json_stringify_primitives_and_collections() {
    let script = "let user = { name: 'Alice \"Ace\"', score: 42, active: true }; \
                  json_stringify(user)";
    let result = evaluate(script, 1000).expect("evaluated");
    assert_eq!(
        result,
        Value::String("{\"active\":true,\"name\":\"Alice \\\"Ace\\\"\",\"score\":42}".into())
    );
}

#[test]
fn test_json_roundtrip_and_mutation() {
    let script = "fn mutate_and_dump() -> Result[str, str] {\n\
    let json_text = '{\"items\":[1,2,3],\"settings\":{\"enabled\":false}}'\n\
    mut obj = json_parse(json_text)?\n\
    obj.items[1] = 42\n\
    obj.settings.enabled = true\n\
    return Ok(json_stringify(obj))\n\
}\n\
mutate_and_dump()";
    let result = evaluate(script, 1000).expect("evaluated");
    assert_eq!(
        result,
        Value::Variant(
            "Ok",
            vec![Value::String(
                "{\"items\":[1,42,3],\"settings\":{\"enabled\":true}}".into()
            )]
        )
    );
}

#[test]
fn test_json_stringify_error_on_non_serializable() {
    let script = "let bad = x => x; json_stringify(bad)";
    let err = evaluate(script, 1000).expect_err("should fail serialization");
    assert!(err.message.contains("cannot be serialized to JSON"));
}

#[test]
fn test_json_stringify_non_finite_rust() {
    let nan_val = Value::Number(f64::NAN);
    let err = json_stringify(&nan_val).expect_err("should fail on NaN");
    assert!(err.contains("Non-finite number"));

    let inf_val = Value::Number(f64::INFINITY);
    let err = json_stringify(&inf_val).expect_err("should fail on Infinity");
    assert!(err.contains("Non-finite number"));
}
