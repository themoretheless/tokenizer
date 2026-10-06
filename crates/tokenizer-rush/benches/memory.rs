//! Requested heap bytes, measured separately to avoid distorting timing benchmarks.
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};

struct CountingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

fn allocated(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
    ALLOCATIONS.fetch_add(1, Relaxed);
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

fn main() {
    if cfg!(debug_assertions) {
        println!("Use the optimized bench profile");
        return;
    }
    // Calibrate live-byte accounting before relying on runtime deltas.
    let baseline = LIVE.load(Relaxed);
    let mut probe = Vec::with_capacity(4096);
    probe.resize(4096, 1_u8);
    assert_eq!(LIVE.load(Relaxed) - baseline, probe.capacity());
    probe.reserve(8192);
    assert_eq!(LIVE.load(Relaxed) - baseline, probe.capacity());
    drop(black_box(probe));
    assert_eq!(LIVE.load(Relaxed), baseline);

    println!("workload\titems\trun\tpeak_extra_bytes\tlive_after_drop_bytes\tallocation_calls");
    match std::env::var("RUSH_MEMORY_CASE").ok().as_deref() {
        Some("grouping") => {
            grouping_memory();
            return;
        }
        Some("cells") => {
            cell_memory();
            return;
        }
        None => {}
        Some(_) => panic!("Unknown RUSH_MEMORY_CASE"),
    }
    let selected_items = std::env::var("RUSH_MEMORY_ITEMS").ok().map(|value| {
        let items = value.parse::<usize>().expect("Integer RUSH_MEMORY_ITEMS");
        assert!(
            [1000, 100_000, 1_000_000].contains(&items),
            "Unsupported RUSH_MEMORY_ITEMS"
        );
        items
    });
    for items in [1000_usize, 100_000, 1_000_000] {
        if selected_items.is_some_and(|selected| selected != items) {
            continue;
        }
        let expected: u64 = (0..items as u64)
            .map(|x| x * 2)
            .filter(|x| x % 3 == 0)
            .sum();
        for (name, range) in [("eager", "range"), ("lazy", "range_iter")] {
            let source = format!(
                "{range}(0,{items}) | map(x => x * 2) | filter(x => x % 3 == 0) | fold(0, (sum,x) => sum + x)"
            );
            let program = Program::compile(&source).unwrap();
            let cancellation = CancellationToken::default();
            for run in 1..=3 {
                let baseline = LIVE.load(Relaxed);
                PEAK.store(baseline, Relaxed);
                let allocations = ALLOCATIONS.load(Relaxed);
                let result = program
                    .run(items * 100 + 100_000, &cancellation, &[])
                    .unwrap();
                assert_eq!(result, Value::Number(expected as f64));
                drop(black_box(result));
                let retained = LIVE.load(Relaxed) as i128 - baseline as i128;
                let peak = PEAK.load(Relaxed) - baseline;
                let allocations = ALLOCATIONS.load(Relaxed) - allocations;
                println!("{name}\t{items}\t{run}\t{peak}\t{retained}\t{allocations}");
                assert_eq!(
                    retained, 0,
                    "Runtime retained heap allocations after the result was dropped"
                );
            }
        }
    }
}

fn cell_memory() {
    for items in [100_usize, 1000, 10_000] {
        for (name, prefix, body, suffix, extra) in [
            (
                "local_cells",
                "",
                "mut local = range(0,128); total += len(local)",
                "total",
                0,
            ),
            (
                "call_cells",
                "fn work() { mut local = range(0,128); return len(local) };",
                "total += work()",
                "total",
                0,
            ),
            (
                "escaped_cell",
                "fn counter() { mut n = 0; return () => n += 1 }; let next = counter();",
                "mut local = range(0,128); total += len(local)",
                "total + next()",
                1,
            ),
            (
                "cyclic_cells",
                "fn work() { mut local = range(0,128); mut f = () => local; f = () => f; return len(local) };",
                "total += work()",
                "total",
                0,
            ),
        ] {
            let source = format!(
                "{prefix}mut total = 0; for i in range_iter(0,{items}) {{ {body} }}; {suffix}"
            );
            let program = Program::compile(&source).unwrap();
            let cancellation = CancellationToken::default();
            for run in 1..=3 {
                let baseline = LIVE.load(Relaxed);
                PEAK.store(baseline, Relaxed);
                let allocations = ALLOCATIONS.load(Relaxed);
                let result = program
                    .run(items * 1000 + 100_000, &cancellation, &[])
                    .unwrap();
                assert_eq!(result, Value::Number((items * 128 + extra) as f64));
                drop(black_box(result));
                let retained = LIVE.load(Relaxed) as i128 - baseline as i128;
                let peak = PEAK.load(Relaxed) - baseline;
                let allocations = ALLOCATIONS.load(Relaxed) - allocations;
                println!("{name}\t{items}\t{run}\t{peak}\t{retained}\t{allocations}");
                assert_eq!(retained, 0, "Cell workload retained allocations after run");
            }
        }
    }
}

fn grouping_memory() {
    use std::collections::BTreeMap;
    for items in [1000_usize, 100_000, 1_000_000] {
        let expected = Value::List(
            (0..2)
                .map(|parity| {
                    let sum: usize = (parity..items).step_by(2).sum();
                    Value::Record(BTreeMap::from([
                        (
                            "key".into(),
                            Value::String(if parity == 0 { "even" } else { "odd" }.into()),
                        ),
                        ("value".into(), Value::Number(sum as f64)),
                    ]))
                })
                .collect(),
        );
        for name in ["group_by", "fold_by"] {
            // Keep the buffering baseline bounded; the streaming case also runs at 1M.
            if name == "group_by" && items > 100_000 {
                continue;
            }
            let operation = if name == "group_by" {
                "group_by(x => if x % 2 == 0 { 'even' } else { 'odd' }) | map(g => {key:g.key,value: g.values | fold(0,(sum,x) => sum+x)})"
            } else {
                "fold_by(x => if x % 2 == 0 { 'even' } else { 'odd' }, 0, (sum,x) => sum+x)"
            };
            let source = format!("range_iter(0,{items}) | {operation}");
            let program = Program::compile(&source).unwrap();
            let cancellation = CancellationToken::default();
            for run in 1..=3 {
                let baseline = LIVE.load(Relaxed);
                PEAK.store(baseline, Relaxed);
                let allocations = ALLOCATIONS.load(Relaxed);
                let result = program
                    .run(items * 100 + 100_000, &cancellation, &[])
                    .unwrap();
                assert_eq!(result, expected);
                drop(black_box(result));
                let retained = LIVE.load(Relaxed) as i128 - baseline as i128;
                let peak = PEAK.load(Relaxed) - baseline;
                let allocations = ALLOCATIONS.load(Relaxed) - allocations;
                println!("{name}\t{items}\t{run}\t{peak}\t{retained}\t{allocations}");
                assert_eq!(
                    retained, 0,
                    "Grouping retained allocations after dropping result"
                );
            }
        }
    }
}
