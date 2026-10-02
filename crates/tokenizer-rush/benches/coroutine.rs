//! End-to-end coroutine creation, loop traversal, yielding and cleanup.
use std::{hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineState, ExecutionLimits, Program, Value,
};
fn main() {
    if cfg!(debug_assertions) {
        return;
    }
    let iterations: usize = std::env::var("RUSH_BENCH_ITERS")
        .ok()
        .map(|v| v.parse().unwrap())
        .unwrap_or(100);
    let limits = ExecutionLimits::new(1_000_000);
    let token = CancellationToken::default();
    for (case, source) in [
        (
            "while",
            "fn work()->number {mut i=0;mut sum=0;while i<1000 {sum+=i;i+=1};return sum}",
        ),
        (
            "for",
            "fn work()->number {mut sum=0;for i in range_iter(0,1000) {sum+=i};return sum}",
        ),
        (
            "yield",
            "fn work()->number {mut sum=0;for i in range_iter(0,1000) {sum+=i;yield i};return sum}",
        ),
    ] {
        let program = Program::compile(source).unwrap();
        let mut script = program
            .instantiate(limits, &token, &[], &[], &[], &[])
            .unwrap();
        let mut run = || {
            let task = script.spawn_coroutine("work", &[], limits).unwrap();
            let mut yields = 0;
            loop {
                match script.resume_coroutine(task, limits).unwrap() {
                    CoroutineState::Yielded(value) => {
                        assert_eq!(value, Value::Number(yields as f64));
                        yields += 1;
                    }
                    CoroutineState::Complete(value) => {
                        assert_eq!(value, Value::Number(499500.));
                        black_box(value);
                        break;
                    }
                }
            }
            assert_eq!(yields, if case == "yield" { 1000 } else { 0 });
        };
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
            "{case}\t{iterations}\t{:.3}\t{:.3}\t{:.3}",
            samples[0], samples[3], samples[6]
        );
    }
}
