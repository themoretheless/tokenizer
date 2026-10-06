use rhai::{Dynamic, Engine};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
fn main() {
    if cfg!(debug_assertions) {
        eprintln!("Use --release for measurements");
        std::process::exit(2);
    }
    let scenario = std::env::var("RUSH_CANCEL_CASE").unwrap_or("loop".into());
    assert!(["loop", "blocking_host"].contains(&scenario.as_str()));
    let blocking = scenario == "blocking_host";
    println!(
        "Rhai 1.26.1; {scenario}; 25 samples after 3 warmups; request 5ms after entry; host sleep 25ms when selected"
    );
    println!("sample\trequest_to_return_us\treuse_result");
    for sample in 0..28 {
        let mut engine = Engine::new();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        engine.on_progress(move |_| {
            flag.load(Ordering::Acquire)
                .then(|| Dynamic::from("requested cancellation"))
        });
        let (entered, ready) = mpsc::sync_channel(1);
        engine.register_fn("entered", move || {
            entered.send(()).unwrap();
            if blocking {
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        let program = engine.compile("entered(); loop {}").unwrap();
        let requested = Arc::new(Mutex::new(None));
        let sent = requested.clone();
        let request_flag = cancelled.clone();
        let requester = std::thread::spawn(move || {
            ready
                .recv_timeout(Duration::from_secs(5))
                .expect("Script did not enter");
            std::thread::sleep(Duration::from_millis(5));
            *sent.lock().unwrap() = Some(Instant::now());
            request_flag.store(true, Ordering::Release);
        });
        let error = engine.eval_ast::<()>(&program).unwrap_err();
        let returned = Instant::now();
        requester.join().unwrap();
        match *error {
            rhai::EvalAltResult::ErrorTerminated(value, _) => {
                assert_eq!(value.into_string().unwrap(), "requested cancellation");
            }
            other => panic!("Unexpected error: {other}"),
        }
        let elapsed = returned.duration_since(requested.lock().unwrap().unwrap());
        cancelled.store(false, Ordering::Release);
        assert_eq!(engine.eval::<rhai::INT>("1+2").unwrap(), 3);
        if sample >= 3 {
            println!("{}\t{:.3}\t3", sample - 3, elapsed.as_secs_f64() * 1e6);
        }
    }
}
