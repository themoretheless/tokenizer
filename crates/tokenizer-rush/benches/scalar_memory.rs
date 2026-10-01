//! Requested heap bytes; separate binary so allocator instrumentation cannot distort timing.
#[path = "support/scalar.rs"]
mod scalar;
use scalar::{Scalar, Tree};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use themoretheless_tokenizer_rush::{CancellationToken, HostFunction, Value, ValueType};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
struct Counter;
fn allocated(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
    CALLS.fetch_add(1, Relaxed);
}
// SAFETY: delegates all pointers and layouts to System; atomics never allocate.
unsafe impl GlobalAlloc for Counter {
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
static ALLOCATOR: Counter = Counter;

fn measure<T>(source: &str, name: &str, run: usize, operation: impl FnOnce() -> T) {
    let before = LIVE.load(Relaxed);
    PEAK.store(before, Relaxed);
    let calls = CALLS.load(Relaxed);
    let result = black_box(operation());
    let retained = LIVE.load(Relaxed) - before;
    let peak = PEAK.load(Relaxed) - before;
    let calls = CALLS.load(Relaxed) - calls;
    drop(result);
    let after = LIVE.load(Relaxed);
    assert_eq!(after, before, "Allocation retained after drop: {name}");
    println!("{source}\t{name}\t{run}\t{peak}\t{retained}\t{calls}\t0");
}
fn main() {
    if cfg!(debug_assertions) {
        println!("Use optimized bench profile");
        return;
    }
    let baseline = LIVE.load(Relaxed);
    let probe = black_box(vec![0u8; 4096]);
    assert_eq!(LIVE.load(Relaxed) - baseline, 4096);
    drop(probe);
    assert_eq!(LIVE.load(Relaxed), baseline);
    println!(
        "Requested heap bytes; stack/allocator overhead/RSS excluded; realloc counts as one call"
    );
    println!(
        "source\toperation\trun\tpeak_extra_bytes\tretained_result_bytes\tallocation_calls\tafter_drop_bytes"
    );
    let token = CancellationToken::default();
    for source in [
        "x*2",
        "(if x%2==0 { x*2 } else { x })",
        "x%3==0 and x<15000",
    ] {
        let tree = Tree::compile(source, &["x"]).unwrap();
        let vm = tree.bytecode();
        let mut stack = vm.stack();
        let args = [Scalar::Number(12.0)];
        let expected = tree.run(&args, 100, &token).unwrap();
        assert_eq!(vm.run(&args, 100, &token, &mut stack).unwrap(), expected);
        for run in 0..3 {
            measure(source, "parse_and_tree", run, || {
                Tree::compile(source, &["x"]).unwrap()
            });
            measure(source, "tree_to_instructions", run, || tree.bytecode());
            measure(source, "parse_and_instructions", run, || {
                Tree::compile(source, &["x"]).unwrap().bytecode()
            });
            measure(source, "vm_stack", run, || vm.stack());
            measure(source, "tree_run", run, || {
                for _ in 0..1000 {
                    assert_eq!(
                        black_box(tree.run(black_box(&args), 100, &token).unwrap()),
                        expected
                    );
                }
            });
            measure(source, "vm_run_reused_stack", run, || {
                for _ in 0..1000 {
                    assert_eq!(
                        black_box(vm.run(black_box(&args), 100, &token, &mut stack).unwrap()),
                        expected
                    );
                }
            });
        }
    }
    fn double<'s>(args: &[Value<'s>], _: &CancellationToken) -> Result<Value<'s>, String> {
        let Value::Number(n) = args[0] else {
            unreachable!()
        };
        Ok(Value::Number(n * 2.0))
    }
    let host = std::rc::Rc::new(HostFunction {
        name: "double",
        parameters: vec![ValueType::Number],
        result: ValueType::Number,
        callback: double,
    });
    let tree = Tree::compile_with_hosts("double(x)", &["x"], &[host]).unwrap();
    let vm = tree.bytecode();
    let mut stack = vm.stack();
    let args = [Scalar::Number(12.0)];
    for run in 0..3 {
        measure("double(x)", "tree_host_run", run, || {
            for _ in 0..1000 {
                assert_eq!(
                    black_box(tree.run(black_box(&args), 100, &token).unwrap()),
                    Scalar::Number(24.0)
                );
            }
        });
        measure("double(x)", "vm_host_run", run, || {
            for _ in 0..1000 {
                assert_eq!(
                    black_box(vm.run(black_box(&args), 100, &token, &mut stack).unwrap()),
                    Scalar::Number(24.0)
                );
            }
        });
    }
    let factory = scalar::closures::Factory::compile("factor => x => x*factor").unwrap();
    let code_factory = factory.bytecode();
    for (name, factory) in [("tree_closure", &factory), ("vm_closure", &code_factory)] {
        let bound = factory.bind(&[Scalar::Number(3.0)]).unwrap();
        let mut stack = bound.stack();
        for run in 0..3 {
            measure(name, "bind", run, || {
                factory.bind(&[Scalar::Number(3.0)]).unwrap()
            });
            measure(name, "prepared_closure_run", run, || {
                for _ in 0..1000 {
                    assert_eq!(
                        black_box(
                            bound
                                .run(black_box(&[Scalar::Number(7.0)]), 100, &token, &mut stack)
                                .unwrap()
                        ),
                        Scalar::Number(21.0)
                    );
                }
            });
            measure(name, "create_and_call", run, || {
                for _ in 0..1000 {
                    let closure = factory.bind(black_box(&[Scalar::Number(3.0)])).unwrap();
                    assert_eq!(
                        black_box(
                            closure
                                .run(black_box(&[Scalar::Number(7.0)]), 100, &token, &mut stack)
                                .unwrap()
                        ),
                        Scalar::Number(21.0)
                    );
                }
            });
        }
    }
}
