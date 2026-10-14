use themoretheless_tokenizer_rush::{Value, evaluate, format_source};

fn run(source: &str) -> Value<'_> {
    evaluate(source, 10000).expect("Evaluation failed")
}

#[test]
fn test_basic_interpolation() {
    let result = run(r#"
let name = "Rush"
f"Hello, {name}!"
"#);
    assert_eq!(result, Value::String("Hello, Rush!".to_string()));
}

#[test]
fn test_multiple_expressions_and_arithmetic() {
    let result = run(r#"
let a = 15
let b = 27
f"{a} + {b} = {a + b}"
"#);
    assert_eq!(result, Value::String("15 + 27 = 42".to_string()));
}

#[test]
fn test_escaped_braces() {
    let result = run(r#"
let val = 100
f"Literal {{open}} and {{close}} with {val}"
"#);
    assert_eq!(
        result,
        Value::String("Literal {open} and {close} with 100".to_string())
    );
}

#[test]
fn test_nested_calls_and_pipelines() {
    let result = run(r#"
let xs = [1, 2, 3, 4]
f"Count is {len(xs)} and doubled is {xs | map(x => x * 2)}"
"#);
    assert_eq!(
        result,
        Value::String("Count is 4 and doubled is [2, 4, 6, 8]".to_string())
    );
}

#[test]
fn test_record_and_boolean_interpolation() {
    let result = run(r#"
let user = { name: "Alice", active: true }
f"User {user.name} is active: {user.active}"
"#);
    assert_eq!(
        result,
        Value::String("User Alice is active: true".to_string())
    );
}

#[test]
fn test_interpolation_quotes_and_escapes() {
    let result = run(r#"
let item = "widget"
f"Item: \"{item}\"\nNext line"
"#);
    assert_eq!(
        result,
        Value::String("Item: \"widget\"\nNext line".to_string())
    );
}

#[test]
fn test_empty_string_interpolation() {
    let result = run(r#"f"""#);
    assert_eq!(result, Value::String("".to_string()));

    let result2 = run(r#"f"constant only without expr""#);
    assert_eq!(
        result2,
        Value::String("constant only without expr".to_string())
    );
}

#[test]
fn test_format_source_preserves_interpolation() {
    let code = "let x = 10\nlet msg = f\"Value: {x + 1} and {{raw}}\"\n";
    let formatted = format_source(code).expect("Format failed");
    assert!(formatted.contains("f\"Value: {x + 1} and {{raw}}\""));
}
