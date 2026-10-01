use koto::prelude::*;
use std::time::{Duration, Instant};
fn main() {
    if cfg!(debug_assertions) {
        eprintln!("Use --release for measurements");
        std::process::exit(2);
    }
    let scenario = std::env::var("RUSH_CANCEL_CASE").unwrap_or("loop".into());
    assert!(["loop", "blocking_host"].contains(&scenario.as_str()));
    println!(
        "Koto 0.16.1; {scenario}; execution limit 5ms; 25 samples after 3 warmups; elapsed run time, not request latency"
    );
    println!("sample\trun_us\treuse_result");
    for sample in 0..28 {
        let settings = KotoSettings::default().with_execution_limit(Duration::from_millis(5));
        let mut koto = Koto::with_settings(settings);
        koto.prelude().add_fn("blocking", |_| {
            std::thread::sleep(Duration::from_millis(25));
            Ok(KValue::Null)
        });
        let source = if scenario == "blocking_host" {
            "blocking()\nwhile true\n  null"
        } else {
            "while true\n  null"
        };
        let program = koto.compile(source).unwrap();
        let start = Instant::now();
        let error = koto.run(program).unwrap_err();
        let elapsed = start.elapsed();
        assert!(error.to_string().contains("execution timed out"), "{error}");
        assert!(
            matches!(koto.compile_and_run("1+2").unwrap(), KValue::Number(n) if f64::from(n)==3.0)
        );
        if sample >= 3 {
            println!("{}\t{:.3}\t3", sample - 3, elapsed.as_secs_f64() * 1e6);
        }
    }
}
