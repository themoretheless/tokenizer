use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, HostSequence, HostSequenceIterator, Program, Value, ValueType,
};

#[derive(Debug, Default)]
struct Stats {
    opens: Cell<usize>,
    reads: Cell<usize>,
    closes: Cell<usize>,
}
thread_local! { static CURRENT: RefCell<Rc<Stats>> = RefCell::new(Rc::default()); }
#[derive(Debug)]
struct Source {
    stats: Rc<Stats>,
    mode: u8,
}
struct Reader {
    stats: Rc<Stats>,
    mode: u8,
    index: usize,
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stats.closes.set(self.stats.closes.get() + 1);
    }
}
impl HostSequence for Source {
    fn open(&self, token: &CancellationToken) -> Result<Box<dyn HostSequenceIterator>, String> {
        self.stats.opens.set(self.stats.opens.get() + 1);
        let reader = Reader {
            stats: self.stats.clone(),
            mode: self.mode,
            index: 0,
        };
        if self.mode == 4 {
            return Err("Open failed".into());
        }
        if self.mode == 5 {
            token.cancel();
        }
        Ok(Box::new(reader))
    }
}
impl HostSequenceIterator for Reader {
    fn next(&mut self, token: &CancellationToken) -> Result<Option<Value<'static>>, String> {
        self.stats.reads.set(self.stats.reads.get() + 1);
        match self.mode {
            1 => return Err("Read failed".into()),
            2 => return Ok(Some(Value::Number(f64::NAN))),
            3 => token.cancel(),
            7 => return Ok(Some(Value::List(vec![Value::String("abcd".into())]))),
            _ => {}
        }
        if self.index == 3 && self.mode != 6 {
            return Ok(None);
        }
        let value = self.index as f64;
        self.index += 1;
        Ok(Some(Value::Number(value)))
    }
}
fn source<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::Number(mode) = args[0] else {
        unreachable!()
    };
    Ok(Value::host_sequence(
        Rc::new(Source {
            stats: CURRENT.with(|stats| stats.borrow().clone()),
            mode: mode as u8,
        }),
        if mode == 7. {
            ValueType::List(Box::new(ValueType::String))
        } else {
            ValueType::Number
        },
    ))
}
fn registered() -> Rc<HostFunction> {
    Rc::new(HostFunction {
        name: "source",
        parameters: vec![ValueType::Number],
        result: ValueType::Sequence,
        callback: source,
    })
}
fn fresh() -> Rc<Stats> {
    let stats = Rc::new(Stats::default());
    CURRENT.with(|current| *current.borrow_mut() = stats.clone());
    stats
}
fn counts(stats: &Stats) -> (usize, usize, usize) {
    (stats.opens.get(), stats.reads.get(), stats.closes.get())
}

#[test]
fn sources_open_on_demand_and_close_on_every_consumer_exit() {
    for (text, expected) in [
        ("source(0)", (0, 0, 0)),
        ("source(0) | map(x => x + 1)", (0, 0, 0)),
        ("source(0) | collect(0)", (0, 0, 0)),
        ("source(0) | collect(10)", (1, 4, 1)),
        ("source(0) | group_by(x => 'all')", (1, 4, 1)),
        ("source(0) | collect(1)", (1, 1, 1)),
        ("for x in source(0) { break }", (1, 1, 1)),
        ("fn f() { for x in source(0) { return x } }\nf()", (1, 1, 1)),
        ("source(0) | any(x => x == 0)", (1, 1, 1)),
        ("source(0) | all(x => x < 0)", (1, 1, 1)),
        (
            "source(0) | filter(x => x > 0) | map(x => x*2) | fold(0, (a,b) => a+b)",
            (1, 4, 1),
        ),
        (
            "let s: sequence = source(0)\ncollect(s, 1)\ncollect(s, 2)",
            (2, 3, 2),
        ),
    ] {
        let stats = fresh();
        let program = Program::compile(text).unwrap();
        drop(
            program
                .run_with_host(1000, &CancellationToken::default(), &[], &[registered()])
                .unwrap(),
        );
        assert_eq!(counts(&stats), expected, "{text}");
    }
}

#[test]
fn host_failures_type_errors_cancellation_and_budget_release_the_iterator() {
    for (text, message, expected) in [
        ("source(1) | collect(1)", "Read failed", (1, 1, 1)),
        (
            "source(1) | fold_by(x => 'all', 0, (a,x) => a+x)",
            "Read failed",
            (1, 1, 1),
        ),
        (
            "source(3) | fold_by(x => 'all', 0, (a,x) => a+x)",
            "Execution cancelled",
            (1, 1, 1),
        ),
        (
            "source(0) | fold_by(x => 'all', 0, (a,x) => 1/0)",
            "Non-finite",
            (1, 1, 1),
        ),
        ("source(1) | group_by(x => 'all')", "Read failed", (1, 1, 1)),
        (
            "source(3) | group_by(x => 'all')",
            "Execution cancelled",
            (1, 1, 1),
        ),
        ("source(0) | group_by(x => x)", "Group key", (1, 1, 1)),
        ("source(2) | collect(1)", "declared type", (1, 1, 1)),
        ("source(3) | collect(1)", "Execution cancelled", (1, 1, 1)),
        ("source(4) | collect(1)", "Open failed", (1, 0, 1)),
        ("source(5) | collect(1)", "Execution cancelled", (1, 0, 1)),
        (
            "source(0) | map(x => 1/0) | collect(1)",
            "Non-finite arithmetic result",
            (1, 1, 1),
        ),
        (
            "source(0) | filter(x => 1) | collect(1)",
            "boolean",
            (1, 1, 1),
        ),
    ] {
        let stats = fresh();
        let program = Program::compile(text).unwrap();
        let error = program
            .run_with_host(1000, &CancellationToken::default(), &[], &[registered()])
            .unwrap_err();
        assert!(error.message.contains(message), "{text}: {}", error.message);
        assert_eq!(counts(&stats), expected, "{text}");
    }
    let stats = fresh();
    let program = Program::compile("source(6) | filter(x => false) | collect(1)").unwrap();
    assert!(
        program
            .run_with_host(100, &CancellationToken::default(), &[], &[registered()])
            .unwrap_err()
            .message
            .contains("limit")
    );
    assert_eq!(stats.opens.get(), 1);
    assert_eq!(stats.closes.get(), 1);
    assert!(stats.reads.get() > 0);
}

#[test]
fn host_items_flow_through_transformations_and_reopening_restarts_the_source() {
    let stats = fresh();
    let program = Program::compile("let s = source(0)\nlet sum = s | map(x => x * 2) | fold(0, (a,b) => a+b)\n(sum, collect(s, 2))").unwrap();
    assert_eq!(
        program
            .run_with_host(1000, &CancellationToken::default(), &[], &[registered()])
            .unwrap(),
        Value::Tuple(vec![
            Value::Number(6.),
            Value::List(vec![Value::Number(0.), Value::Number(1.)])
        ])
    );
    assert_eq!(counts(&stats), (2, 6, 2));
}

#[test]
fn depth_exhaustion_in_imported_callbacks_closes_open_sources() {
    use themoretheless_tokenizer_rush::ExecutionLimits;
    let helper = Program::compile("fn count(n) { if n == 0 { return 7 }; return count(n - 1) }; fn visit(x) { return count(6) }; {visit:visit}").unwrap();
    let program =
        Program::compile("import helper; source(6) | map(helper.visit) | collect(1)").unwrap();
    let modules = [("helper", &helper)];
    let mut failures_after_open = 0;
    for max_depth in 0..=16 {
        let stats = fresh();
        let error = program
            .run_with_limits(
                ExecutionLimits {
                    steps: 10000,
                    max_depth,
                    max_collection_items: usize::MAX,
                    max_string_bytes: usize::MAX,
                },
                &Default::default(),
                &[],
                &[registered()],
                &modules,
            )
            .unwrap_err();
        assert!(error.message.contains("limit"), "{max_depth}: {error:?}");
        let (opened, read, closed) = counts(&stats);
        assert_eq!(opened, closed, "depth {max_depth} leaked a host reader");
        if opened != 0 {
            failures_after_open += 1;
            assert_eq!(read, 1, "failed callback must not prefetch more input");
            assert_eq!(error.module.as_deref(), Some("helper"));
        }
    }
    assert!(failures_after_open > 0);
    let stats = fresh();
    assert_eq!(
        program
            .run_with_limits(
                ExecutionLimits::new(10000),
                &Default::default(),
                &[],
                &[registered()],
                &modules
            )
            .unwrap(),
        Value::List(vec![Value::Number(7.)])
    );
    assert_eq!(counts(&stats), (1, 1, 1));
}

#[test]
fn oversized_collect_request_does_not_open_host_source() {
    use themoretheless_tokenizer_rush::ExecutionLimits;
    let stats = fresh();
    let program = Program::compile("source(0) | collect(3)").unwrap();
    let limits = ExecutionLimits {
        max_collection_items: 2,
        ..ExecutionLimits::new(1000)
    };
    let error = program
        .run_with_limits(limits, &Default::default(), &[], &[registered()], &[])
        .unwrap_err();
    assert_eq!(error.message, "Collection item limit exceeded");
    assert_eq!(counts(&stats), (0, 0, 0));
}

#[test]
fn flat_map_size_error_closes_the_source_without_prefetch() {
    use themoretheless_tokenizer_rush::ExecutionLimits;
    let stats = fresh();
    let program = Program::compile("source(6) | flat_map(x => [x,x])").unwrap();
    let limits = ExecutionLimits {
        max_collection_items: 2,
        ..ExecutionLimits::new(1000)
    };
    let error = program
        .run_with_limits(limits, &Default::default(), &[], &[registered()], &[])
        .unwrap_err();
    assert_eq!(error.message, "Collection item limit exceeded");
    assert_eq!(counts(&stats), (1, 2, 1));
}

#[test]
fn oversized_host_item_closes_source_before_callback_or_prefetch() {
    use themoretheless_tokenizer_rush::ExecutionLimits;
    for consumer in ["collect(1)", "map(x => 1/0) | collect(1)", "any(x => 1/0)"] {
        let stats = fresh();
        let source = format!("source(7) | {consumer}");
        let program = Program::compile(&source).unwrap();
        let limits = ExecutionLimits {
            max_string_bytes: 3,
            ..ExecutionLimits::new(1000)
        };
        let error = program
            .run_with_limits(limits, &Default::default(), &[], &[registered()], &[])
            .unwrap_err();
        assert_eq!(error.message, "String byte limit exceeded");
        assert_eq!(counts(&stats), (1, 1, 1));
    }
    let stats = fresh();
    let program = Program::compile("source(7) | collect(1)").unwrap();
    let limits = ExecutionLimits {
        max_string_bytes: 4,
        ..ExecutionLimits::new(1000)
    };
    assert_eq!(
        program
            .run_with_limits(limits, &Default::default(), &[], &[registered()], &[])
            .unwrap(),
        Value::List(vec![Value::List(vec![Value::String("abcd".into())])])
    );
    assert_eq!(counts(&stats), (1, 1, 1));
}

#[test]
fn grouping_limit_closes_an_infinite_source() {
    use themoretheless_tokenizer_rush::ExecutionLimits;
    let stats = fresh();
    let program = Program::compile("source(6) | group_by(x => 'all')").unwrap();
    let error = program
        .run_with_limits(
            ExecutionLimits {
                max_collection_items: 2,
                ..ExecutionLimits::new(10000)
            },
            &CancellationToken::default(),
            &[],
            &[registered()],
            &[],
        )
        .unwrap_err();
    assert!(error.message.contains("limit"), "{}", error.message);
    assert_eq!(counts(&stats), (1, 3, 1));
}

#[test]
fn grouped_aggregation_limits_stop_before_prefetch_and_close_source() {
    use themoretheless_tokenizer_rush::ExecutionLimits;
    for (consumer, items, bytes, expected_reads, message) in [
        (
            "group_by(x => if x == 0 {'a'} else {if x == 1 {'b'} else {'c'}})",
            2,
            100,
            3,
            "Collection item limit",
        ),
        (
            "fold_by(x => if x == 0 {'a'} else {if x == 1 {'b'} else {'c'}}, 0, (a,x) => if x == 2 {1/0} else {a+x})",
            2,
            100,
            3,
            "Collection item limit",
        ),
        (
            "fold_by(x => 'all', [], (a,x) => [a,[x]] | flat_map(xs => xs))",
            2,
            100,
            3,
            "Collection item limit",
        ),
        (
            "fold_by(x => 'all', '', (a,x) => a+'x')",
            2,
            6,
            7,
            "String byte limit",
        ),
    ] {
        let stats = fresh();
        let source = format!("source(6) | {consumer}");
        let program = Program::compile(&source).unwrap();
        let error = program
            .run_with_limits(
                ExecutionLimits {
                    max_collection_items: items,
                    max_string_bytes: bytes,
                    ..ExecutionLimits::new(10000)
                },
                &CancellationToken::default(),
                &[],
                &[registered()],
                &[],
            )
            .unwrap_err();
        assert!(
            error.message.contains(message),
            "{consumer}: {}",
            error.message
        );
        assert_eq!(counts(&stats), (1, expected_reads, 1), "{consumer}");
    }
}

#[test]
fn fold_by_budget_and_reducer_cancellation_close_the_source() {
    let stats = fresh();
    let program = Program::compile("source(6) | fold_by(x => 'all', 0, (a,x) => a+x)").unwrap();
    let error = program
        .run_with_host(500, &CancellationToken::default(), &[], &[registered()])
        .unwrap_err();
    assert!(
        error.message.contains("Execution limit"),
        "{}",
        error.message
    );
    assert_eq!(stats.opens.get(), 1);
    assert_eq!(stats.closes.get(), 1);
    assert!(stats.reads.get() > 0 && stats.reads.get() < 500);

    fn stop<'s>(_: &[Value<'s>], token: &CancellationToken) -> Result<Value<'s>, String> {
        token.cancel();
        Ok(Value::Number(0.0))
    }
    let stats = fresh();
    let stop = Rc::new(HostFunction {
        name: "stop",
        parameters: vec![ValueType::Number, ValueType::Number],
        result: ValueType::Number,
        callback: stop,
    });
    let program = Program::compile("source(6) | fold_by(x => 'all', 0, stop)").unwrap();
    let error = program
        .run_with_host(
            10000,
            &CancellationToken::default(),
            &[],
            &[registered(), stop],
        )
        .unwrap_err();
    assert_eq!(error.message, "Execution cancelled");
    assert_eq!(counts(&stats), (1, 1, 1));
}

#[test]
fn wrong_callback_arity_does_not_open_a_host_source() {
    for consumer in [
        "map((a,b) => a)",
        "fold(0,x => x)",
        "fold_by(x => 'a',0,x => x)",
    ] {
        let stats = fresh();
        let text = format!("source(0) | {consumer}");
        let program = Program::compile(&text).unwrap();
        let error = program
            .run_with_host(10000, &CancellationToken::default(), &[], &[registered()])
            .unwrap_err();
        assert!(error.message.contains("Callback argument count"));
        assert_eq!(counts(&stats), (0, 0, 0));
    }
}
