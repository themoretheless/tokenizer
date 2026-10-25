use themoretheless_tokenizer_rush::{BytecodeProgram, ExecutionLimits, Value, evaluate_bytecode};

fn list_len(value: Value<'static>) -> usize {
    match value {
        Value::List(items) => items.len(),
        other => panic!("Expected list, got {other:?}"),
    }
}

fn run_with(source: &str, heap: usize, allocated: usize) -> Result<Value<'static>, String> {
    BytecodeProgram::compile(source)
        .unwrap()
        .execute_with_limits(
            ExecutionLimits {
                max_heap_bytes: heap,
                max_allocated_bytes: allocated,
                ..ExecutionLimits::new(1_000_000)
            },
            None,
        )
        .map_err(|e| e.message)
}

#[test]
fn eager_range_obeys_the_heap_limit() {
    // range(0,1000) pre-reserves exactly 1000 Value slots.
    let slots = 1000 * std::mem::size_of::<Value>();
    assert_eq!(
        run_with("range(0,1000)", slots - 1, usize::MAX).unwrap_err(),
        "Heap limit exceeded"
    );
    assert_eq!(
        list_len(run_with("range(0,1000)", slots, usize::MAX).unwrap()),
        1000
    );
}

#[test]
fn eager_collect_obeys_the_heap_limit() {
    assert_eq!(
        run_with("range_iter(0,100000) | collect(100000)", 4096, usize::MAX).unwrap_err(),
        "Heap limit exceeded"
    );
    assert_eq!(
        list_len(run_with("range_iter(0,100) | collect(100)", usize::MAX, usize::MAX).unwrap()),
        100
    );
}

#[test]
fn quadratic_concatenation_hits_the_cumulative_limit() {
    let source = r#"
mut s = ""
mut i = 0
while i < 200 {
    s += "x"
    i += 1
}
len(s)
"#;
    assert_eq!(
        run_with(source, usize::MAX, 1000).unwrap_err(),
        "Allocation limit exceeded"
    );
    assert_eq!(
        run_with(source, usize::MAX, usize::MAX).unwrap(),
        Value::Number(200.0)
    );
}

#[test]
fn list_literal_and_interpolation_charge_the_work_budget() {
    let slots = 3 * std::mem::size_of::<Value>();
    assert_eq!(
        run_with("[1,2,3]", usize::MAX, slots - 1).unwrap_err(),
        "Allocation limit exceeded"
    );
    assert_eq!(list_len(run_with("[1,2,3]", usize::MAX, slots).unwrap()), 3);
    assert_eq!(
        run_with("let a = \"ab\"; f\"{a}{a}\"", usize::MAX, 3).unwrap_err(),
        "Allocation limit exceeded"
    );
    assert_eq!(
        run_with("let a = \"ab\"; f\"{a}{a}\"", usize::MAX, 4).unwrap(),
        Value::String("abab".into())
    );
}

#[test]
fn a_compiled_program_stays_reusable_after_a_limit_error() {
    let program = BytecodeProgram::compile("range(0,1000)").unwrap();
    let tight = ExecutionLimits {
        max_heap_bytes: 16,
        ..ExecutionLimits::new(1_000_000)
    };
    assert_eq!(
        program
            .execute_with_limits(tight, None)
            .unwrap_err()
            .message,
        "Heap limit exceeded"
    );
    assert_eq!(
        list_len(
            program
                .execute_with_limits(ExecutionLimits::new(1_000_000), None)
                .unwrap()
        ),
        1000
    );
}

#[test]
fn default_constructor_keeps_memory_limits_off() {
    // Vm::new via evaluate_bytecode: no heap or work budget applies.
    assert_eq!(
        list_len(evaluate_bytecode("range(0,100000)", 200_000).unwrap()),
        100_000
    );
}
