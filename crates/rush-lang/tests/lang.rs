//! Host tests for the rush interpreter core: drive `step` over a script,
//! collecting `Run` segments and feeding synthetic statuses.

use rush_lang::{Interpreter, RushError, Step};

/// Run a whole script; `fail` lists 1-based command ordinals that fail.
/// Returns the executed segments.
fn run(script: &str, fail: &[u32]) -> (Vec<String>, Result<u32, RushError>) {
    run_with_sources(script, fail, &[])
}

/// `sources` maps a `foreach` source command to the words it yields.
fn run_with_sources(
    script: &str,
    fail: &[u32],
    sources: &[(&str, &str)],
) -> (Vec<String>, Result<u32, RushError>) {
    let mut interp = Interpreter::new();
    interp.seed("USER", "admin");
    interp.seed("HOME", "/");
    interp.seed("STATUS", "0");
    let mut executed = Vec::new();
    let mut outcome = Ok(0);
    let mut lines = script.lines();
    let mut ended = false;
    'outer: loop {
        // A `foreach` waits for its source words.
        if let Some((_, source)) = interp.source_request() {
            let source = source.to_string();
            match sources.iter().find(|(name, _)| *name == source) {
                Some((_, words)) => {
                    if let Err(error) = interp.provide_source_words(words) {
                        outcome = Err(error);
                        break;
                    }
                }
                None => interp.fail_source(),
            }
            continue;
        }
        // Replays (fn calls, loop iterations) and pending chains take
        // priority over fresh input; drain them with empty steps.
        let line = if interp.busy() {
            ""
        } else if ended {
            break;
        } else {
            match lines.next() {
                Some(line) => line,
                None => {
                    ended = true;
                    if let Err(error) = interp.end_input() {
                        outcome = Err(error);
                        break;
                    }
                    continue;
                }
            }
        };
        match interp.step(line) {
            Ok(Step::Skip) => {}
            Ok(Step::Run(segment)) => {
                executed.push(segment.to_string());
                let ordinal = executed.len() as u32;
                interp.note_status(!fail.contains(&ordinal));
            }
            Err(error) => {
                outcome = Err(error);
                break 'outer;
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

#[test]
fn fn_defines_and_calls_with_params() {
    let executed = ok("fn twice word:\n    echo $word\n    echo $word again\ntwice hi\necho done\n");
    assert_eq!(
        executed,
        vec!["echo $word", "echo $word again", "echo done"]
    );
    // The parameter is bound when the call starts (the defining capture
    // closes on the dedent line, which is then replayed from pushback).
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("fn say msg:"), Ok(Step::Skip));
    assert_eq!(interp.step("    echo $msg"), Ok(Step::Skip));
    assert_eq!(interp.step("say hello"), Ok(Step::Skip));
    assert!(interp.busy());
    assert_eq!(interp.step(""), Ok(Step::Skip));
    assert_eq!(interp.get("msg"), Some("hello"));
}

#[test]
fn for_loops_over_words() {
    let executed = ok("for d in alpha beta:\n    echo $d\necho after\n");
    assert_eq!(executed, vec!["echo $d", "echo $d", "echo after"]);
}

#[test]
fn if_inside_for_body() {
    let executed = ok("for n in 1 2:\n    if $STATUS = 0:\n        echo ok-$n\necho done\n");
    assert_eq!(executed, vec!["echo ok-$n", "echo ok-$n", "echo done"]);
}

#[test]
fn foreach_replays_per_source_word() {
    let script = "foreach f in ls:\n    yield file $f\necho after\n";
    let (executed, outcome) =
        run_with_sources(script, &[], &[("ls", "hello.txt docs demo.r")]);
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(
        executed,
        vec![
            "yield file $f",
            "yield file $f",
            "yield file $f",
            "echo after"
        ]
    );
    // The loop variable binds like a `for` variable.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("foreach f in ls:"), Ok(Step::Skip));
    assert_eq!(interp.step("    echo $f"), Ok(Step::Skip));
    interp.end_input().unwrap(); // EOF closes the capture
    assert_eq!(interp.source_request(), Some(("f", "ls")));
    interp.provide_source_words("a b").unwrap();
    assert_eq!(interp.get("f"), Some("a"));
    assert!(interp.busy());
    // An empty source runs nothing.
    let (executed, outcome) = run_with_sources("foreach f in ls:\n    echo $f\n", &[], &[("ls", "")]);
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(executed, Vec::<String>::new());
    // An unknown source fails soft: loop skipped, script continues.
    let (executed, outcome) =
        run_with_sources("foreach f in bogus:\n    echo $f\necho next\n", &[], &[]);
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(executed, vec!["echo next"]);
}

#[test]
fn foreach_errors_are_stable() {
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("foreach f:"), Err(RushError::BadForeach));
    assert_eq!(interp.step("foreach 1f in ls:"), Err(RushError::BadForeach));
    assert_eq!(interp.step("foreach f in :"), Err(RushError::BadForeach));
    // Nested control flow inside a replayed body rejects foreach too.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("fn bad:"), Ok(Step::Skip));
    assert_eq!(interp.step("    foreach f in ls:"), Ok(Step::Skip));
    assert_eq!(interp.step("bad"), Ok(Step::Skip));
    let mut drained = Ok(());
    for _ in 0..16 {
        match interp.step("") {
            Ok(_) => {}
            Err(error) => {
                drained = Err(error);
                break;
            }
        }
        if !interp.busy() {
            break;
        }
    }
    assert_eq!(drained, Err(RushError::NestedControl));
}

#[test]
fn match_selects_the_first_matching_arm() {
    let script = "fs = fat32\nmatch $fs:\n    \"ruofs\" => echo native\n    fat32 | exfat => echo fat-family\n    _ => echo other\necho after\n";
    assert_eq!(ok(script), vec!["echo fat-family", "echo after"]);

    // Wildcard catches an unmatched subject.
    assert_eq!(
        ok("match nope:\n    \"ruofs\" => echo native\n    _ => echo other\n"),
        vec!["echo other"]
    );
    // No matching arm and no wildcard: nothing runs, no error.
    assert_eq!(ok("match nope:\n    \"ruofs\" => echo native\n"), Vec::<String>::new());
    // First match wins.
    assert_eq!(
        ok("match a:\n    a => echo one\n    a | _ => echo two\n"),
        vec!["echo one"]
    );
}

#[test]
fn match_inside_if_and_dead_match() {
    // A match frame under an inactive `if` consumes its arms silently.
    let script = "if 1 = 2:\n    match $STATUS:\n        0 => echo no\necho top\n";
    assert_eq!(ok(script), vec!["echo top"]);
    // A match arm runs chains like any command line.
    let (executed, _) = run("match a:\n    a => echo x and echo y\n", &[]);
    assert_eq!(executed, vec!["echo x", "echo y"]);
}

#[test]
fn match_errors_are_stable() {
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("match $STATUS"), Err(RushError::BadMatch));
    assert_eq!(interp.step("match :"), Err(RushError::BadMatch));

    let mut interp = Interpreter::new();
    assert_eq!(interp.step("match a:"), Ok(Step::Skip));
    assert_eq!(interp.step("    echo no-arrow"), Err(RushError::BadMatchArm));

    let mut interp = Interpreter::new();
    assert_eq!(interp.step("match a:"), Ok(Step::Skip));
    assert_eq!(interp.step("    a =>"), Err(RushError::BadMatchArm));

    // `else:` does not attach to a match frame.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("match a:"), Ok(Step::Skip));
    assert_eq!(interp.step("else:"), Err(RushError::ElseWithoutIf));
}

#[test]
fn control_flow_errors_are_stable() {
    let mut interp = Interpreter::new();
    // A `for` line inside a `fn` body is captured verbatim; the nested
    // control flow is rejected when the call replays it.
    assert_eq!(interp.step("fn bad:"), Ok(Step::Skip));
    assert_eq!(interp.step("    for x in a:"), Ok(Step::Skip));
    assert_eq!(interp.step("bad"), Ok(Step::Skip));
    let mut drained = Ok(());
    for _ in 0..16 {
        match interp.step("") {
            Ok(_) => {}
            Err(error) => {
                drained = Err(error);
                break;
            }
        }
        if !interp.busy() {
            break;
        }
    }
    assert_eq!(drained, Err(RushError::NestedControl));

    // Malformed headers.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("fn :"), Err(RushError::BadFnDef));
    assert_eq!(interp.step("for x:"), Err(RushError::BadFor));
    assert_eq!(interp.step("for 1x in a:"), Err(RushError::BadFor));

    // Recursion past the call bound fails deterministically.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("fn recur:"), Ok(Step::Skip));
    assert_eq!(interp.step("    recur"), Ok(Step::Skip));
    assert_eq!(interp.step("recur"), Ok(Step::Skip));
    let mut drained = Ok(());
    for _ in 0..64 {
        match interp.step("") {
            Ok(_) => {}
            Err(error) => {
                drained = Err(error);
                break;
            }
        }
        if !interp.busy() {
            break;
        }
    }
    assert_eq!(drained, Err(RushError::CallTooDeep));
}
