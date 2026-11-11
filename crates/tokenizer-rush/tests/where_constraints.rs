use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, ModelProgram, Program, Value, analyze_calls, evaluate,
    evaluate_bytecode, format_source, parse, rename_project_symbol, tokenize,
};

const LIMITS: ExecutionLimits = ExecutionLimits::new(10_000);

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

fn assert_both_err(source: &str, msg_substr: &str) {
    let checked = analyze_calls(source);
    assert!(
        checked.is_valid(),
        "expected valid static analysis before runtime check for:\n{source}\ngot: {:?}",
        checked.diagnostics
    );
    let ast_err =
        evaluate(source, 10_000).expect_err(&format!("expected AST error for:\n{source}"));
    assert!(
        ast_err.message.contains(msg_substr),
        "AST error {:?} did not contain {msg_substr:?}",
        ast_err.message
    );
    let bc_err = evaluate_bytecode(source, 10_000)
        .expect_err(&format!("expected Bytecode error for:\n{source}"));
    assert!(
        bc_err.message.contains(msg_substr),
        "Bytecode error {:?} did not contain {msg_substr:?}",
        bc_err.message
    );
}

#[test]
fn param_where_constraints_validate_defaults_and_host_input_overrides() {
    assert_both(
        "let base = 4\nparam width: number = base where width > 0 && width <= 100\nwidth * 2",
        Value::Number(8.0),
    );
    assert_both_err(
        "param width: number = -3 where width > 0\nwidth",
        "violates where constraint",
    );

    // Host input overrides via Program::instantiate
    let source = "param width: number = 4 where width >= 1 && width <= 10\nlet scaled = width * 3";
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();

    let ok_instance = program
        .instantiate(
            LIMITS,
            &token,
            &[("width", Value::Number(7.0))],
            &[],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(ok_instance.get("scaled"), Some(Value::Number(21.0)));

    let bad_err = match program.instantiate(
        LIMITS,
        &token,
        &[("width", Value::Number(0.0))],
        &[],
        &[],
        &[],
    ) {
        Ok(_) => panic!("expected constraint violation on host input override"),
        Err(err) => err,
    };
    assert!(bad_err.message.contains("violates where constraint"));
}

#[test]
fn model_program_set_parameter_enforces_where_atomically_without_false_cycle() {
    let source = "\
param width: number = 4 where width > 0 && width <= 50
param height: number = 3 where height > 0
node area = width * height
show area";
    let model_prog = ModelProgram::compile(source).unwrap();
    let token = CancellationToken::default();
    let mut model = model_prog.instantiate(LIMITS, &token, &[]).unwrap();
    assert_eq!(model.output(), &Value::Number(12.0));

    // Valid update (including int -> number coercion)
    let updated = model
        .set_parameter("width", Value::Int(10), LIMITS)
        .unwrap();
    assert_eq!(updated, vec!["area"]);
    assert_eq!(model.output(), &Value::Number(30.0));

    // Invalid update rolls back atomically; parameter & derived output stay intact
    let err = model
        .set_parameter("width", Value::Number(-5.0), LIMITS)
        .unwrap_err();
    assert!(err.message.contains("violates where constraint"));
    assert_eq!(model.get("width"), Some(Value::Number(10.0)));
    assert_eq!(model.get("area"), Some(Value::Number(30.0)));
    assert_eq!(model.output(), &Value::Number(30.0));

    // Model remains completely usable afterwards
    model
        .set_parameter("height", Value::Number(5.0), LIMITS)
        .unwrap();
    assert_eq!(model.output(), &Value::Number(50.0));
}

#[test]
fn struct_where_constraints_support_cross_field_and_forward_functions_across_engines() {
    let header = "\
struct SpanRange {
    lo: int,
    hi: int where hi >= lo && is_bounded(hi - lo),
}
fn is_bounded(delta: int) -> bool {
    return delta <= 100
}";

    // Valid construction, functional spread update, and field mutation
    assert_both(
        &format!(
            "{header}\n\
let r0 = SpanRange({{ lo: 10, hi: 20 }})\n\
let mut r1 = {{ ..r0, hi: 50 }}\n\
r1.lo = 25\n\
r1.hi += 10\n\
(r0.hi, r1.lo, r1.hi)"
        ),
        Value::Tuple(vec![Value::Int(20), Value::Int(25), Value::Int(60)]),
    );

    // Construction violation (cross-field & forward helper)
    assert_both_err(
        &format!("{header}\nSpanRange({{ lo: 30, hi: 20 }})"),
        "violates where constraint",
    );
    assert_both_err(
        &format!("{header}\nSpanRange({{ lo: 0, hi: 150 }})"),
        "violates where constraint",
    );

    // Functional spread violation
    assert_both_err(
        &format!(
            "{header}\n\
let r = SpanRange({{ lo: 10, hi: 20 }})\n\
{{ ..r, lo: 25 }}"
        ),
        "violates where constraint",
    );

    // Direct and nested struct field mutation violations
    assert_both_err(
        &format!(
            "{header}\n\
let mut r = SpanRange({{ lo: 10, hi: 20 }})\n\
r.lo = 30"
        ),
        "violates where constraint",
    );
    assert_both_err(
        &format!(
            "{header}\n\
let mut holder = {{ inner: SpanRange({{ lo: 10, hi: 20 }}) }}\n\
holder.inner.hi -= 15"
        ),
        "violates where constraint",
    );
}

#[test]
fn where_remains_contextual_and_records_semantic_keyword_role() {
    let source = "\
fn where(x: int, min_v: int) -> int {
    return if x > min_v { x } else { min_v }
}
struct Boxed {
    where: int where where > 0,
}
param count: int = 5 where count > 0
let b = Boxed({ where: count | where(10) })
b.where";
    let parsed = parse(source);
    assert!(parsed.is_valid(), "diagnostics: {:?}", parsed.diagnostics);
    // Two contextual `where` clauses (one in struct, one in param) get semantic role "keyword",
    // while the other `where` tokens keep their declaration/member/identifier roles.
    let sem = tokenize(source);
    let where_tokens: Vec<_> = sem
        .tokens
        .iter()
        .filter(|t| &source[t.span.start..t.span.end] == "where")
        .collect();
    let kw_count = where_tokens.iter().filter(|t| t.kind == "keyword").count();
    assert_eq!(kw_count, 2, "where tokens: {where_tokens:?}");
    assert!(where_tokens.len() >= 6);
    assert_both(source, Value::Int(10));
}

#[test]
fn static_analysis_and_project_rename_cover_where_clauses() {
    let bad_param = analyze_calls("param x: int = 1 where x + 2\nx");
    assert!(
        bad_param
            .diagnostics
            .iter()
            .any(|d| d.code == "condition-type")
    );

    let bad_struct = analyze_calls("struct S { x: int where x }\n1");
    assert!(
        bad_struct
            .diagnostics
            .iter()
            .any(|d| d.code == "condition-type")
    );

    // Renaming `lo` updates declaration, cross-field `where` clause, constructor, spread, and member read
    let source = "struct Range { lo: int, hi: int where hi >= lo }\nlet r0 = Range({ lo: 1, hi: 5 })\nlet r1 = { ..r0, lo: 2 }\nr1.lo";
    let program = Program::compile(source).unwrap();
    let registry = [("main", &program)];
    let where_lo_offset = source.find("hi >= lo").unwrap() + 6;
    let edits = rename_project_symbol(&registry, "main", where_lo_offset, "min_bound").unwrap();
    assert_eq!(edits.len(), 5);

    let mut renamed = source.to_owned();
    for edit in edits.iter().rev() {
        renamed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(renamed.contains("struct Range { min_bound: int, hi: int where hi >= min_bound }"));
    assert_both(&renamed, Value::Int(2));
}

#[test]
fn failed_struct_field_mutation_rolls_back_mutated_slot_atomically() {
    let source = "\
struct SpanRange {
    lo: int,
    hi: int where hi >= lo,
}
mut r = SpanRange({ lo: 10, hi: 20 })
fn bad_mutate() {
    r.lo = 99
}
fn read_bounds() -> tuple[int, int] {
    return (r.lo, r.hi)
}";
    let program = Program::compile(source).unwrap();
    let token = CancellationToken::default();
    let mut instance = program
        .instantiate(LIMITS, &token, &[], &[], &[], &[])
        .unwrap();
    let err = instance.call("bad_mutate", &[], LIMITS).unwrap_err();
    assert!(err.message.contains("violates where constraint"));
    // `r` must still hold its valid pre-mutation state (10, 20)
    assert_eq!(
        instance.call("read_bounds", &[], LIMITS).unwrap(),
        Value::Tuple(vec![Value::Int(10), Value::Int(20)])
    );
}

#[test]
fn where_constraints_format_idempotently() {
    let source = "struct Material {\n    roughness: f64 where roughness >= 0.0 && roughness <= 1.0,\n    metallic: f64 where metallic >= 0.0 && metallic <= 1.0,\n}\nparam scale: f64 = 1.0 where scale > 0.0\nlet m = Material({ roughness: 0.2, metallic: 0.8 })\nm.roughness * scale\n";
    let formatted = format_source(source).unwrap();
    assert_eq!(formatted, format_source(&formatted).unwrap());
    assert_both(&formatted, Value::Number(0.2));
}
