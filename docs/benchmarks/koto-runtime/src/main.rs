use koto::prelude::*;
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
    let items: usize = std::env::var("RUSH_BENCH_ITEMS")
        .unwrap_or("1000".into())
        .parse()
        .unwrap();
    let iterations: usize = std::env::var("RUSH_BENCH_ITERS")
        .unwrap_or("100".into())
        .parse()
        .unwrap();
    assert!(iterations > 0 && (1..=1_000_000).contains(&items));
    let selected = std::env::var("RUSH_BENCH_CASE").ok();
    let operation = std::env::var("RUSH_BENCH_OPERATION").ok();
    assert!(
        operation
            .as_deref()
            .is_none_or(|s| ["compile", "prepared_run", "compile_and_run"].contains(&s))
    );
    let eager = format!(
        "values = iterator.each(0..{items}, |x| x * 1.0).to_list()\nmapped = iterator.each(values, |x| x * 2.0).to_list()\nfiltered = iterator.keep(mapped, |x| x % 3.0 == 0.0).to_list()\niterator.fold(filtered, 0.0, |sum,x| sum+x)"
    );
    let lazy = format!(
        "values = iterator.each(0..{items}, |x| x * 1.0)\nmapped = iterator.each(values, |x| x * 2.0)\nfiltered = iterator.keep(mapped, |x| x % 3.0 == 0.0)\niterator.fold(filtered, 0.0, |sum,x| sum+x)"
    );
    let expected: u64 = (0..items as u64)
        .map(|x| x * 2)
        .filter(|x| x % 3 == 0)
        .sum();
    let workloads = [
        (
            "closure",
            "scale = |factor| |x| x * factor\ntwice = scale(2.0)\ntwice(21.0)",
            42.0,
        ),
        ("collections", eager.as_str(), expected as f64),
        ("collections_lazy", lazy.as_str(), expected as f64),
    ];
    assert!(
        selected
            .as_ref()
            .is_none_or(|s| workloads.iter().any(|(name, _, _)| name == s))
    );
    let mut koto = Koto::default();
    println!(
        "Koto 0.16.1 · {} {} · default features; no execution budget; exports cleared per run",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("Collection input items: {items}; verified expected sum: {expected}");
    println!("7 samples, 10 warmups; VM construction excluded");
    println!("workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us");
    for (name, source, expected) in workloads {
        if selected.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        let chunk = koto.compile(source).unwrap();
        koto.exports_mut().clear();
        let KValue::Number(value) = koto.run(chunk.clone()).unwrap() else {
            panic!("Expected number");
        };
        assert_eq!(f64::from(value), expected);
        if operation.as_deref().is_none_or(|s| s == "compile") {
            measure(name, "compile", iterations, || {
                koto.compile(black_box(source)).unwrap()
            });
        }
        if operation.as_deref().is_none_or(|s| s == "prepared_run") {
            measure(name, "prepared_run", iterations, || {
                koto.exports_mut().clear();
                koto.run(chunk.clone()).unwrap()
            });
        }
        if operation.as_deref().is_none_or(|s| s == "compile_and_run") {
            measure(name, "compile_and_run", iterations, || {
                koto.exports_mut().clear();
                koto.compile_and_run(black_box(source)).unwrap()
            });
        }
    }
}
