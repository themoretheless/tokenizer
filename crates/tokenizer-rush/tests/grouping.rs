use themoretheless_tokenizer_rush::{CancellationToken, ExecutionLimits, Program, evaluate};

#[test]
fn groups_preserve_first_key_order_item_order_and_callback_count() {
    evaluate(
        r#"fn classify(x) { return if x % 2 == 0 { "even" } else { "odd" } }
let result = [3,2,1,4] | group_by(classify)
assert(len(result) == 2)
assert(result[0].key == "odd")
assert(result[0].values == [3,1])
assert(result[1].key == "even")
assert(result[1].values == [2,4])
assert(group_by([], classify) == [])
let streamed = range_iter(0,4) | group_by(classify)
assert(streamed[0].values == [0,2])"#,
        10000,
    )
    .unwrap();
    evaluate(
        r#"mut calls = 0
fn key(x) { calls += 1; return "all" }
group_by([1,2,3], key)
assert(calls == 3)"#,
        10000,
    )
    .unwrap();
}

#[test]
fn groups_enforce_key_types_callbacks_and_limits() {
    for source in [
        "group_by([], 1)",
        "group_by([1], x => x)",
        "group_by(1, x => 'a')",
    ] {
        assert!(evaluate(source, 1000).is_err(), "{source}");
    }
    let program = Program::compile("range_iter(0,4) | group_by(x => 'all')").unwrap();
    let limits = ExecutionLimits {
        max_collection_items: 3,
        ..ExecutionLimits::new(10000)
    };
    assert!(
        program
            .run_with_limits(limits, &CancellationToken::default(), &[], &[], &[])
            .is_err()
    );
    let program = Program::compile("group_by([1], x => 'all')").unwrap();
    let limits = ExecutionLimits {
        max_string_bytes: 5,
        ..ExecutionLimits::new(10000)
    };
    assert!(
        program
            .run_with_limits(limits, &CancellationToken::default(), &[], &[], &[])
            .is_err()
    );
}

#[test]
fn fold_by_keeps_separate_accumulators_in_first_key_order() {
    evaluate(r#"let groups = range_iter(1,6) | fold_by(x => if x % 2 == 0 { 'even' } else { 'odd' }, 0, (sum,x) => sum+x)
assert(groups == [{key:'odd',value:9},{key:'even',value:6}])
let counts = [1,2,3] | fold_by(x => if x == 2 { 'b' } else { 'a' }, {count:0}, (a,x) => {count:a.count+1})
assert(counts[0].value.count == 2)
assert(counts[1].value.count == 1)
assert(fold_by([], x => 'a', 0, (a,x) => a+x) == [])"#, 10000).unwrap();
    for source in [
        "fold_by([], 1, 0, (a,x) => a)",
        "fold_by([], x => 'a', 0, 1)",
        "fold_by([1], x => x, 0, (a,x) => a)",
    ] {
        assert!(evaluate(source, 1000).is_err(), "{source}");
    }
}

#[test]
fn fold_by_handles_large_stream_with_small_group_limit() {
    let program =
        Program::compile("range_iter(0,10000) | fold_by(x => 'all', 0, (a,x) => a+x)").unwrap();
    let result = program
        .run_with_limits(
            ExecutionLimits {
                max_collection_items: 2,
                ..ExecutionLimits::new(1_000_000)
            },
            &CancellationToken::default(),
            &[],
            &[],
            &[],
        )
        .unwrap();
    let expected = evaluate("[{key:'all',value:49995000}]", 1000).unwrap();
    assert_eq!(result, expected);
}
