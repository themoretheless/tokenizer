use mlua::{Lua, VmState};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

fn run(install: impl Fn(&Lua, Arc<AtomicBool>), remove: impl Fn(&Lua)) {
    let scenario = std::env::var("RUSH_CANCEL_CASE").unwrap_or("loop".into());
    assert!(["loop", "blocking_host"].contains(&scenario.as_str()));
    let blocking = scenario == "blocking_host";
    println!("Scenario: {scenario}; blocking host sleeps 25ms when selected");
    if cfg!(debug_assertions) {
        eprintln!("Use --release for measurements");
        std::process::exit(2);
    }
    println!(
        "{}; 25 samples after 3 warmups; request 5ms after script entry",
        env!("CARGO_PKG_NAME")
    );
    println!("sample\trequest_to_return_us\treuse_result");
    for sample in 0..28 {
        let lua = Lua::new();
        let cancelled = Arc::new(AtomicBool::new(false));
        install(&lua, cancelled.clone());
        let (entered, ready) = mpsc::sync_channel(1);
        lua.globals()
            .set(
                "entered",
                lua.create_function(move |_, ()| {
                    entered.send(()).unwrap();
                    if blocking { std::thread::sleep(Duration::from_millis(25)); }
                    Ok(())
                })
                .unwrap(),
            )
            .unwrap();
        let program = lua
            .load("entered()\nwhile true do end")
            .into_function()
            .unwrap();
        let requested = Arc::new(Mutex::new(None));
        let sent = requested.clone();
        let requester = std::thread::spawn(move || {
            ready
                .recv_timeout(Duration::from_secs(5))
                .expect("Script did not enter");
            std::thread::sleep(Duration::from_millis(5));
            *sent.lock().unwrap() = Some(Instant::now());
            cancelled.store(true, Ordering::Release);
        });
        let result = program.call::<()>(());
        let returned = Instant::now();
        requester.join().unwrap();
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("requested cancellation")
        );
        let elapsed = returned.duration_since(requested.lock().unwrap().unwrap());
        remove(&lua);
        let value = lua.load("return 1+2").eval::<i64>().unwrap();
        assert_eq!(value, 3);
        if sample >= 3 {
            println!(
                "{}\t{:.3}\t{value}",
                sample - 3,
                elapsed.as_secs_f64() * 1e6
            );
        }
    }
}

fn check(flag: &AtomicBool) -> mlua::Result<VmState> {
    if flag.load(Ordering::Acquire) {
        Err(mlua::Error::RuntimeError("requested cancellation".into()))
    } else { Ok(VmState::Continue) }
}
