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
        vec!["echo hello $who", "nosuchcmd", "echo $STATUS",]
    );
    // The driver resolves $who/$STATUS at parse time; the table holds them:
    let mut interp = Interpreter::new();
    interp.seed("USER", "admin");
    assert_eq!(interp.step("who = $USER"), Ok(Step::Skip));
    assert_eq!(interp.get("who"), Some("admin"));
}

#[test]
fn if_else_blocks_follow_the_condition() {
    let executed = ok("if $STATUS = 0:\n    echo if-ok\nelse:\n    echo else-bad\necho after\n");
    assert_eq!(executed, vec!["echo if-ok", "echo after"]);

    let (executed, outcome) = run(
        "badcmd\nif $STATUS = 0:\n    echo no\nelse:\n    echo yes\n",
        &[1],
    );
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(executed, vec!["badcmd", "echo yes"]);
}

#[test]
fn nested_blocks_and_not() {
    let executed =
        ok("if not $MISSING:\n    if 2 > 1:\n        echo deep\n    echo mid\necho top\n");
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
    let executed =
        ok("fn twice word:\n    echo $word\n    echo $word again\ntwice hi\necho done\n");
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
    let (executed, outcome) = run_with_sources(script, &[], &[("ls", "hello.txt docs demo.r")]);
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
    let (executed, outcome) =
        run_with_sources("foreach f in ls:\n    echo $f\n", &[], &[("ls", "")]);
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
    assert_eq!(
        ok("match nope:\n    \"ruofs\" => echo native\n"),
        Vec::<String>::new()
    );
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
    assert_eq!(
        interp.step("    echo no-arrow"),
        Err(RushError::BadMatchArm)
    );

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

#[test]
fn break_exits_for_and_foreach_loops() {
    // `break` ends the loop after the current command, mid-word-list.
    let executed = ok("for n in a b c:\n    echo $n\n    break\necho after\n");
    assert_eq!(executed, vec!["echo $n", "echo after"]);

    // Same for a host-fed `foreach` loop.
    let (executed, outcome) = run_with_sources(
        "foreach f in ls:\n    echo $f\n    break\necho after\n",
        &[],
        &[("ls", "one two three")],
    );
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(executed, vec!["echo $f", "echo after"]);

    // A `break` in a dead branch does not fire.
    let executed =
        ok("for n in a b:\n    if 1 = 2:\n        break\n    echo $n\necho done\n");
    assert_eq!(executed, vec!["echo $n", "echo $n", "echo done"]);
}

#[test]
fn continue_skips_to_the_next_iteration() {
    let executed = ok(
        "for n in a b c:\n    if $n = b:\n        continue\n    echo $n\necho done\n",
    );
    assert_eq!(executed, vec!["echo $n", "echo $n", "echo done"]);

    // `continue` skips the rest of the body, not just one line.
    let executed =
        ok("for n in a b:\n    echo one-$n\n    continue\n    echo two-$n\necho done\n");
    assert_eq!(executed, vec!["echo one-$n", "echo one-$n", "echo done"]);

    // Same for `foreach`.
    let (executed, outcome) = run_with_sources(
        "foreach f in ls:\n    if $f = b:\n        continue\n    echo $f\necho done\n",
        &[],
        &[("ls", "a b c")],
    );
    assert_eq!(outcome.map(|_| ()), Ok(()));
    assert_eq!(executed, vec!["echo $f", "echo $f", "echo done"]);
}

#[test]
fn loop_control_outside_a_loop_is_an_error() {
    let mut interp = Interpreter::new();
    assert_eq!(
        interp.step("break"),
        Err(RushError::LoopControlOutsideLoop)
    );
    let mut interp = Interpreter::new();
    assert_eq!(
        interp.step("continue"),
        Err(RushError::LoopControlOutsideLoop)
    );

    // A `fn` body is its own replay frame, not a loop: `break` inside a
    // function errors even when the call site is top level.
    let (_, outcome) = run("fn stop:\n    break\nstop\n", &[]);
    assert_eq!(outcome, Err(RushError::LoopControlOutsideLoop));
}

#[test]
fn loop_control_does_not_cross_the_fn_boundary() {
    // Documented behaviour: a `fn` call is a hard boundary for loop
    // control. `break`/`continue` inside a function called from a loop
    // body errors (`LoopControlOutsideLoop`) instead of punching through
    // to the caller's loop — the callee's replay frame is not a loop.
    let (_, outcome) = run("fn stop:\n    break\nfor n in a b:\n    stop\necho after\n", &[]);
    assert_eq!(outcome, Err(RushError::LoopControlOutsideLoop));

    let (_, outcome) = run("fn skip:\n    continue\nfor n in a b:\n    skip\n", &[]);
    assert_eq!(outcome, Err(RushError::LoopControlOutsideLoop));

    // Calls from a loop still work when the callee uses no loop control.
    let executed = ok("fn say word:\n    echo $word\nfor n in a b:\n    say $n\necho done\n");
    assert_eq!(
        executed,
        vec!["echo $word", "echo $word", "echo done"]
    );
}

#[test]
fn else_if_chains_pick_the_first_taken_branch() {
    // Middle rung matches: later `else` stays dead.
    let executed =
        ok("if 1 = 2:\n    echo a\nelse if 2 = 2:\n    echo b\nelse:\n    echo c\necho after\n");
    assert_eq!(executed, vec!["echo b", "echo after"]);

    // First branch taken: later rungs stay dead even when true.
    let executed = ok("if 1:\n    echo a\nelse if 1:\n    echo b\nelse:\n    echo c\n");
    assert_eq!(executed, vec!["echo a"]);

    // Nothing taken: `else` runs.
    let executed =
        ok("if 1 = 2:\n    echo a\nelse if 3 = 4:\n    echo b\nelse:\n    echo c\n");
    assert_eq!(executed, vec!["echo c"]);

    // Chain without `else`: nothing runs, no error.
    let executed =
        ok("if 1 = 2:\n    echo a\nelse if 3 = 4:\n    echo b\necho after\n");
    assert_eq!(executed, vec!["echo after"]);

    // Longer chain: first matching rung wins.
    let executed = ok(
        "if 1 = 2:\n    echo a\nelse if 2 = 3:\n    echo b\nelse if 3 = 3:\n    echo c\nelse if 4 = 4:\n    echo d\nelse:\n    echo e\n",
    );
    assert_eq!(executed, vec!["echo c"]);

    // `else if` under a dead outer `if` evaluates but stays dead.
    let executed =
        ok("if 1 = 2:\n    if 1 = 2:\n        echo a\n    else if 1:\n        echo b\necho top\n");
    assert_eq!(executed, vec!["echo top"]);
}

#[test]
fn else_if_errors_are_stable() {
    // `else if` without a matching open `if`.
    let mut interp = Interpreter::new();
    assert_eq!(
        interp.step("else if 1:"),
        Err(RushError::ElseWithoutIf)
    );

    // Malformed condition.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("if 1:"), Ok(Step::Skip));
    assert_eq!(
        interp.step("else if $STATUS 0:"),
        Err(RushError::BadCondition)
    );

    // `else if` after `else:` — the chain is already closed.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("if 1:"), Ok(Step::Skip));
    assert_eq!(interp.step("else:"), Ok(Step::Skip));
    assert_eq!(
        interp.step("else if 1:"),
        Err(RushError::ElseWithoutIf)
    );
}

#[test]
fn break_inside_nested_if_in_a_loop() {
    // `break` nested two `if` levels deep still exits the loop, and the
    // stale branch frames do not leak past it.
    let executed = ok(
        "for n in 1 2 3:\n    if $n != 1:\n        if $n != 3:\n            break\n    echo $n\necho done\n",
    );
    assert_eq!(executed, vec!["echo $n", "echo done"]);

    // A loop nested in a block: replayed body lines must not disturb the
    // enclosing `if` frame, whether the loop ends via `break`...
    let executed = ok(
        "if 1:\n    for n in a b c:\n        echo $n\n        if $n = b:\n            break\n    echo inner\necho top\n",
    );
    assert_eq!(
        executed,
        vec!["echo $n", "echo $n", "echo inner", "echo top"]
    );

    // ...or by running the word list out with a dead branch at the tail.
    let executed = ok(
        "if 1:\n    for n in a b:\n        echo $n\n        if 1 = 2:\n            echo never\n    echo inner\necho top\n",
    );
    assert_eq!(
        executed,
        vec!["echo $n", "echo $n", "echo inner", "echo top"]
    );
}


#[test]
fn foreach_rows_bind_fields() {
    // Drive a rows source by hand: one `field=value ...` line per iteration.
    let mut interp = Interpreter::new();
    assert_eq!(interp.step("foreach f in ls:"), Ok(Step::Skip));
    assert_eq!(interp.step("    n = $f.name"), Ok(Step::Skip));
    assert_eq!(interp.step("    s = $f.size"), Ok(Step::Skip));
    interp.end_input().unwrap();
    assert_eq!(interp.source_request(), Some(("f", "ls")));
    interp
        .provide_source_rows("name=a.txt size=3\n\nname=b.bin size=5\n")
        .unwrap();
    // First row bound immediately; the blank line is skipped.
    assert_eq!(interp.get("f"), Some("a.txt"));
    assert_eq!(interp.get("f.name"), Some("a.txt"));
    assert_eq!(interp.get("f.size"), Some("3"));
    while interp.busy() {
        interp.step("").unwrap();
    }
    interp.finish().unwrap();
    // After the loop the last row's bindings remain visible.
    assert_eq!(interp.get("f.name"), Some("b.bin"));
    assert_eq!(interp.get("f.size"), Some("5"));
    assert_eq!(interp.get("n"), Some("b.bin"));
    assert_eq!(interp.get("s"), Some("5"));
}

#[test]
fn foreach_rows_plain_token_binds_var_only() {
    let mut interp = Interpreter::new();
    interp.step("foreach f in ls:").unwrap();
    interp.step("    echo $f").unwrap();
    interp.end_input().unwrap();
    interp.provide_source_rows("plain\n").unwrap();
    assert_eq!(interp.get("f"), Some("plain"));
    assert_eq!(interp.get("f.plain"), None);
    while interp.busy() {
        interp.step("").unwrap();
    }
    interp.finish().unwrap();
}

#[test]
fn foreach_rows_too_long_is_stable_error() {
    let mut interp = Interpreter::new();
    interp.step("foreach f in ls:").unwrap();
    interp.step("    echo $f").unwrap();
    interp.end_input().unwrap();
    let long = format!("name={}\n", "x".repeat(80));
    assert_eq!(
        interp.provide_source_rows(&long),
        Err(RushError::ValueTooLong)
    );
}

#[test]
fn dotted_expansion_falls_back_to_base_name() {
    let mut interp = Interpreter::new();
    interp.seed("USER", "admin");
    // No `USER.txt` variable: the `.txt` suffix stays literal.
    assert_eq!(interp.step("x = $USER.txt"), Ok(Step::Skip));
    assert_eq!(interp.get("x"), Some("admin.txt"));
    // A real dotted variable (bound by a foreach row) wins over the base.
    interp.step("foreach f in ls:").unwrap();
    interp.step("    y = $f.name").unwrap();
    interp.end_input().unwrap();
    interp.provide_source_rows("name=a.txt\n").unwrap();
    while interp.busy() {
        interp.step("").unwrap();
    }
    interp.finish().unwrap();
    assert_eq!(interp.get("y"), Some("a.txt"));
    // Trailing dot is literal punctuation, not part of the name.
    assert_eq!(interp.step("z = $USER."), Ok(Step::Skip));
    assert_eq!(interp.get("z"), Some("admin."));
}

#[test]
fn loop_variable_compares_in_condition() {
    let mut interp = Interpreter::new();
    for line in [
        "for n in one two three:",
        "    if $n = two:",
        "        hit = yes",
    ] {
        assert_eq!(interp.step(line), Ok(Step::Skip));
    }
    interp.end_input().unwrap();
    while interp.busy() {
        interp.step("").unwrap();
    }
    interp.finish().unwrap();
    assert_eq!(interp.get("hit"), Some("yes"));
}
