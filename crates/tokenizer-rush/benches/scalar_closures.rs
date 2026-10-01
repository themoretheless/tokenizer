// This target uses the closure subset of the shared experimental engines.
#[allow(dead_code)]
#[path = "support/scalar.rs"]
mod scalar;
use scalar::{Scalar, closures::Factory};
use std::{hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{CancellationToken, Value, evaluate};

fn main() {
    if cfg!(debug_assertions) {
        println!("Use optimized bench profile");
        return;
    }
    let tree_factory = Factory::compile("factor => x => x*factor").unwrap();
    let vm_factory = tree_factory.bytecode();
    let tree = tree_factory.bind(&[Scalar::Number(3.0)]).unwrap();
    let vm = vm_factory.bind(&[Scalar::Number(3.0)]).unwrap();
    let mut tree_stack = tree.stack();
    let mut vm_stack = vm.stack();
    let token = CancellationToken::default();
    println!(
        "Two-stage immutable scalar closure; calls driven by Rust harness, not script call instructions"
    );
    println!(
        "Same frame assembly/allocation in both engines; reused VM operand stack; 10 warmups and 8 alternating samples"
    );
    println!("items\toperation\tengine\tsample\tus_per_pipeline\tverified_sum");
    for items in [100usize, 1000, 10000] {
        let expected = (0..items as u64).map(|x| x * 3).sum::<u64>() as f64;
        let source = format!(
            "const scale = factor => x => x*factor; const triple = scale(3); range_iter(0,{items}) | map(triple) | fold(0,(sum,x)=>sum+x)"
        );
        assert_eq!(
            evaluate(&source, 1_000_000).unwrap(),
            Value::Number(expected)
        );
        for create in [false, true] {
            for sample in -10i32..8 {
                for slot in 0..2 {
                    let bytecode = (sample + slot).rem_euclid(2) == 1;
                    let (factory, closure, stack) = if bytecode {
                        (&vm_factory, &vm, &mut vm_stack)
                    } else {
                        (&tree_factory, &tree, &mut tree_stack)
                    };
                    let iterations = if sample < 0 { 1 } else { 100 };
                    let start = Instant::now();
                    for _ in 0..iterations {
                        let mut total = 0.0;
                        for x in 0..items {
                            let result = if create {
                                let bound =
                                    factory.bind(black_box(&[Scalar::Number(3.0)])).unwrap();
                                bound.run(
                                    black_box(&[Scalar::Number(x as f64)]),
                                    100,
                                    &token,
                                    stack,
                                )
                            } else {
                                closure.run(
                                    black_box(&[Scalar::Number(x as f64)]),
                                    100,
                                    &token,
                                    stack,
                                )
                            }
                            .unwrap();
                            let Scalar::Number(value) = black_box(result) else {
                                panic!()
                            };
                            total += value;
                        }
                        assert_eq!(black_box(total), expected);
                    }
                    if sample >= 0 {
                        println!(
                            "{items}\t{}\t{}\t{sample}\t{:.3}\t{expected}",
                            if create {
                                "create_and_call"
                            } else {
                                "prepared_closure"
                            },
                            if bytecode { "bytecode" } else { "lowered_tree" },
                            start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations)
                        );
                    }
                }
            }
        }
    }
}
