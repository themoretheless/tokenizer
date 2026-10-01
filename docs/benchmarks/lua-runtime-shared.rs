use mlua::Lua;
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
        "local values = range_values({items})\nlocal mapped = map_values(values, function(x) return x*2.0 end)\nlocal filtered = filter_values(mapped, function(x) return x%3.0 == 0.0 end)\nreturn fold_values(filtered, 0.0, function(sum,x) return sum+x end)"
    );
    let lazy = format!(
        "local values = range_iterator({items})\nlocal mapped = map_iterator(values, function(x) return x*2.0 end)\nlocal filtered = filter_iterator(mapped, function(x) return x%3.0 == 0.0 end)\nreturn fold_iterator(filtered, 0.0, function(sum,x) return sum+x end)"
    );
    let expected: u64 = (0..items as u64)
        .map(|x| x * 2)
        .filter(|x| x % 3 == 0)
        .sum();
    let workloads = [
        (
            "closure",
            "local scale = function(factor) return function(x) return x*factor end end\nlocal twice = scale(2.0)\nreturn twice(21.0)",
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
    let lua = Lua::new();
    lua.load(include_str!("lua-collections.lua"))
        .exec()
        .unwrap();
    lua.load(include_str!("lua-collections-check.lua"))
        .exec()
        .unwrap();
    let version: String = lua.globals().get("_VERSION").unwrap();
    println!(
        "{} · {version} · mlua 0.12.1 · {} {} · no execution budget; default GC",
        env!("CARGO_PKG_NAME"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("Collection input items: {items}; verified expected sum: {expected}");
    println!(
        "7 samples, 10 warmups; VM and support-library construction excluded; local script bindings"
    );
    println!("workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us");
    for (name, source, expected) in workloads {
        if selected.as_ref().is_some_and(|s| s != name) {
            continue;
        }
        let function = lua.load(source).into_function().unwrap();
        assert_eq!(function.call::<f64>(()).unwrap(), expected);
        if operation.as_deref().is_none_or(|s| s == "compile") {
            measure(name, "compile", iterations, || {
                lua.load(black_box(source)).into_function().unwrap()
            });
        }
        if operation.as_deref().is_none_or(|s| s == "prepared_run") {
            measure(name, "prepared_run", iterations, || {
                function.call::<f64>(()).unwrap()
            });
        }
        if operation.as_deref().is_none_or(|s| s == "compile_and_run") {
            measure(name, "compile_and_run", iterations, || {
                lua.load(black_box(source)).eval::<f64>().unwrap()
            });
        }
    }
}
