use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use themoretheless_tokenizer_rush::{
    CancellationToken, HostFunction, HostSequence, HostSequenceIterator, Program, Value, ValueType,
};

#[derive(Debug, Default)]
struct Counts {
    live: Cell<usize>,
    peak: Cell<usize>,
    created: Cell<usize>,
}
thread_local! { static COUNTS: RefCell<Rc<Counts>> = RefCell::new(Rc::default()); }
#[derive(Debug)]
struct Tracked(Rc<Counts>);
impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.live.set(self.0.live.get() - 1);
    }
}
impl HostSequence for Tracked {
    fn open(&self, _: &CancellationToken) -> Result<Box<dyn HostSequenceIterator>, String> {
        Err("This lifetime fixture must not be consumed".into())
    }
}
fn tracked<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    COUNTS.with(|counts| {
        let counts = counts.borrow().clone();
        counts.created.set(counts.created.get() + 1);
        counts.live.set(counts.live.get() + 1);
        counts.peak.set(counts.peak.get().max(counts.live.get()));
        Ok(Value::host_sequence(
            Rc::new(Tracked(counts)),
            ValueType::Number,
        ))
    })
}
fn run(source: &str) -> (Value<'_>, Rc<Counts>) {
    let counts = Rc::new(Counts::default());
    COUNTS.with(|slot| *slot.borrow_mut() = counts.clone());
    let function = Rc::new(HostFunction {
        name: "tracked",
        parameters: vec![],
        result: ValueType::Sequence,
        callback: tracked,
    });
    let live = Rc::new(HostFunction {
        name: "live",
        parameters: vec![],
        result: ValueType::Number,
        callback: |_, _| {
            Ok(Value::Number(
                COUNTS.with(|counts| counts.borrow().live.get()) as f64,
            ))
        },
    });
    let value = Program::compile(source)
        .unwrap()
        .run_with_host(
            100_000,
            &CancellationToken::default(),
            &[],
            &[function, live],
        )
        .unwrap();
    (value, counts)
}

#[test]
fn dead_loop_and_call_cells_do_not_retain_all_prior_values() {
    for source in [
        "for i in range_iter(0,1000) { mut local = tracked(); 0 }; 7",
        "fn work() { mut local = tracked(); return 0 }; for i in range_iter(0,1000) { work() }; 7",
        "fn work() { mut local = tracked(); mut f = () => local; return 0 }; for i in range_iter(0,1000) { work() }; 7",
    ] {
        let (value, counts) = run(source);
        assert_eq!(value, Value::Number(7.0));
        assert_eq!(counts.created.get(), 1000);
        assert_eq!(counts.peak.get(), 1);
        assert_eq!(counts.live.get(), 0);
    }
}

#[test]
fn escaped_mutable_captures_survive_other_cell_reuse() {
    let (value, counts) = run("fn counter() { mut n: number = 10; return () => n += 1 }; \
         let next = counter(); \
         for i in range_iter(0,1000) { mut local = tracked(); 0 }; \
         (next(), next())");
    assert_eq!(
        value,
        Value::Tuple(vec![Value::Number(11.0), Value::Number(12.0)])
    );
    assert!(counts.peak.get() <= 2);
    assert_eq!(counts.live.get(), 0);
}

#[test]
fn a_self_capturing_cell_is_released_when_execution_ends() {
    let (value, counts) = run("mut resource = tracked(); mut f = () => resource; f = () => f; 0");
    assert_eq!(value, Value::Number(0.0));
    assert_eq!(counts.live.get(), 0);
}

#[test]
fn scope_exit_releases_values_before_the_next_host_call() {
    for source in [
        "if true { mut local = tracked(); 0 }; live()",
        "while true { mut local = tracked(); break }; live()",
        "for i in range_iter(0,3) { mut local = tracked(); continue }; live()",
        "fn work() { mut local = tracked(); return 0 }; work(); live()",
        "fn work() { if true { mut local = tracked(); return 0 } }; work(); live()",
        "fn work() { mut local = tracked(); mut f = () => local; return 0 }; work(); live()",
    ] {
        let (value, counts) = run(source);
        assert_eq!(value, Value::Number(0.0), "{source}");
        assert_eq!(counts.live.get(), 0);
    }
}

#[test]
fn returned_value_outlives_its_reclaimed_cell() {
    let (value, counts) = run("fn make() { mut local = tracked(); return local }; make()");
    assert!(matches!(value, Value::Sequence(_)));
    assert_eq!(counts.live.get(), 1);
    drop(value);
    assert_eq!(counts.live.get(), 0);
}

#[test]
fn unreachable_cycles_are_reclaimed_during_execution() {
    for body in [
        "mut resource = tracked(); mut f = () => resource; f = () => (resource,f); return 0",
        "mut resource = tracked(); mut f = () => resource; mut record = {callback:f}; f = () => record; return 0",
        "mut resource = tracked(); mut sequence = range_iter(0,1); let f = () => (resource,sequence); sequence = iter([f]); return 0",
        "mut resource = tracked(); mut sequence = range_iter(0,1); sequence = sequence | map(x => (resource,sequence)); return 0",
    ] {
        let source = format!("fn work() {{ {body} }}; for i in range_iter(0,1000) {{ work() }}; 0");
        let (value, counts) = run(&source);
        assert_eq!(value, Value::Number(0.0));
        assert_eq!(counts.created.get(), 1000);
        assert!(
            counts.peak.get() <= 64,
            "{body}: peak {}",
            counts.peak.get()
        );
        assert_eq!(counts.live.get(), 0);
    }
}

#[test]
fn reachable_recursive_function_survives_collection_through_a_sequence() {
    let (value, counts) = run(
        "fn make() { mut f = x => x; f = x => if x > 0 { f(x-1) } else { 7 }; return iter([f]) }; \
         let saved = make(); \
         for i in range_iter(0,1000) { mut scratch = tracked(); 0 }; \
         let callbacks = saved | collect(1); callbacks[0](3)",
    );
    assert_eq!(value, Value::Number(7.0));
    assert_eq!(counts.live.get(), 0);
}

#[test]
fn active_callee_and_shared_captures_are_roots_during_collection() {
    for (source, expected) in [
        (
            "fn make() { mut f = x => x; f = x => if x > 0 { f(x-1) } else { 7 }; return f }; \
             fn churn() { for i in range_iter(0,1000) { mut scratch = tracked(); 0 }; return 3 }; \
             make()(churn())",
            7.0,
        ),
        (
            "fn make() { mut n = 0; return (() => n += 1, () => n += 10) }; \
             let pair = make(); \
             for i in range_iter(0,1000) { mut scratch = tracked(); 0 }; \
             pair[0](); pair[1]()",
            11.0,
        ),
    ] {
        let (value, counts) = run(source);
        assert_eq!(value, Value::Number(expected));
        assert_eq!(counts.live.get(), 0);
    }
}

#[test]
fn collection_exhausts_the_shared_budget_and_program_can_run_again() {
    // The 64th declaration triggers collection. Its expression has already
    // evaluated when the collector needs more steps to visit the live cells.
    let mut source = (0..64)
        .map(|i| format!("mut value{i} = {i}; "))
        .collect::<String>();
    let initializer = source.rfind("63;").unwrap();
    source.push_str("value0 + value63");
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let error = program.run(180, &token, &[]).unwrap_err();
    assert_eq!(error.message, "Execution limit exceeded");
    assert_eq!(error.span.start, initializer);
    assert_eq!(program.run(2000, &token, &[]).unwrap(), Value::Number(63.0));
}

#[test]
fn interrupted_collection_drops_host_values_and_does_not_poison_the_program() {
    let counts = Rc::new(Counts::default());
    COUNTS.with(|slot| *slot.borrow_mut() = counts.clone());
    let function = Rc::new(HostFunction {
        name: "tracked",
        parameters: vec![],
        result: ValueType::Sequence,
        callback: tracked,
    });
    let mut source = String::from("mut resource = tracked(); ");
    for i in 0..63 {
        source.push_str(&format!("mut value{i} = {i}; "));
    }
    let initializer = source.rfind("62;").unwrap();
    source.push('7');
    let program = Program::compile(&source).unwrap();
    let token = CancellationToken::default();
    let error = program
        .run_with_host(180, &token, &[], std::slice::from_ref(&function))
        .unwrap_err();
    assert_eq!(error.message, "Execution limit exceeded");
    assert_eq!(error.span.start, initializer);
    assert_eq!(counts.created.get(), 1);
    assert_eq!(counts.live.get(), 0);
    assert_eq!(
        program
            .run_with_host(2000, &token, &[], &[function])
            .unwrap(),
        Value::Number(7.0)
    );
    assert_eq!(counts.created.get(), 2);
    assert_eq!(counts.live.get(), 0);
}

#[test]
fn closures_do_not_retain_unused_locals_or_shadowed_outer_parameters() {
    for source in [
        "fn make() { let resource = tracked(); return () => 7 }; let f = make(); live()",
        "fn make() { mut resource = tracked(); return () => 7 }; let f = make(); live()",
        "fn make() { let resource = tracked(); return resource => resource }; let f = make(); live()",
        "fn make() { let resource = tracked(); fn nested(resource) { return resource }; return nested }; let f = make(); live()",
    ] {
        let (value, counts) = run(source);
        assert_eq!(value, Value::Number(0.0), "{source}");
        assert_eq!(counts.live.get(), 0);
    }
}

#[test]
fn nested_closure_keeps_the_outer_free_variable_it_needs() {
    let (value, counts) = run(
        "fn make() { let resource = tracked(); return () => () => resource }; \
         let outer = make(); let inner = outer(); inner()",
    );
    assert!(matches!(value, Value::Sequence(_)));
    assert_eq!(counts.live.get(), 1);
    drop(value);
    assert_eq!(counts.live.get(), 0);
}

#[test]
fn host_supplied_sequence_preserves_its_closures_source_metadata() {
    let function = Rc::new(HostFunction {
        name: "foreign",
        parameters: vec![],
        result: ValueType::Sequence,
        callback: |_, token| {
            Program::compile("let base = 2; range_iter(0,1) | map(x => y => base+x+y)")
                .unwrap()
                .run(1000, token, &[])
                .map_err(|error| error.message)
        },
    });
    let result = Program::compile("let values = foreign() | collect(1); values[0](3)")
        .unwrap()
        .run_with_host(1000, &CancellationToken::default(), &[], &[function])
        .unwrap();
    assert_eq!(result, Value::Number(5.0));
}

#[test]
fn foreign_mutable_captures_fail_without_reading_another_runs_slots() {
    for foreign_source in [
        "mut base = 2; range_iter(0,1) | map(x => base)",
        "mut base = 2; range_iter(0,1) | map(x => base += 1)",
        "mut base = 2; range_iter(0,1) | map(x => base = 7)",
        "mut base = 2; range_iter(0,1) | map(x => base = 1/0)",
    ] {
        FOREIGN_SOURCE.with(|slot| slot.set(foreign_source));
        let function = Rc::new(HostFunction {
            name: "foreign",
            parameters: vec![],
            result: ValueType::Sequence,
            callback: |_, token| {
                FOREIGN_SOURCE.with(|source| {
                    Program::compile(source.get())
                        .unwrap()
                        .run(1000, token, &[])
                        .map_err(|error| error.message)
                })
            },
        });
        for source in [
            "foreign() | collect(1)",
            "mut unrelated = 40; foreign() | collect(1)",
            "mut saved = foreign(); for i in range_iter(0,1000) { mut scratch = i; 0 }; saved | collect(1)",
        ] {
            let program = Program::compile(source).unwrap();
            let error = program
                .run_with_host(
                    100_000,
                    &CancellationToken::default(),
                    &[],
                    std::slice::from_ref(&function),
                )
                .unwrap_err();
            assert_eq!(
                error.message,
                "Mutable capture belongs to another execution"
            );
        }
    }
}
thread_local! { static FOREIGN_SOURCE: Cell<&'static str> = const { Cell::new("") }; }
