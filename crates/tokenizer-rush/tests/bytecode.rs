use themoretheless_tokenizer_rush::{BytecodeProgram, Value, evaluate, evaluate_bytecode};

#[test]
fn test_bytecode_arithmetic_and_precedence() {
    let script = "10 + 20 * 3 - 5 / 2";
    let ast_res = evaluate(script, 1000).expect("ast eval");
    let vm_res = evaluate_bytecode(script, 1000).expect("vm eval");
    assert_eq!(ast_res, vm_res);
    assert_eq!(vm_res, Value::Number(67.5));
}

#[test]
fn test_bytecode_variables_and_mutations() {
    let script = r#"
mut x = 10
x += 5
x *= 2
x -= 4
x
"#;
    let vm_res = evaluate_bytecode(script, 1000).expect("vm eval");
    assert_eq!(vm_res, Value::Number(26.0));
}

#[test]
fn test_bytecode_if_else_branching() {
    let script1 = "let a = 15; if a > 10 { a * 2 } else { a / 2 }";
    assert_eq!(
        evaluate_bytecode(script1, 1000).expect("vm"),
        Value::Number(30.0)
    );

    let script2 = "let a = 5; if a > 10 { a * 2 } else { a * 3 }";
    assert_eq!(
        evaluate_bytecode(script2, 1000).expect("vm"),
        Value::Number(15.0)
    );
}

#[test]
fn test_bytecode_while_loop_with_accumulator() {
    let script = r#"
mut sum = 0
mut i = 1
while i <= 10 {
    sum += i
    i += 1
}
sum
"#;
    assert_eq!(
        evaluate_bytecode(script, 1000).expect("vm"),
        Value::Number(55.0)
    );
}

#[test]
fn test_bytecode_loop_break_and_continue() {
    let script = r#"
mut sum = 0
mut i = 0
while i < 10 {
    i += 1
    if i == 5 {
        continue
    }
    if i == 8 {
        break
    }
    sum += i
}
sum
"#;
    // i: 1 + 2 + 3 + 4 + (skip 5) + 6 + 7 = 23
    assert_eq!(
        evaluate_bytecode(script, 1000).expect("vm"),
        Value::Number(23.0)
    );
}

#[test]
fn test_bytecode_string_interpolation_and_builtins() {
    let script = r#"
let lang = "Rush"
let ver = 2
f"Hello from {lang} v{ver}!"
"#;
    assert_eq!(
        evaluate_bytecode(script, 1000).expect("vm"),
        Value::String("Hello from Rush v2!".to_string())
    );
}

#[test]
fn test_bytecode_pipelines_and_string_utilities() {
    let script = r#"
"  alpha, beta, gamma  "
    | trim
    | split(", ")
    | join(" :: ")
"#;
    assert_eq!(
        evaluate_bytecode(script, 1000).expect("vm"),
        Value::String("alpha :: beta :: gamma".to_string())
    );
}

#[test]
fn test_bytecode_collections_and_mutations() {
    let script = r#"
mut list = [10, 20, 30]
list[1] = 99

mut rec = { x: 1, y: 2 }
rec.x = 42

list[1] + rec.x
"#;
    assert_eq!(
        evaluate_bytecode(script, 1000).expect("vm"),
        Value::Number(141.0)
    );
}

#[test]
fn test_bytecode_json_builtins() {
    let script = r#"
let obj = { val: 123 }
let encoded = json_stringify(obj)
let parsed = json_parse(encoded)
(encoded, parsed)
"#;
    let res = evaluate_bytecode(script, 1000).expect("vm");
    match res {
        Value::Tuple(items) => {
            assert_eq!(items[0], Value::String("{\"val\":123}".to_string()));
            let mut expected_record = std::collections::BTreeMap::new();
            expected_record.insert("val".to_string(), Value::Number(123.0));
            assert_eq!(
                items[1],
                Value::Variant("Ok", vec![Value::Record(expected_record)])
            );
        }
        other => panic!("Expected tuple, got {other:?}"),
    }
}

#[test]
fn test_bytecode_fuel_limit_enforcement() {
    let infinite_loop = "while true {}";
    let program = BytecodeProgram::compile(infinite_loop).expect("compiled");
    let err = program.execute(50).expect_err("should exhaust fuel");
    assert!(err.message.contains("limit exceeded") || err.message.contains("Execution limit"));
}

#[test]
fn test_bytecode_differential_with_ast() {
    let cases = [
        "1 + 2 * 3",
        "(10 - 2) * (3 + 1)",
        "true && false || true",
        "!false && (5 > 3)",
        "[1, 2, 3][1]",
        "{ a: 10, b: 20 }.b",
        "len([1, 2, 3, 4, 5])",
        "trim('  abc  ')",
        "to_upper('hello')",
        "contains('hello world', 'world')",
        "contains([1, 2, 3], 2)",
        "replace('banana', 'a', 'o')",
    ];

    for case in cases {
        let ast = evaluate(case, 1000).unwrap_or_else(|e| panic!("AST failed on '{case}': {e:?}"));
        let vm = evaluate_bytecode(case, 1000)
            .unwrap_or_else(|e| panic!("VM failed on '{case}': {e:?}"));
        assert_eq!(ast, vm, "Differential mismatch on: {case}");
    }
}
