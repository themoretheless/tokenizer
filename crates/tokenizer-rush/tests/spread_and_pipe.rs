use themoretheless_tokenizer_rush::{
    Program, Value, analyze_calls, evaluate, evaluate_bytecode, format_source,
    rename_project_symbol,
};

fn assert_both(source: &str, expected: Value<'static>) {
    let checked = analyze_calls(source);
    assert!(
        checked.is_valid(),
        "expected valid static analysis for:\n{source}\ngot diagnostics: {:?}",
        checked.diagnostics
    );
    let ast_val =
        evaluate(source, 10_000).unwrap_or_else(|e| panic!("AST failed on:\n{source}\n{e:?}"));
    let bc_val = evaluate_bytecode(source, 10_000)
        .unwrap_or_else(|e| panic!("Bytecode failed on:\n{source}\n{e:?}"));
    assert_eq!(ast_val, expected, "AST mismatch for:\n{source}");
    assert_eq!(bc_val, expected, "Bytecode mismatch for:\n{source}");
}

fn assert_diagnostic(source: &str, code: &str) {
    let parsed = analyze_calls(source);
    assert!(
        !parsed.is_valid(),
        "expected diagnostic `{code}` for:\n{source}"
    );
    assert!(
        parsed.diagnostics.iter().any(|d| d.code == code),
        "expected diagnostic `{code}`, got {:?} for:\n{source}",
        parsed.diagnostics
    );
}

#[test]
fn record_spread_updates_merge_fields_left_to_right_without_mutating_base() {
    assert_both(
        "\
let base = { a: 1, b: 2, c: 3 }
let updated = { ..base, b: 20, d: 40 }
(base.b, updated.a, updated.b, updated.c, updated.d)",
        Value::Tuple(vec![
            Value::Number(2.0),
            Value::Number(1.0),
            Value::Number(20.0),
            Value::Number(3.0),
            Value::Number(40.0),
        ]),
    );

    // Order sensitivity: entries before spread can be overridden by spread, entries after win
    assert_both(
        "\
let r1 = { a: 10, b: 20 }
let r2 = { b: 200, c: 300 }
let merged = { a: 1, ..r1, ..r2, c: 3000 }
(merged.a, merged.b, merged.c)",
        Value::Tuple(vec![
            Value::Number(10.0),
            Value::Number(200.0),
            Value::Number(3000.0),
        ]),
    );
}

#[test]
fn large_record_update_over_64_entries_batches_cleanly_in_bytecode() {
    let mut fields = String::new();
    for i in 0..70 {
        if i > 0 {
            fields.push_str(", ");
        }
        fields.push_str(&format!("k{i}: {i}"));
    }
    let source = format!(
        "let base = {{ k0: -1, k69: -1 }}\nlet big = {{ ..base, {fields} }}\nbig.k0 + big.k69"
    );
    assert_both(&source, Value::Number(69.0));
}

#[test]
fn nominal_struct_spread_preserves_identity_coerces_types_and_supports_constructor_wrapping() {
    assert_both(
        "\
struct Point { x: f64, y: f64 }
fn length_sq(p: Point) -> f64 {
    return p.x * p.x + p.y * p.y
}
let p0 = Point({ x: 3, y: 4 })
let p1 = { ..p0, x: 6 }
let p2 = Point({ ..p1, y: 8 })
(p0.x, length_sq(p1), length_sq(p2))",
        Value::Tuple(vec![
            Value::Number(3.0),
            Value::Number(52.0),
            Value::Number(100.0),
        ]),
    );
}

#[test]
fn spread_diagnostics_catch_unknown_struct_fields_bad_types_and_non_record_spreads() {
    assert_diagnostic(
        "struct Point { x: f64, y: f64 }\nlet p = Point({ x: 1, y: 2 })\n{ ..p, z: 3 }",
        "unknown-record-field",
    );
    assert_diagnostic(
        "struct Point { x: f64, y: f64 }\nlet p = Point({ x: 1, y: 2 })\n{ ..p, x: 'bad' }",
        "struct-field-type",
    );
    assert_diagnostic("let x = 42\n{ ..x, a: 1 }", "spread-type");

    // Runtime rejection when untyped
    let dyn_bad = "fn f(x) { return { ..x, a: 1 } }\nf(42)";
    assert!(evaluate(dyn_bad, 1_000).is_err());
    assert!(evaluate_bytecode(dyn_bad, 1_000).is_err());

    let dyn_struct_unknown =
        "struct Point { x: f64 }\nfn f(p) { return { ..p, y: 2 } }\nf(Point({ x: 1 }))";
    assert!(evaluate(dyn_struct_unknown, 1_000).is_err());
    assert!(evaluate_bytecode(dyn_struct_unknown, 1_000).is_err());
}

#[test]
fn rename_project_symbol_updates_struct_fields_inside_functional_spread_updates() {
    let source = "struct Settings { speed: number, scale: number }\nlet s0 = Settings({ speed: 2, scale: 1 })\nlet s1 = { ..s0, speed: 5 }\ns1.speed";
    let program = Program::compile(source).unwrap();
    let registry = [("main", &program)];
    let spread_field_offset = source.find("speed: 5").unwrap();
    let edits = rename_project_symbol(&registry, "main", spread_field_offset, "velocity").unwrap();
    // Declaration + constructor literal + spread update + member access = 4 edits
    assert_eq!(edits.len(), 4);

    let mut updated = source.to_owned();
    for edit in edits.iter().rev() {
        updated.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(updated.contains("struct Settings { velocity: number"));
    assert!(updated.contains("Settings({ velocity: 2"));
    assert!(updated.contains("{ ..s0, velocity: 5 }"));
    assert!(updated.contains("s1.velocity"));
    assert_both(&updated, Value::Number(5.0));
}

#[test]
fn pipeline_placeholder_routes_topic_to_explicit_slots_and_evaluates_once() {
    // Route piped value into the second argument slot
    assert_both(
        "fn sub(a: int, b: int) -> int { return a - b }\n3 | sub(10, _)",
        Value::Int(7),
    );
    // Multiple placeholders in one call: dot(v, v)
    assert_both("vec2(3, 4) | dot(_, _)", Value::Number(25.0));

    // Mixing `_` placeholder stages and prepend-at-0 stages
    assert_both(
        "\
fn between(lo: number, x: number, hi: number) -> number {
    return clamp(x, lo, hi)
}
fn scale(x: number, k: number) -> number {
    return x * k
}
-5.0 | between(0.0, _, 10.0) | clamp(2.0, 100.0) | scale(3.0)",
        Value::Number(6.0),
    );

    // Side-effectful input expression is evaluated ONCE even when `_` appears 3 times
    assert_both(
        "\
let mut calls: int = 0
fn next_val() -> int {
    calls += 1
    return 4
}
fn sum3(a: int, b: int, c: int) -> int {
    return a + b + c
}
let total = next_val() | sum3(_, _, _)
(calls, total)",
        Value::Tuple(vec![Value::Int(1), Value::Int(12)]),
    );
}

#[test]
fn pipeline_placeholder_supports_parameter_inference_and_static_diagnostics() {
    // Parameter `x` inferred from `between(0.0, _, 1.0)`
    let source = "\
fn between(lo: f64, val: f64, hi: f64) -> f64 {
    return clamp(val, lo, hi)
}
fn saturate(x) {
    return x | between(0.0, _, 1.0)
}
saturate(2.5)";
    assert_both(source, Value::Number(1.0));
    assert_diagnostic(
        &format!("{source}\nsaturate('not-a-number')"),
        "argument-type",
    );

    // Arity mismatch with placeholder
    assert_diagnostic("3 | clamp(5, _)", "argument-count");
    // Type mismatch in placeholder slot
    assert_diagnostic("'bad' | lerp(0, 10, _)", "argument-type");
}

#[test]
fn spread_and_pipeline_placeholder_format_idempotently() {
    let source = "let r = { ..{ a: 1, b: 2 }, b: 3 }\nr.b | clamp(10, 0, _)\n";
    let formatted = format_source(source).unwrap();
    assert_eq!(formatted, format_source(&formatted).unwrap());
    assert_both(&formatted, Value::Number(3.0));
}
