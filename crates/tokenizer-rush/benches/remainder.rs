//! Integer fast path and floating-point fallback controls, with independent sums.
use std::{hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};
fn main() {
    if cfg!(debug_assertions) {
        return;
    }
    let iterations: usize = std::env::var("RUSH_BENCH_ITERS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(100);
    assert!(iterations > 0);
    let selected = std::env::var("RUSH_BENCH_CASE").ok();
    for (case, offset, divisor) in [
        ("integer_rem", 0., 3.),
        ("fractional_rem", 0.25, 3.5),
        ("large_rem", 9_007_199_254_740_992., 3.),
    ] {
        if selected.as_ref().is_some_and(|v| v != case) {
            continue;
        }
        let source = format!(
            "range_iter(0,1000) | map(x => (x+{offset})%{divisor}) | fold(0,(sum,x)=>sum+x)"
        );
        let program = Program::compile(&source).unwrap();
        let token = CancellationToken::default();
        let expected = (0..1000)
            .map(|x| (x as f64 + offset) % divisor)
            .sum::<f64>();
        assert_eq!(
            program.run(1000000, &token, &[]).unwrap(),
            Value::Number(expected)
        );
        let run = || black_box(program.run(1000000, &token, &[]).unwrap());
        for _ in 0..10 {
            run();
        }
        let mut samples = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            for _ in 0..iterations {
                run();
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{case}\tprepared_run\t{iterations}\t{:.3}\t{:.3}\t{:.3}",
            samples[0], samples[3], samples[6]
        );
    }
}
