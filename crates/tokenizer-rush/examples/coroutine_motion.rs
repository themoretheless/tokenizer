//! Deterministic scheduler simulation; no wall-clock sleeps.
use std::time::Duration;
use themoretheless_tokenizer_rush::{
    CancellationToken, CoroutineScheduler, ExecutionLimits, Program, ScheduledState, Value,
    WakeRequest,
};
fn main() -> Result<(), themoretheless_tokenizer_rush::RuntimeError> {
    let program = Program::compile(include_str!("scripts/coroutine-motion.r"))?;
    let token = CancellationToken::default();
    let limits = ExecutionLimits::new(10_000);
    let mut script = program.instantiate(limits, &token, &[], &[], &[], &[])?;
    let mut scheduler = CoroutineScheduler::new(&mut script);
    scheduler.spawn("motion", &[], limits)?;
    let mut now = Duration::ZERO;
    while !scheduler.is_empty() {
        for step in scheduler.poll(now, limits)? {
            match step.state {
                ScheduledState::Waiting(WakeRequest::Event(name)) => {
                    println!("Host delivered event: {name}");
                    assert_eq!(scheduler.emit(&name)?, 1);
                }
                ScheduledState::Waiting(WakeRequest::After(_)) => {}
                ScheduledState::Complete(value) => {
                    assert_eq!(value, Value::Number(3.));
                    assert_eq!(now, Duration::from_millis(750));
                    println!("Completed: position = 3");
                }
                ScheduledState::Failed(error) => return Err(error),
            }
        }
        if let Some(deadline) = scheduler.next_deadline()
            && deadline > now
        {
            now = deadline;
            println!("Timer wake at simulated {:.2} seconds", now.as_secs_f64());
        }
    }
    Ok(())
}
