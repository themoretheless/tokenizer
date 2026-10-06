use rhai::{Engine, FLOAT};
use std::{hint::black_box, time::Instant};
fn measure<T>(name: &str, operation: &str, iterations: usize, mut action: impl FnMut() -> T) {
    for _ in 0..10 {
        black_box(action());
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(action());
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
        eprintln!("Use --release for measurements");
        std::process::exit(2);
    }
    let iterations: usize = std::env::var("RUSH_BENCH_ITERS")
        .unwrap_or("100".into())
        .parse()
        .unwrap();
    let items: usize = std::env::var("RUSH_BENCH_ITEMS")
        .unwrap_or("1000".into())
        .parse()
        .unwrap();
    assert!(iterations > 0 && (1..=1_000_000).contains(&items));
    let mut engine = Engine::new();
    engine.set_max_operations((items * 100 + 100_000) as u64);
    let expected: u64 = (0..items as u64)
        .map(|x| x * 2)
        .filter(|x| x % 3 == 0)
        .sum();
    let collection = format!(
        "let values = []; for x in 0..{items} {{ values.push(x.to_float()); }} values.map(|x| x * 2.0).filter(|x| x % 3.0 == 0.0).reduce(|sum,x| sum+x, 0.0)"
    );
    let workloads = [
        (
            "closure",
            "let scale = |factor| |x| x * factor; let twice = scale.call(2.0); twice.call(21.0)",
            42.0,
        ),
        ("collections", collection.as_str(), expected as FLOAT),
    ];
    println!(
        "Rhai · {} {} · default features; max operations {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        items * 100 + 100_000
    );
    println!("Collection input items: {items}; verified expected sum: {expected}");
    println!("7 samples, 10 warmup operations; engine construction excluded");
    println!("workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us");
    let selected = std::env::var("RUSH_BENCH_CASE").ok();
    let operation = std::env::var("RUSH_BENCH_OPERATION").ok();
    assert!(
        selected
            .as_ref()
            .is_none_or(|s| workloads.iter().any(|(name, _, _)| name == s))
    );
    assert!(
        operation
            .as_deref()
            .is_none_or(|s| ["compile", "prepared_run", "compile_and_run"].contains(&s))
    );
    for (name, source, expected) in workloads {
        if selected.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        let ast = engine.compile(source).unwrap();
        assert_eq!(engine.eval_ast::<FLOAT>(&ast).unwrap(), expected);
        if operation.as_deref().is_none_or(|s| s == "compile") {
            measure(name, "compile", iterations, || {
                engine.compile(black_box(source)).unwrap()
            });
        }
        if operation.as_deref().is_none_or(|s| s == "prepared_run") {
            measure(name, "prepared_run", iterations, || {
                engine.eval_ast::<FLOAT>(&ast).unwrap()
            });
        }
        if operation.as_deref().is_none_or(|s| s == "compile_and_run") {
            measure(name, "compile_and_run", iterations, || {
                engine.eval::<FLOAT>(black_box(source)).unwrap()
            });
        }
    }
}
