//! Host tests for the rush interpreter core: drive `step` over a script,
//! collecting `Run` segments and feeding synthetic statuses.

use rush_lang::{Interpreter, RushError, Step};

/// Run a whole script; `fail` lists 1-based command ordinals that fail.
/// Returns the executed segments.
fn run(script: &str, fail: &[u32]) -> (Vec<String>, Result<u32, RushError>) {
    let mut interp = Interpreter::new();
    interp.seed("USER", "admin");
    interp.seed("HOME", "/");
    interp.seed("STATUS", "0");
    let mut executed = Vec::new();
    let mut outcome = Ok(0);
    'outer: for line in script.lines().chain(core::iter::once("")) {
        loop {
            match interp.step(line) {
                Ok(Step::Skip) => break,
                Ok(Step::Run(segment)) => {
                    executed.push(segment.to_string());
                    let ordinal = executed.len() as u32;
                    interp.note_status(!fail.contains(&ordinal));
                    if !interp.chain_pending() {
                        break;
                    }
                }
                Err(error) => {
                    outcome = Err(error);
                    break 'outer;
                }
            }
        }
    }
    if outcome.is_ok() {
        outcome = interp.finish();
    }
    (executed, outcome)
}

fn ok(script: &str) -> Vec<String> {
    let (executed, outcome) = run(script, &[]);
    assert_eq!(outcome.map(|_| ()), Ok(()));
    executed
}

#[test]
fn variables_expand_and_status_tracks() {
    let executed = ok("who = $USER\necho hello $who\nnosuchcmd\necho $STATUS\n");
    assert_eq!(
        executed,
        vec![
            "echo hello $who",
            "nosuchcmd",
            "echo $STATUS",
        ]
    );
    // The driver resolves $who/$STATUS at parse time; the table holds them:
    let mut interp = Interpreter::new();
    interp.seed("USER", "admin");
    assert_eq!(interp.step("who = $USER"), Ok(Step::Skip));
    assert_eq!(interp.get("who"), Some("admin"));
}

#[test]
fn if_else_blocks_follow_the_condition() {
    let executed = ok(
        "if $STATUS = 0:\n    echo if-ok\nelse:\n    echo else-bad\necho after\n",
    );
    assert_eq!(executed, vec!["echo if-ok", "echo after"]);

    let (executed, outcome) = run("badcmd\nif $STATUS = 0:\n    echo no\nelse:\n    echo yes\n", &[1]);
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(executed, vec!["badcmd", "echo yes"]);
}

#[test]
fn nested_blocks_and_not() {
    let executed = ok(
        "if not $MISSING:\n    if 2 > 1:\n        echo deep\n    echo mid\necho top\n",
    );
    assert_eq!(executed, vec!["echo deep", "echo mid", "echo top"]);
}

#[test]
fn and_or_short_circuit() {
    // All succeed: every segment runs.
    assert_eq!(
        ok("echo a and echo b or echo c\n"),
        vec!["echo a", "echo b"]
    );
    // First fails: `and` drops b, `or` runs c.
    let (executed, _) = run("echo a and echo b or echo c\n", &[1]);
    assert_eq!(executed, vec!["echo a", "echo c"]);
    // && / || shorthand.
    let (executed, _) = run("echo a && echo b || echo c\n", &[1]);
    assert_eq!(executed, vec!["echo a", "echo c"]);
}

#[test]
fn stable_errors() {
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("else:"), Err(RushError::ElseWithoutIf));
    assert_eq!(interp.step("if $STATUS 0:"), Err(RushError::BadCondition));

    // Blocks close implicitly at EOF (Python-style).
    let (executed, outcome) = run("if $STATUS = 0:\n    echo open\n", &[]);
    assert_eq!(outcome, Ok(1));
    assert_eq!(executed, vec!["echo open"]);

    let deep = "if 1:\n  if 1:\n    if 1:\n      if 1:\n        if 1:\n          if 1:\n            if 1:\n              if 1:\n                if 1:\n                  echo x\n";
    let (_, outcome) = run(deep, &[]);
    assert_eq!(outcome, Err(RushError::TooDeep));
}

#[test]
fn comments_and_blanks_are_free() {
    assert_eq!(
        ok("# header\n\n   # indented comment\necho only\n"),
        vec!["echo only"]
    );
}

#[test]
fn quoted_operators_do_not_split() {
    assert_eq!(
        ok("echo 'a and b'\necho \"x || y\"\n"),
        vec!["echo 'a and b'", "echo \"x || y\""]
    );
}
