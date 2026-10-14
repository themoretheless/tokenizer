//! Performance benchmark comparing AST interpreter vs Bytecode VM.
//! Run with: cargo bench -p themoretheless-tokenizer-rush --bench bytecode_vm
use std::{hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{BytecodeProgram, evaluate};

fn measure<T>(iterations: usize, mut f: impl FnMut() -> T) -> f64 {
    for _ in 0..5 {
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
    samples[3] // median in microseconds
}

fn bench_case(name: &str, source: &str, budget: usize, iters: usize) {
    let ast_ast_time = measure(iters, || {
        evaluate(source, budget).expect("ast run succeeds")
    });

    let bc_prog = BytecodeProgram::compile(source).expect("bytecode compile succeeds");
    let bc_exec_time = measure(iters, || {
        bc_prog.execute(budget).expect("bytecode run succeeds")
    });

    let speedup = ast_ast_time / bc_exec_time;
    println!(
        "{:<35} | AST: {:>9.2} µs | Bytecode VM: {:>9.2} µs | Speedup: {:>6.2}x",
        name, ast_ast_time, bc_exec_time, speedup
    );
}

fn main() {
    if cfg!(debug_assertions) {
        println!(
            "Note: running in debug profile. For accurate results run with: cargo bench --bench bytecode_vm"
        );
    }

    println!("\n=== Rush Performance: AST Interpreter vs Bytecode VM ===");
    println!("{:-<95}", "");

    // 1. Recursive Fibonacci
    let fib_source = r#"
fn fib(n) {
    if n <= 1 { return n }
    return fib(n - 1) + fib(n - 2)
}
fib(18)
"#;
    bench_case(
        "1. Recursive Fibonacci (fib(18))",
        fib_source,
        1_000_000,
        20,
    );

    // 2. For loop with mutable accumulation
    let loop_source = r#"
mut sum = 0
for i in range_iter(0, 2000) {
    sum += i
}
sum
"#;
    bench_case(
        "2. For-loop range_iter (2000 items)",
        loop_source,
        500_000,
        50,
    );

    // 3. Lazy functional sequence pipeline
    let pipeline_source = r#"
range_iter(0, 2000) | map(x => x * 2) | filter(x => x % 3 == 0) | fold(0, (sum, x) => sum + x)
"#;
    bench_case(
        "3. Lazy pipeline map/filter/fold",
        pipeline_source,
        500_000,
        50,
    );

    // 4. Nested loops
    let nested_source = r#"
mut count = 0
for i in range_iter(0, 60) {
    for j in range_iter(0, 60) {
        count += 1
    }
}
count
"#;
    bench_case(
        "4. Nested loops (60x60 = 3600 iters)",
        nested_source,
        500_000,
        40,
    );

    // 5. Arithmetic computation with local variables
    let math_source = r#"
mut acc = 1.0
for i in range_iter(1, 1000) {
    let term = (i * 3 + 7) % 17
    acc = (acc * 1.01 + term) % 1000.0
}
acc
"#;
    bench_case("5. Arithmetic locals calculation", math_source, 500_000, 50);

    println!("{:-<95}\n", "");
}
