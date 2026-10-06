//! Diagnostic macOS allocator counters; not a timing benchmark or runtime policy.
#[cfg(target_os = "macos")]
mod macos {
    use std::{ffi::c_void, hint::black_box};
    use themoretheless_tokenizer_rush::{CancellationToken, Program, Value};
    // Layout and null-zone semantics from the macOS SDK malloc/malloc.h.
    #[repr(C)]
    #[derive(Default)]
    struct Statistics {
        blocks_in_use: u32,
        size_in_use: usize,
        max_size_in_use: usize,
        size_allocated: usize,
    }
    // rusage_info_v0 from sys/resource.h (RUSAGE_INFO_V0 = 0).
    #[repr(C)]
    #[derive(Default)]
    struct Usage {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        package_wakeups: u64,
        interrupt_wakeups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        physical_footprint: u64,
        start_time: u64,
        exit_time: u64,
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut *mut c_void) -> i32;
        fn malloc_zone_statistics(zone: *mut c_void, statistics: *mut Statistics);
        fn malloc_zone_pressure_relief(zone: *mut c_void, goal: usize) -> usize;
    }
    fn report(run: usize, released: usize) {
        let mut stats = Statistics::default();
        // SAFETY: null selects all zones; stats matches the C ABI and is writable.
        unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut stats) };
        let mut usage = Usage::default();
        // SAFETY: flavor 0 writes a rusage_info_v0 into the caller's buffer.
        // libproc declares the opaque buffer as void**; it is not an out-pointer.
        let status = unsafe {
            proc_pid_rusage(
                std::process::id() as i32,
                0,
                (&mut usage as *mut Usage).cast(),
            )
        };
        assert_eq!(
            status,
            0,
            "proc_pid_rusage: {}",
            std::io::Error::last_os_error()
        );
        println!(
            "{run}\t{}\t{}\t{}\t{released}\t{}\t{}",
            stats.blocks_in_use,
            stats.size_in_use,
            stats.size_allocated,
            usage.resident_size,
            usage.physical_footprint
        );
    }
    pub fn run() {
        let relief = std::env::var("RUSH_BENCH_RELIEF").as_deref() == Ok("1");
        let lazy = std::env::var("RUSH_BENCH_LAZY").as_deref() == Ok("1");
        let range = if lazy { "range_iter" } else { "range" };
        let source = format!(
            "{range}(0,1000000) | map(x => x * 2) | filter(x => x % 3 == 0) | fold(0, (sum,x) => sum+x)"
        );
        let program = Program::compile(&source).unwrap();
        let cancellation = CancellationToken::default();
        println!("lazy={lazy}; relief={relief}; 1000000 items; 18 verified runs");
        for name in ["MallocNanoZone", "MallocLargeCache"] {
            println!(
                "{name}={}",
                std::env::var(name).unwrap_or_else(|_| "<unset>".into())
            );
        }
        println!(
            "run\tblocks_in_use\tbytes_in_use\tbytes_reserved\tbytes_released\tresident_bytes\tphysical_footprint_bytes"
        );
        report(0, 0);
        for run in 1..=18 {
            let result = program.run(100_100_000, &cancellation, &[]).unwrap();
            assert_eq!(result, Value::Number(333_333_666_666.));
            drop(black_box(result));
            let released = if relief {
                // SAFETY: null/zero asks all system allocator zones to release
                // unused reservations. Live allocations remain valid.
                unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) }
            } else {
                0
            };
            report(run, released);
        }
        if std::env::var("RUSH_BENCH_WAIT").as_deref() == Ok("1") {
            use std::io::Write;
            println!("READY_FOR_VMMAP {}", std::process::id());
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
        }
    }
}
fn main() {
    if cfg!(debug_assertions) {
        println!("Use the optimized bench profile");
        return;
    }
    #[cfg(target_os = "macos")]
    macos::run();
    #[cfg(not(target_os = "macos"))]
    println!("Allocator diagnostics require macOS; use the portable memory benchmark here");
}
