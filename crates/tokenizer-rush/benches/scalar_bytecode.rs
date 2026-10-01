//! Dispatch experiment only; no replacement of the general Rush runtime.
#[path = "support/scalar.rs"]
mod scalar;
use scalar::{Scalar, Tree};
use std::{hint::black_box, rc::Rc, time::Instant};
use themoretheless_tokenizer_rush::{CancellationToken, HostFunction, Program, Value, ValueType};

fn double<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::Number(n) = args[0] else {
        unreachable!()
    };
    Ok(Value::Number(n * 2.0))
}
fn selected<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::Number(n) = args[0] else {
        unreachable!()
    };
    Ok(Value::Bool(n % 3.0 == 0.0))
}

fn pipeline(
    mut map: impl FnMut(&[Scalar]) -> Scalar,
    mut filter: impl FnMut(&[Scalar]) -> Scalar,
    mut fold: impl FnMut(&[Scalar]) -> Scalar,
    items: usize,
) -> Scalar {
    let mut total = Scalar::Number(0.0);
    for x in 0..items {
        let value = map(&[Scalar::Number(x as f64)]);
        if filter(&[value]) == Scalar::Bool(true) {
            total = fold(&[total, value]);
        }
    }
    total
}

fn main() {
    if cfg!(debug_assertions) {
        println!("Use optimized bench profile");
        return;
    }
    if std::env::var("RUSH_SCALAR_OPERATION").as_deref() == Ok("prepare") {
        preparation();
        return;
    }
    let token = CancellationToken::default();
    let scenario = std::env::var("RUSH_SCALAR_CASE").unwrap_or_else(|_| "arithmetic".into());
    assert!(matches!(
        scenario.as_str(),
        "arithmetic" | "branches" | "host"
    ));
    let hosts = if scenario == "host" {
        vec![
            Rc::new(HostFunction {
                name: "double",
                parameters: vec![ValueType::Number],
                result: ValueType::Number,
                callback: double,
            }),
            Rc::new(HostFunction {
                name: "selected",
                parameters: vec![ValueType::Number],
                result: ValueType::Bool,
                callback: selected,
            }),
        ]
    } else {
        vec![]
    };
    let branches = scenario == "branches";
    let map_source = if branches {
        "(if x%2==0 { x*2 } else { x })"
    } else if scenario == "host" {
        "double(x)"
    } else {
        "x*2"
    };
    let filter_source = if branches {
        "x%3==0 and x<15000"
    } else if scenario == "host" {
        "selected(x)"
    } else {
        "x%3==0"
    };
    let map = Tree::compile_with_hosts(map_source, &["x"], &hosts).unwrap();
    let filter = Tree::compile_with_hosts(filter_source, &["x"], &hosts).unwrap();
    let fold = Tree::compile("sum+x", &["sum", "x"]).unwrap();
    let (map_vm, filter_vm, fold_vm) = (map.bytecode(), filter.bytecode(), fold.bytecode());
    let (mut map_stack, mut filter_stack, mut fold_stack) =
        (map_vm.stack(), filter_vm.stack(), fold_vm.stack());
    let iterations = 100;
    println!("Scenario: {scenario}; map={map_source}; filter={filter_source}");
    println!("Scalar dispatch experiment; 10 warmups, 8 alternating samples; 100 pipelines/sample");
    println!("Predecoded constants and positional parameters in BOTH engines; reused VM stacks");
    println!(
        "Tree is lowered scalar AST, not general Rush runtime; scalar host calls supported; no closures/collections"
    );
    println!("items\tengine\tsample\tus_per_pipeline\tverified_sum");
    for items in [100, 1000, 10000] {
        let expected: u64 = (0..items as u64)
            .map(|x| if !branches || x % 2 == 0 { x * 2 } else { x })
            .filter(|x| x % 3 == 0 && (!branches || *x < 15000))
            .sum();
        let source = format!(
            "range_iter(0,{items}) | map(x=>{map_source}) | filter(x=>{filter_source}) | fold(0,(sum,x)=>sum+x)"
        );
        let program = Program::compile(&source).unwrap();
        assert_eq!(
            program
                .run_with_host(1_000_000, &token, &[], &hosts)
                .unwrap(),
            Value::Number(expected as f64)
        );
        for sample in -10i32..8 {
            for slot in 0..2 {
                let vm = (sample + slot).rem_euclid(2) == 1;
                let start = Instant::now();
                for _ in 0..if sample < 0 { 1 } else { iterations } {
                    let result = if vm {
                        pipeline(
                            |args| {
                                map_vm
                                    .run(black_box(args), 100, &token, &mut map_stack)
                                    .unwrap()
                            },
                            |args| {
                                filter_vm
                                    .run(black_box(args), 100, &token, &mut filter_stack)
                                    .unwrap()
                            },
                            |args| {
                                fold_vm
                                    .run(black_box(args), 100, &token, &mut fold_stack)
                                    .unwrap()
                            },
                            items,
                        )
                    } else {
                        pipeline(
                            |args| map.run(black_box(args), 100, &token).unwrap(),
                            |args| filter.run(black_box(args), 100, &token).unwrap(),
                            |args| fold.run(black_box(args), 100, &token).unwrap(),
                            items,
                        )
                    };
                    assert_eq!(black_box(result), Scalar::Number(expected as f64));
                }
                if sample >= 0 {
                    println!(
                        "{items}\t{}\t{sample}\t{:.3}\t{expected}",
                        if vm { "bytecode" } else { "lowered_tree" },
                        start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations)
                    );
                }
            }
        }
    }
}

fn preparation() {
    fn sample<T>(iterations: usize, mut operation: impl FnMut() -> T) -> f64 {
        let start = Instant::now();
        for _ in 0..iterations {
            drop(black_box(operation()));
        }
        start.elapsed().as_secs_f64() * 1e6 / iterations as f64
    }
    println!(
        "Preparation including drop; 10 warmups per operation, 9 cyclic samples of 1000 preparations"
    );
    println!("source\toperation\tsample\tus_per_preparation");
    for source in [
        "x*2",
        "(if x%2==0 { x*2 } else { x })",
        "x%3==0 and x<15000",
    ] {
        let tree = Tree::compile(source, &["x"]).unwrap();
        let token = CancellationToken::default();
        let vm = tree.bytecode();
        let mut stack = vm.stack();
        assert_eq!(
            tree.run(&[Scalar::Number(12.0)], 100, &token),
            vm.run(&[Scalar::Number(12.0)], 100, &token, &mut stack)
        );
        for round in -10i32..9 {
            for slot in 0..3 {
                let operation = (round + slot).rem_euclid(3);
                let iterations = if round < 0 { 1 } else { 1000 };
                let (name, elapsed) = match operation {
                    0 => (
                        "parse_and_tree",
                        sample(iterations, || {
                            Tree::compile(black_box(source), &["x"]).unwrap()
                        }),
                    ),
                    1 => (
                        "tree_to_instructions",
                        sample(iterations, || black_box(&tree).bytecode()),
                    ),
                    _ => (
                        "parse_and_instructions",
                        sample(iterations, || {
                            Tree::compile(black_box(source), &["x"]).unwrap().bytecode()
                        }),
                    ),
                };
                if round >= 0 {
                    println!("{source}\t{name}\t{round}\t{elapsed:.3}");
                }
            }
        }
    }
}
