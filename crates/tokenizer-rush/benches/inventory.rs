//! Memory inventory probes: unaccounted allocation paths (AST, tokens,
//! transient evaluation buffers) measured with the same counting allocator
//! style as benches/memory.rs. Not a timing benchmark.
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use themoretheless_tokenizer_rush::{CancellationToken, Expr, ExprKind, InterpolationPart, Module, Program, Stmt, StmtKind, Value};

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static CUMULATIVE: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
    ALLOCATIONS.fetch_add(1, Relaxed);
    CUMULATIVE.fetch_add(size, Relaxed);
}

// SAFETY: All allocation operations delegate unchanged pointers/layouts to System.
// Counters allocate no memory and do not alter allocation ownership or alignment.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            allocated(size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count_expr(expr: &Expr<'_>, nodes: &mut usize) {
    *nodes += 1;
    match &expr.kind {
        ExprKind::Try(inner) => count_expr(inner, nodes),
        ExprKind::If {
            condition,
            then_value,
            else_value,
        } => {
            count_expr(condition, nodes);
            count_expr(then_value, nodes);
            count_expr(else_value, nodes);
        }
        ExprKind::Lambda { parameters, body } => {
            for parameter in parameters {
                count_expr(parameter, nodes);
            }
            count_expr(body, nodes);
        }
        ExprKind::Unary { value, .. } => count_expr(value, nodes),
        ExprKind::Binary { left, right, .. } | ExprKind::Assign {
            target: left,
            value: right,
            ..
        } => {
            count_expr(left, nodes);
            count_expr(right, nodes);
        }
        ExprKind::Call { callee, arguments } => {
            count_expr(callee, nodes);
            for argument in arguments {
                count_expr(argument, nodes);
            }
        }
        ExprKind::Member { object, .. } => count_expr(object, nodes),
        ExprKind::Index { object, index } => {
            count_expr(object, nodes);
            count_expr(index, nodes);
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                count_expr(item, nodes);
            }
        }
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                count_expr(key, nodes);
                count_expr(value, nodes);
            }
        }
        ExprKind::Pipeline { input, stages } => {
            count_expr(input, nodes);
            for stage in stages {
                count_expr(stage, nodes);
            }
        }
        ExprKind::Match { value, arms } => {
            count_expr(value, nodes);
            for arm in arms {
                *nodes += 1;
                count_expr(&arm.pattern, nodes);
                if let Some(guard) = &arm.guard {
                    count_expr(guard, nodes);
                }
                count_expr(&arm.value, nodes);
            }
        }
        ExprKind::Interpolate(parts) => {
            for part in parts {
                if let InterpolationPart::Expr(expr) = part {
                    count_expr(expr, nodes);
                }
            }
        }
        _ => {}
    }
}

fn count_stmt(stmt: &Stmt<'_>, nodes: &mut usize) {
    *nodes += 1;
    match &stmt.kind {
        StmtKind::Show(expr) | StmtKind::Yield(expr) | StmtKind::Expr(expr) => {
            count_expr(expr, nodes)
        }
        StmtKind::Destructure { pattern, value } => {
            count_expr(pattern, nodes);
            count_expr(value, nodes);
        }
        StmtKind::Function {
            parameters, body, ..
        } => {
            for parameter in parameters {
                count_expr(&parameter.pattern, nodes);
            }
            for statement in body.stmts.iter() {
                count_stmt(statement, nodes);
            }
        }
        StmtKind::Declaration { value, .. } => count_expr(value, nodes),
        StmtKind::Return(value) => {
            if let Some(value) = value {
                count_expr(value, nodes);
            }
        }
        StmtKind::If {
            condition,
            then_block,
            else_block,
        } => {
            count_expr(condition, nodes);
            for statement in then_block.stmts.iter() {
                count_stmt(statement, nodes);
            }
            if let Some(else_block) = else_block {
                for statement in else_block.stmts.iter() {
                    count_stmt(statement, nodes);
                }
            }
        }
        StmtKind::While { condition, body } => {
            count_expr(condition, nodes);
            for statement in body.stmts.iter() {
                count_stmt(statement, nodes);
            }
        }
        StmtKind::For {
            iterable, body, ..
        } => {
            count_expr(iterable, nodes);
            for statement in body.stmts.iter() {
                count_stmt(statement, nodes);
            }
        }
        StmtKind::Region { budget, body, .. } => {
            if let Some(budget) = budget {
                count_expr(budget, nodes);
            }
            for statement in body.stmts.iter() {
                count_stmt(statement, nodes);
            }
        }
        _ => {}
    }
}

fn count_nodes(module: &Module<'_>) -> usize {
    let mut nodes = 0;
    for statement in &module.items {
        count_stmt(statement, &mut nodes);
    }
    nodes
}

fn ast_probe() {
    println!("# AST probe: retained parse output vs source size");
    println!("file\tsource_bytes\ttokens\tast_nodes\tretained_parse_bytes\tpeak_extra_bytes\tallocation_calls\tretained_per_source_byte\tnodes_per_100_source_bytes");
    let mut generated = String::from("fn f(x) { return x * 2 + 1 }\n");
    for index in 0..2000_usize {
        generated.push_str(&format!(
            "let v{index} = f({index}) + f(v{prev}) / 2\n",
            prev = index.saturating_sub(1)
        ));
    }
    for (label, source) in [
        ("examples/scripts/flower.r".to_string(), None),
        ("examples/scripts/spiral.r".to_string(), None),
        ("examples/scripts/coroutine-motion.r".to_string(), None),
        ("<generated 2000 declarations>".to_string(), Some(generated)),
    ] {
        let source = match source {
            Some(source) => source,
            None => std::fs::read_to_string(&label).expect("read example script"),
        };
        let baseline = LIVE.load(Relaxed);
        PEAK.store(baseline, Relaxed);
        let allocations = ALLOCATIONS.load(Relaxed);
        let parsed = themoretheless_tokenizer_rush::parse(&source);
        let nodes = count_nodes(&parsed.module);
        let retained = LIVE.load(Relaxed) - baseline;
        let peak = PEAK.load(Relaxed) - baseline;
        let allocations = ALLOCATIONS.load(Relaxed) - allocations;
        println!(
            "{label}\t{}\t{}\t{nodes}\t{retained}\t{peak}\t{allocations}\t{:.2}\t{:.1}",
            source.len(),
            parsed.lexed.tokens.len(),
            retained as f64 / source.len() as f64,
            nodes as f64 * 100.0 / source.len() as f64,
        );
        drop(black_box(parsed));
        assert_eq!(LIVE.load(Relaxed), baseline, "Parse retained heap after drop");
    }
}

fn concat_probe() {
    println!("# String concatenation probe: quadratic transient work, 1000-byte live result");
    println!("workload\titerations\trun\tpeak_extra_bytes\tcumulative_alloc_bytes\tlive_result_bytes\tallocation_calls");
    for items in [100_usize, 1000, 10_000] {
        let source = format!(
            "mut s = ''; for i in range_iter(0,{items}) {{ s = s + 'x' }}; s"
        );
        let program = Program::compile(&source).unwrap();
        let cancellation = CancellationToken::default();
        for run in 1..=3 {
            let baseline = LIVE.load(Relaxed);
            PEAK.store(baseline, Relaxed);
            let allocations = ALLOCATIONS.load(Relaxed);
            let cumulative = CUMULATIVE.load(Relaxed);
            let result = program
                .run(2 * items * items + 1_000_000, &cancellation, &[])
                .unwrap();
            let live_result = LIVE.load(Relaxed) - baseline;
            assert!(matches!(&result, Value::String(text) if text.len() == items));
            drop(black_box(result));
            let retained = LIVE.load(Relaxed) as i128 - baseline as i128;
            assert_eq!(retained, 0, "Concat probe retained heap after drop");
            let peak = PEAK.load(Relaxed) - baseline;
            let cumulative = CUMULATIVE.load(Relaxed) - cumulative;
            let allocations = ALLOCATIONS.load(Relaxed) - allocations;
            println!("concat\t{items}\t{run}\t{peak}\t{cumulative}\t{live_result}\t{allocations}");
        }
    }
}

fn main() {
    if cfg!(debug_assertions) {
        println!("Use the optimized bench profile");
        return;
    }
    match std::env::var("RUSH_INVENTORY_CASE").ok().as_deref() {
        None | Some("ast") => ast_probe(),
        Some("concat") => concat_probe(),
        Some(_) => panic!("Unknown RUSH_INVENTORY_CASE"),
    }
    if std::env::var("RUSH_INVENTORY_CASE").is_err() {
        concat_probe();
    }
}
