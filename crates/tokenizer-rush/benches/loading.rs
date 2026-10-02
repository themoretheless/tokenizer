//! cargo bench -p themoretheless-tokenizer-rush --bench loading
//! Validates each workload before measuring fresh instances of prepared programs.
use std::{hint::black_box, rc::Rc, time::Instant};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, HostFunction, Program, Value, ValueType,
};
const LIMITS: ExecutionLimits = ExecutionLimits::new(100_000);
fn measure(name: &str, iterations: usize, mut f: impl FnMut()) {
    for _ in 0..10 {
        f();
    }
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..iterations {
            f();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name}\t{iterations}\t{:.3}\t{:.3}\t{:.3}",
        samples[0], samples[3], samples[6]
    );
}
fn double<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
    let Value::Number(n) = args[0] else {
        unreachable!()
    };
    Ok(Value::Number(n * 2.))
}
fn main() {
    if cfg!(debug_assertions) {
        println!("Use optimized bench profile");
        return;
    }
    let iterations = std::env::var("RUSH_BENCH_ITERS")
        .ok()
        .map(|s| s.parse().unwrap())
        .unwrap_or(100);
    assert!(iterations > 0);
    let selected = std::env::var("RUSH_LOADING_CASE").ok();
    let token = CancellationToken::default();
    println!(
        "loading · {} {} · 7 samples, 10 warmups; microseconds per fresh instance",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("case\titerations/sample\tmin_us\tmedian_us\tmax_us");
    for host in [false, true] {
        let name = if host { "host" } else { "plain" };
        if selected.as_deref().is_some_and(|s| s != name) {
            continue;
        }
        let source = if host {
            "fn work()->number{return host_double(21)}; work()"
        } else {
            "fn work(n:number)->number{return n+1}; work(41)"
        };
        let program = Program::compile(source).unwrap();
        let hosts = if host {
            vec![Rc::new(HostFunction {
                name: "host_double",
                parameters: vec![ValueType::Number],
                result: ValueType::Number,
                callback: double,
            })]
        } else {
            vec![]
        };
        let mut instance = program
            .instantiate(LIMITS, &token, &[], &hosts, &[], &[])
            .unwrap();
        assert_eq!(instance.initial_value(), &Value::Number(42.));
        assert_eq!(
            instance
                .call(
                    "work",
                    if host { &[] } else { &[Value::Number(41.)] },
                    LIMITS
                )
                .unwrap(),
            Value::Number(42.)
        );
        drop(instance);
        measure(name, iterations, || {
            black_box(
                program
                    .instantiate(LIMITS, &token, &[], &hosts, &[], &[])
                    .unwrap(),
            );
        });
    }
    for count in [1, 8, 24] {
        let name = format!("modules_{count}");
        if selected.as_deref().is_some_and(|s| s != name) {
            continue;
        }
        let names: Vec<_> = (0..count).map(|i| format!("m{i}")).collect();
        let sources: Vec<_> = (0..count)
            .map(|i| {
                if i == 0 {
                    "struct Point{x:number}; fn value()->number{return 42}; export Point,value"
                        .into()
                } else {
                    format!(
                        "import m{}; fn value()->number{{return 42}}; export value",
                        i - 1
                    )
                }
            })
            .collect();
        let programs: Vec<_> = sources
            .iter()
            .map(|s| Program::compile(s).unwrap())
            .collect();
        let modules: Vec<_> = names
            .iter()
            .zip(&programs)
            .map(|(n, p)| (n.as_str(), p))
            .collect();
        let source = format!("import m{}; m{}.value()", count - 1, count - 1);
        let program = Program::compile(&source).unwrap();
        assert_eq!(
            program
                .instantiate(LIMITS, &token, &[], &[], &[], &modules)
                .unwrap()
                .initial_value(),
            &Value::Number(42.)
        );
        measure(&name, iterations, || {
            black_box(
                program
                    .instantiate(LIMITS, &token, &[], &[], &[], &modules)
                    .unwrap(),
            );
        });
    }
}
