//! Cancellation latency includes runtime cleanup, but excludes requester startup.
use std::{
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
use themoretheless_tokenizer_rush::{CancellationToken, HostFunction, Program, Value, ValueType};

static ENTERED: AtomicBool = AtomicBool::new(false);

fn ready<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    ENTERED.store(true, Ordering::Release);
    Ok(Value::Null)
}
fn blocking_host<'s>(_: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    ENTERED.store(true, Ordering::Release);
    thread::sleep(Duration::from_millis(20));
    Ok(Value::Null)
}

fn main() {
    if cfg!(debug_assertions) {
        println!("Use the optimized bench profile");
        return;
    }
    let cases = [
        ("while", "ready(); while true {}"),
        ("eager_range", "ready(); range(0, 1000000000000)"),
        (
            "lazy_rejected_filter",
            "ready(); range_iter(0, 1000000000000) | filter(x => false) | collect(1)",
        ),
        ("blocking_host_20ms", "blocking_host()"),
    ];
    let functions = [
        Rc::new(HostFunction {
            name: "ready",
            parameters: vec![],
            result: ValueType::Null,
            callback: ready,
        }),
        Rc::new(HostFunction {
            name: "blocking_host",
            parameters: vec![],
            result: ValueType::Null,
            callback: blocking_host,
        }),
    ];
    println!("workload\tsamples\tmin_us\tmedian_us\tp95_us\tmax_us");
    for (name, source) in cases {
        let program = Program::compile(source).unwrap();
        let mut samples = Vec::new();
        for iteration in 0..27 {
            ENTERED.store(false, Ordering::Release);
            let cancellation = CancellationToken::default();
            let requester_token = cancellation.clone();
            let requester = thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !ENTERED.load(Ordering::Acquire) {
                    if Instant::now() >= deadline {
                        requester_token.cancel();
                        panic!("Runtime did not enter the workload within five seconds");
                    }
                    thread::yield_now();
                }
                // Let execution proceed before requesting cancellation.
                thread::sleep(Duration::from_millis(5));
                let sent = Instant::now();
                requester_token.cancel();
                sent
            });
            let result = program.run_with_host(100_000_000, &cancellation, &[], &functions);
            let returned = Instant::now();
            let sent = requester.join().unwrap();
            let error = result.expect_err("Workload must terminate by cancellation");
            assert_eq!(error.message, "Execution cancelled");
            assert!(
                returned >= sent,
                "Runtime returned before cancellation was requested"
            );
            if iteration >= 2 {
                samples.push(returned.duration_since(sent).as_secs_f64() * 1e6);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{name}\t25\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
            samples[0], samples[12], samples[23], samples[24]
        );
    }
}
