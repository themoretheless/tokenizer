//! Run with `cargo bench --bench json_bench`.
//!
//! Set `TOKENIZER_BENCH_ITERS` to a positive integer for a short smoke run or
//! to force identical iteration counts across datasets.

use std::{env, hint::black_box, time::Instant};

use themoretheless_tokenizer::json::{lex, parse, tokenize};

const DEFAULT_INPUT_BYTES_PER_BENCHMARK: usize = 16 * 1024 * 1024;
const MINIMUM_ITERATIONS: usize = 100;
const MAXIMUM_ITERATIONS: usize = 100_000;

struct Dataset {
    name: &'static str,
    source: String,
    valid: bool,
}

fn main() {
    if cfg!(debug_assertions) {
        println!("benchmark skipped outside the optimized bench profile");
        return;
    }

    let datasets = datasets();
    validate_datasets(&datasets);
    let iteration_override = env::var("TOKENIZER_BENCH_ITERS")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .expect("TOKENIZER_BENCH_ITERS must be a positive integer")
        })
        .inspect(|iterations| assert!(*iterations > 0, "iteration count must be positive"));

    println!("JSON benchmark (lower ns/op is better)");
    println!("serde_json_parse is a reference DOM API with different ownership and AST semantics");
    println!("dataset\tbytes\toperation\titerations\tns/op\tMiB/s");

    for dataset in &datasets {
        let iterations = iteration_override.unwrap_or_else(|| {
            (DEFAULT_INPUT_BYTES_PER_BENCHMARK / dataset.source.len().max(1))
                .clamp(MINIMUM_ITERATIONS, MAXIMUM_ITERATIONS)
        });
        let source = dataset.source.as_str();

        measure(dataset, "lex", iterations, || lex(black_box(source)));
        measure(dataset, "parse", iterations, || parse(black_box(source)));
        measure(dataset, "tokenize", iterations, || {
            tokenize(black_box(source))
        });

        if dataset.valid {
            measure(dataset, "serde_json_parse", iterations, || {
                serde_json::from_str::<serde_json::Value>(black_box(source))
                    .expect("valid benchmark dataset")
            });
        }
    }
}

fn validate_datasets(datasets: &[Dataset]) {
    for dataset in datasets {
        let ours = parse(&dataset.source);
        assert_eq!(
            ours.is_valid(),
            dataset.valid,
            "unexpected parser validity for benchmark dataset {}",
            dataset.name
        );
        if dataset.valid {
            serde_json::from_str::<serde_json::Value>(&dataset.source).unwrap_or_else(|error| {
                panic!(
                    "serde_json rejected valid benchmark dataset {}: {error}",
                    dataset.name
                )
            });
        }
    }
}

fn measure<T>(
    dataset: &Dataset,
    operation: &str,
    iterations: usize,
    mut benchmark: impl FnMut() -> T,
) {
    for _ in 0..iterations.min(16) {
        black_box(benchmark());
    }

    let start = Instant::now();
    for _ in 0..iterations {
        black_box(benchmark());
    }
    let elapsed = start.elapsed();
    let nanoseconds_per_iteration = elapsed.as_secs_f64() * 1_000_000_000.0 / iterations as f64;
    let mebibytes_per_second =
        dataset.source.len() as f64 * iterations as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64();

    println!(
        "{}\t{}\t{operation}\t{iterations}\t{nanoseconds_per_iteration:.1}\t{mebibytes_per_second:.1}",
        dataset.name,
        dataset.source.len()
    );
}

fn datasets() -> Vec<Dataset> {
    vec![
        Dataset {
            name: "small",
            source: r#"{"name":"tokenizer","version":2,"ready":true,"tags":["json","editor"]}"#
                .to_owned(),
            valid: true,
        },
        Dataset {
            name: "unicode",
            source: unicode_dataset(),
            valid: true,
        },
        Dataset {
            name: "large",
            source: large_dataset(),
            valid: true,
        },
        Dataset {
            name: "deep",
            source: format!("{}null{}", "[".repeat(96), "]".repeat(96)),
            valid: true,
        },
        Dataset {
            name: "malformed",
            source: malformed_dataset(),
            valid: false,
        },
    ]
}

fn unicode_dataset() -> String {
    let values = (0..256)
        .map(|index| format!(r#"{{"id":{index},"text":"Привет 世界 😀 café e\u0301"}}"#))
        .collect::<Vec<_>>();
    format!("[{}]", values.join(","))
}

fn large_dataset() -> String {
    let values = (0..2_048)
        .map(|index| {
            format!(
                r#"{{"id":{index},"enabled":{},"score":{}.25,"name":"item-{index}"}}"#,
                index % 2 == 0,
                index * 17
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", values.join(","))
}

fn malformed_dataset() -> String {
    let values = (0..512)
        .map(|index| {
            format!(r#"{{"id":{index},"missing":,"bad-number":01,"unterminated":"value}}"#)
        })
        .collect::<Vec<_>>();
    format!("[{}, trailing garbage", values.join(","))
}
