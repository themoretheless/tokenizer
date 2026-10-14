use themoretheless_tokenizer_rush::{Value, evaluate};

fn run(source: &str) -> Value<'_> {
    evaluate(source, 10000).expect("Evaluation failed")
}

#[test]
fn test_trim_functions() {
    assert_eq!(
        run(r#"trim("  hello world  \n\t")"#),
        Value::String("hello world".into())
    );
    assert_eq!(
        run(r#"trim_start("   rush")"#),
        Value::String("rush".into())
    );
    assert_eq!(run(r#"trim_end("rush   ")"#), Value::String("rush".into()));
    // In pipeline
    assert_eq!(run(r#""  piped  " | trim"#), Value::String("piped".into()));
}

#[test]
fn test_case_conversions() {
    assert_eq!(
        run(r#"to_lower("RUSH Language")"#),
        Value::String("rush language".into())
    );
    assert_eq!(
        run(r#"to_upper("rush language")"#),
        Value::String("RUSH LANGUAGE".into())
    );
    assert_eq!(run(r#""test" | to_upper"#), Value::String("TEST".into()));
}

#[test]
fn test_prefix_suffix_checks() {
    assert_eq!(
        run(r#"starts_with("filename.rush", "file")"#),
        Value::Bool(true)
    );
    assert_eq!(
        run(r#"starts_with("filename.rush", "other")"#),
        Value::Bool(false)
    );
    assert_eq!(
        run(r#"ends_with("filename.rush", ".rush")"#),
        Value::Bool(true)
    );
    assert_eq!(
        run(r#"ends_with("filename.rush", ".txt")"#),
        Value::Bool(false)
    );
    // In pipeline
    assert_eq!(
        run(r#""cargo.toml" | ends_with(".toml")"#),
        Value::Bool(true)
    );
}

#[test]
fn test_contains() {
    // String needle in string
    assert_eq!(
        run(r#"contains("supercollider", "collider")"#),
        Value::Bool(true)
    );
    assert_eq!(
        run(r#"contains("supercollider", "rust")"#),
        Value::Bool(false)
    );
    // Value in list
    assert_eq!(run(r#"contains([10, 20, 30], 20)"#), Value::Bool(true));
    assert_eq!(run(r#"contains([10, 20, 30], 42)"#), Value::Bool(false));
    // Key in record
    assert_eq!(
        run(r#"contains({ name: "Rush", ver: 1 }, "name")"#),
        Value::Bool(true)
    );
    assert_eq!(
        run(r#"contains({ name: "Rush", ver: 1 }, "missing")"#),
        Value::Bool(false)
    );
}

#[test]
fn test_replace() {
    assert_eq!(
        run(r#"replace("banana", "a", "o")"#),
        Value::String("bonono".into())
    );
    assert_eq!(
        run(r#""foo_bar_baz" | replace("_", "-")"#),
        Value::String("foo-bar-baz".into())
    );
}

#[test]
fn test_split_and_join() {
    assert_eq!(
        run(r#"split("alpha,beta,gamma", ",")"#),
        Value::List(vec![
            Value::String("alpha".into()),
            Value::String("beta".into()),
            Value::String("gamma".into()),
        ])
    );
    assert_eq!(
        run(r#"split("abc", "")"#),
        Value::List(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])
    );
    assert_eq!(
        run(r#"join(["red", "green", "blue"], " / ")"#),
        Value::String("red / green / blue".into())
    );
    assert_eq!(
        run(r#"join([1, 2, 3], ", ")"#),
        Value::String("1, 2, 3".into())
    );
}

#[test]
fn test_complex_pipeline_chaining() {
    let script = r#"
"  one, two, three  "
    | trim
    | split(", ")
    | map(s => s | to_upper)
    | join(" - ")
"#;
    assert_eq!(run(script), Value::String("ONE - TWO - THREE".into()));
}

#[test]
fn test_builtin_type_errors() {
    assert!(evaluate("trim(123)", 1000).is_err());
    assert!(evaluate("split(123, ',')", 1000).is_err());
    assert!(evaluate("join('not a list', ',')", 1000).is_err());
    assert!(evaluate("replace('abc', 1, 2)", 1000).is_err());
}
