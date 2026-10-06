//! cargo bench -p themoretheless-tokenizer-rush --bench runtime
use std::{hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{CancellationToken, Program, Value, evaluate};

fn measure<T>(name: &str, operation: &str, iterations: usize, mut f: impl FnMut() -> T) {
    for _ in 0..10 {
        black_box(f());
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(f());
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name}\t{operation}\t{iterations}\t{:.3}\t{:.3}\t{:.3}",
        samples[0], samples[3], samples[6]
    );
}

fn main() {
    if cfg!(debug_assertions) {
        println!("Use the optimized bench profile");
        return;
    }
    let iterations = std::env::var("RUSH_BENCH_ITERS")
        .ok()
        .map(|s| s.parse::<usize>().expect("positive RUSH_BENCH_ITERS"))
        .unwrap_or(100);
    assert!(iterations > 0);
    let items = std::env::var("RUSH_BENCH_ITEMS")
        .ok()
        .map(|s| s.parse::<usize>().expect("integer RUSH_BENCH_ITEMS"))
        .unwrap_or(1000);
    assert!(
        (1..=1_000_000).contains(&items),
        "RUSH_BENCH_ITEMS must be between 1 and 1000000"
    );
    let selected = std::env::var("RUSH_BENCH_CASE").ok();
    let operation = std::env::var("RUSH_BENCH_OPERATION").ok();
    if let Some(operation) = &operation {
        assert!(
            ["compile", "prepared_run", "compile_and_run", "verify"].contains(&operation.as_str()),
            "Unknown RUSH_BENCH_OPERATION"
        );
    }
    let eager = format!(
        "range(0,{items}) | map(x => x * 2) | filter(x => x % 3 == 0) | fold(0, (sum,x) => sum + x)"
    );
    let lazy = eager.replacen("range(", "range_iter(", 1);
    let expected_sum: u64 = (0..items as u64)
        .map(|x| x * 2)
        .filter(|x| x % 3 == 0)
        .sum();
    let budget = items * 100 + 100_000;
    let cells = format!(
        "mut total = 0; for i in range_iter(0,{items}) {{ mut local = i; total += local }}; total"
    );
    let cycles = format!(
        "fn work(i) {{ mut f = x => x; f = x => if x > 0 {{ f(x-1) }} else {{ i }}; return f(1) }}; mut total = 0; for i in range_iter(0,{items}) {{ total += work(i) }}; total"
    );
    let workloads = [
        (
            "closure",
            "const scale = factor => x => x * factor\nconst twice = scale(2)\ntwice(21)",
        ),
        ("collections", eager.as_str()),
        ("collections_lazy", lazy.as_str()),
        ("cells", cells.as_str()),
        ("cells_cycles", cycles.as_str()),
        (
            "surface",
            "grid_mesh(range(-3,3.1,0.2), range(-3,3.1,0.2), (x,y) => vec3(x,sin(x)*cos(y),y))",
        ),
    ];
    if let Some(selected) = &selected {
        assert!(
            workloads.iter().any(|(name, _)| name == selected),
            "Unknown RUSH_BENCH_CASE"
        );
    }
    println!("Collection input items: {items}; verified expected sum: {expected_sum}");
    println!(
        "Rush runtime · {} {} · {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if operation.as_deref() == Some("verify") {
            "one verified execution, no warmup"
        } else {
            "7 samples, 10 warmup operations"
        }
    );
    println!("workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us");
    let cancellation = CancellationToken::default();
    for (name, source) in workloads {
        if selected.as_ref().is_some_and(|selected| selected != name) {
            continue;
        }
        let budget = if name.starts_with("cells") {
            items * 1000 + 100_000
        } else {
            budget
        };
        let program = Program::compile(source).unwrap();
        let start = Instant::now();
        let value = program.run(budget, &cancellation, &[]).unwrap();
        let verified_us = start.elapsed().as_secs_f64() * 1e6;
        match (name, value) {
            ("closure", Value::Number(x)) => assert_eq!(x, 42.),
            ("collections" | "collections_lazy", Value::Number(x)) => {
                assert_eq!(x, expected_sum as f64)
            }
            ("cells" | "cells_cycles", Value::Number(x)) => {
                assert_eq!(x, (items as u64 * (items as u64 - 1) / 2) as f64);
            }
            ("surface", Value::Mesh(mesh)) => {
                assert_eq!(mesh.vertices().len(), 961);
                assert_eq!(mesh.triangles().len(), 1800);
            }
            _ => panic!("Unexpected benchmark result"),
        }
        if operation.as_deref() == Some("verify") {
            println!("{name}\tverify\t1\t{verified_us:.3}\t{verified_us:.3}\t{verified_us:.3}");
            continue;
        }
        if operation
            .as_deref()
            .is_none_or(|operation| operation == "compile")
        {
            measure(name, "compile", iterations, || {
                Program::compile(black_box(source)).unwrap()
            });
        }
        if operation
            .as_deref()
            .is_none_or(|operation| operation == "prepared_run")
        {
            measure(name, "prepared_run", iterations, || {
                program.run(budget, &cancellation, &[]).unwrap()
            });
        }
        if operation
            .as_deref()
            .is_none_or(|operation| operation == "compile_and_run")
        {
            measure(name, "compile_and_run", iterations, || {
                evaluate(black_box(source), budget).unwrap()
            });
        }
    }
}
