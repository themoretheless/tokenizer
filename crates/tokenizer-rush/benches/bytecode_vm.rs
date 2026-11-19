//! Performance benchmark comparing AST interpreter vs Bytecode VM.
//! Run with: cargo bench -p themoretheless-tokenizer-rush --bench bytecode_vm
//! On Unix, set RUSH_BENCH_CPU_TIME=1 to exclude scheduler wait time.
use std::{hint::black_box, time::Instant};
use themoretheless_tokenizer_rush::{BytecodeProgram, CancellationToken, Program};

#[cfg(unix)]
fn thread_cpu_time() -> std::time::Duration {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // clock_gettime initializes the entire timespec on success.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, time.as_mut_ptr()) },
        0
    );
    let time = unsafe { time.assume_init() };
    std::time::Duration::new(time.tv_sec as u64, time.tv_nsec as u32)
}

fn measure<T>(iterations: usize, mut f: impl FnMut() -> T) -> f64 {
    for _ in 0..5 {
        black_box(f());
    }
    let mut samples = Vec::new();
    #[cfg(unix)]
    let cpu_time = std::env::var_os("RUSH_BENCH_CPU_TIME").is_some();
    for _ in 0..7 {
        let start = Instant::now();
        #[cfg(unix)]
        let cpu_start = cpu_time.then(thread_cpu_time);
        for _ in 0..iterations {
            black_box(f());
        }
        let elapsed = start.elapsed();
        #[cfg(unix)]
        let elapsed = cpu_start.map_or(elapsed, |start| thread_cpu_time() - start);
        samples.push(elapsed.as_secs_f64() * 1e6 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[3] // median in microseconds
}

fn bench_case(name: &str, source: &str, budget: usize, iters: usize) {
    let ast = Program::compile(source).expect("AST compile succeeds");
    let bytecode = BytecodeProgram::compile(source).expect("bytecode compile succeeds");
    let cancel = CancellationToken::default();
    let run_ast = || ast.run(budget, &cancel, &[]).expect("AST run succeeds");
    let run_vm = || bytecode.execute(budget).expect("bytecode run succeeds");
    assert_eq!(run_ast(), run_vm(), "{name}");

    let ast_prepare = measure(iters, || Program::compile(black_box(source)).unwrap());
    let vm_prepare = measure(iters, || {
        BytecodeProgram::compile(black_box(source)).unwrap()
    });
    let ast_execute = measure(iters, run_ast);
    let vm_execute = measure(iters, run_vm);
    let ast_total = measure(iters, || {
        Program::compile(black_box(source))
            .unwrap()
            .run(budget, &cancel, &[])
            .unwrap()
    });
    let vm_total = measure(iters, || {
        BytecodeProgram::compile(black_box(source))
            .unwrap()
            .execute(budget)
            .unwrap()
    });
    println!(
        "{name}\t{ast_prepare:.3}\t{vm_prepare:.3}\t{ast_execute:.3}\t{vm_execute:.3}\t{ast_total:.3}\t{vm_total:.3}",
    );
}

fn main() {
    if cfg!(debug_assertions) {
        println!(
            "Note: running in debug profile. For accurate results run with: cargo bench --bench bytecode_vm"
        );
    }

    println!(
        "scenario\tAST prepare (µs)\tVM prepare (µs)\tAST execute (µs)\tVM execute (µs)\tAST total (µs)\tVM total (µs)"
    );
    bench_case("Addition", "2 + 3", 1000, 2000);
    // Larger bytecode, but a short execution path through it.
    let cold_branch = format!("if false {{ {} }}; 42", "2 + 3; ".repeat(256));
    bench_case("Cold branch (256 expressions)", &cold_branch, 500_000, 100);

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
}
