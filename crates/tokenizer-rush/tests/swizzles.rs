use themoretheless_tokenizer_rush::{
    InlineVec, Value, analyze_calls, evaluate, evaluate_bytecode, format_source,
};

fn assert_both(source: &str, expected: Value<'static>) {
    assert!(
        analyze_calls(source).is_valid(),
        "expected valid static analysis for:\n{source}\ngot diagnostics: {:?}",
        analyze_calls(source).diagnostics
    );
    let ast_val =
        evaluate(source, 5_000).unwrap_or_else(|e| panic!("AST failed on:\n{source}\n{e:?}"));
    let bc_val = evaluate_bytecode(source, 5_000)
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
fn vector_read_swizzles_xyzw_and_rgba_match_across_engines() {
    assert_both("vec2(3, 4).x", Value::Number(3.0));
    assert_both("vec2(3, 4).r", Value::Number(3.0));
    assert_both("vec2(3, 4).g", Value::Number(4.0));
    assert_both(
        "vec2(3, 4).yx",
        Value::Vector(InlineVec::from_slice(&[4.0, 3.0])),
    );
    assert_both(
        "vec2(3, 4).gr",
        Value::Vector(InlineVec::from_slice(&[4.0, 3.0])),
    );
    assert_both(
        "vec2(3, 4).xyxy",
        Value::Vector(InlineVec::from_slice(&[3.0, 4.0, 3.0, 4.0])),
    );
    assert_both(
        "vec3(1, 2, 3).bgr",
        Value::Vector(InlineVec::from_slice(&[3.0, 2.0, 1.0])),
    );
    assert_both(
        "vec3(5, 6, 7).rrr",
        Value::Vector(InlineVec::from_slice(&[5.0, 5.0, 5.0])),
    );
    assert_both(
        "vec4(10, 20, 30, 40).wzyx",
        Value::Vector(InlineVec::from_slice(&[40.0, 30.0, 20.0, 10.0])),
    );
    assert_both(
        "vec4(10, 20, 30, 40).abgr",
        Value::Vector(InlineVec::from_slice(&[40.0, 30.0, 20.0, 10.0])),
    );
    assert_both(
        "(vec4(1, 2, 3, 4).wzyx).xz",
        Value::Vector(InlineVec::from_slice(&[4.0, 2.0])),
    );
}

#[test]
fn vector_write_and_compound_swizzles_mutate_in_place_and_match_bytecode() {
    assert_both(
        "let mut v = vec4(1, 2, 3, 4)\nv.xz = vec2(9, 8)\nv",
        Value::Vector(InlineVec::from_slice(&[9.0, 2.0, 8.0, 4.0])),
    );
    assert_both(
        "let mut c = vec4(0.1, 0.2, 0.3, 1.0)\nc.bgr = vec3(1.0, 2.0, 3.0)\nc",
        Value::Vector(InlineVec::from_slice(&[3.0, 2.0, 1.0, 1.0])),
    );
    assert_both(
        "let mut v = vec3(1, 2, 3)\nv.xy += vec2(10, 20)\nv",
        Value::Vector(InlineVec::from_slice(&[11.0, 22.0, 3.0])),
    );
    assert_both(
        "let mut c = vec4(2, 4, 6, 1)\nc.rgb *= 0.5\nc",
        Value::Vector(InlineVec::from_slice(&[1.0, 2.0, 3.0, 1.0])),
    );
    assert_both(
        "struct Particle { pos: vec3 }\nlet mut p = Particle({ pos: vec3(1, 2, 3) })\np.pos.zx = vec2(30, 10)\np.pos",
        Value::Vector(InlineVec::from_slice(&[10.0, 2.0, 30.0])),
    );
    assert_both(
        "let mut state = { items: [vec3(1, 2, 3), vec3(4, 5, 6)] }\nstate.items[1].zy = vec2(60, 50)\nstate.items[1]",
        Value::Vector(InlineVec::from_slice(&[4.0, 50.0, 60.0])),
    );
}

#[test]
fn swizzle_mutation_rolls_back_inside_aborted_try_block() {
    let source = "\
let mut v = vec4(1, 2, 3, 4)
let outcome = try {
    v.xz = vec2(99, 88)
    v.yw += vec2(10, 20)
    None()?
}
(v, outcome)";
    assert!(analyze_calls(source).is_valid());
    let result = evaluate(source, 5_000).unwrap();
    assert_eq!(
        result,
        Value::Tuple(vec![
            Value::Vector(InlineVec::from_slice(&[1.0, 2.0, 3.0, 4.0])),
            Value::Variant("None", vec![]),
        ])
    );
}

#[test]
fn invalid_swizzles_are_rejected_statically_and_at_runtime() {
    // Mixed xyzw and rgba namespaces
    assert_diagnostic("vec4(1, 2, 3, 4).xr", "vector-component");
    assert_diagnostic("vec4(1, 2, 3, 4).rgz", "vector-component");

    // Out-of-bounds lane for vector dimension
    assert_diagnostic("vec2(1, 2).z", "vector-component");
    assert_diagnostic("vec2(1, 2).xyz", "vector-component");
    assert_diagnostic("vec3(1, 2, 3).a", "vector-component");

    // Length > 4
    assert_diagnostic("vec4(1, 2, 3, 4).xyzwx", "vector-component");

    // Duplicate lanes in write swizzles
    assert_diagnostic(
        "let mut v = vec3(1, 2, 3)\nv.xx = vec2(4, 5)",
        "swizzle-assignment",
    );
    assert_diagnostic(
        "let mut v = vec4(1, 2, 3, 4)\nv.rgbr = vec4(1, 2, 3, 4)",
        "swizzle-assignment",
    );

    // Chained lvalue on temporary swizzle vector
    assert_diagnostic("let mut v = vec3(1, 2, 3)\nv.xy.x = 9", "vector-component");

    // Wrong RHS dimension or type
    assert_diagnostic(
        "let mut v = vec4(1, 2, 3, 4)\nv.xyz = vec2(1, 2)",
        "assignment-type",
    );
    assert_diagnostic("let mut v = vec4(1, 2, 3, 4)\nv.xy = 5", "assignment-type");

    // Runtime rejection when static types are untyped / dynamic
    let dyn_dup = "fn f(x) { let mut v = x; v.xx = vec2(1, 2); return v }\nf(vec3(1, 2, 3))";
    assert!(evaluate(dyn_dup, 1_000).is_err());
    assert!(evaluate_bytecode(dyn_dup, 1_000).is_err());

    let dyn_oob = "fn f(x) { return x.z }\nf(vec2(1, 2))";
    assert!(evaluate(dyn_oob, 1_000).is_err());
    assert!(evaluate_bytecode(dyn_oob, 1_000).is_err());
}

#[test]
fn swizzle_expressions_and_assignments_format_idempotently() {
    let source = "let mut c = vec4(1, 2, 3, 4)\nc.bgr = c.rgb * 0.5\nc.xy += vec2(1, 2)\nc.abgr\n";
    let formatted = format_source(source).unwrap();
    assert_eq!(formatted, format_source(&formatted).unwrap());
    assert_both(
        &formatted,
        Value::Vector(InlineVec::from_slice(&[4.0, 0.5, 3.0, 2.5])),
    );
}
