use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, Program, Value, analyze, evaluate, format_source, parse,
};

fn escape_warns(source: &str) -> bool {
    let result = analyze(source);
    assert!(result.is_valid(), "{source}: {:?}", result.diagnostics);
    result.diagnostics.iter().any(|d| d.code == "region-escape")
}

#[test]
fn region_executes_body_and_yields_last_value() {
    assert_eq!(
        evaluate("region scratch { mut t = 40; t + 2 }", 200).unwrap(),
        Value::Number(42.0)
    );
    assert_eq!(
        evaluate("region { 1 + 1 }", 200).unwrap(),
        Value::Number(2.0)
    );
    // Anonymous and named forms both parse.
    assert!(parse("region { 1 }").is_valid());
    assert!(parse("region scratch { 1 }").is_valid());
    assert!(parse("region Scratch { 1 }").is_valid());
}

#[test]
fn region_is_a_statement_not_an_expression() {
    assert!(!parse("let x = region { 1 }").is_valid());
    assert!(!parse("region").is_valid());
    assert!(!parse("region scratch").is_valid());
    assert!(analyze("region scratch { let t = 1 }").is_valid());
}

#[test]
fn region_bindings_do_not_leak_but_assigned_outer_cells_survive() {
    let leaked = evaluate("region r { mut tmp = 1 }\ntmp", 200);
    assert!(leaked.is_err(), "{leaked:?}");
    assert_eq!(
        evaluate(
            "mut keep = 0\nregion r { mut tmp = 41\nkeep = tmp + 1 }\nkeep",
            200
        )
        .unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn values_still_reachable_after_region_are_promoted_to_parent() {
    // A closure capturing a region-local cell escapes: the cell must
    // outlive the region sweep.
    assert_eq!(
        evaluate(
            "mut g = () => 0\nregion r { mut n = 40\ng = () => n += 2 }\ng()\ng()\n",
            400
        )
        .unwrap(),
        Value::Number(44.0)
    );
    // A plain returned value also survives.
    assert_eq!(
        evaluate("mut keep = [0]\nregion r { keep = [1, 2, 3] }\nkeep", 400).unwrap(),
        evaluate("[1, 2, 3]", 100).unwrap()
    );
}

#[test]
fn nested_regions_free_inner_without_touching_outer() {
    assert_eq!(
        evaluate(
            "mut outer_keep = 0\nregion a { mut x = 1\nregion b { mut y = 2\nouter_keep = x + y }\nouter_keep += x * 10 }\nouter_keep",
            400
        )
        .unwrap(),
        Value::Number(13.0)
    );
    // Inner region bindings are gone once the inner region ends.
    assert!(evaluate("region a { region b { mut y = 1 }\nmut z = y }", 200).is_err());
}

#[test]
fn region_sweeps_on_break_continue_and_return() {
    assert_eq!(
        evaluate(
            "mut s = 0\nfor i in range_iter(0, 10) { region r { mut t = i * 2\nif t > 6 { break }\ns += t } }\ns",
            2000
        )
        .unwrap(),
        Value::Number(12.0)
    );
    assert_eq!(
        evaluate(
            "mut s = 0\nfor i in range_iter(0, 5) { region r { if i % 2 == 0 { continue }\ns += i } }\ns",
            2000
        )
        .unwrap(),
        Value::Number(4.0)
    );
    assert_eq!(
        evaluate("fn f() { region r { return 42 }\nreturn 0 }\nf()", 400).unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn region_cells_are_reused_across_iterations() {
    // 10k iterations each allocating fresh region cells: without the
    // bulk sweep the default cell budget would be exhausted.
    // 2i mod 7 cycles 0,2,4,6,1,3,5 summing to 21 per 7 iterations;
    // 10000 = 1428*7 + 4 -> 1428*21 + (0+2+4+6) = 30000.
    assert_eq!(
        evaluate(
            "mut s = 0\nfor i in range_iter(0, 10000) { region r { mut a = i\nmut b = a * 2\ns += b % 7 } }\ns",
            2_000_000
        )
        .unwrap(),
        Value::Number(30000.0)
    );
}

#[test]
fn formatter_round_trips_regions() {
    let formatted = format_source("region scratch { mut t=1\nt+1 }").unwrap();
    assert_eq!(
        formatted,
        "region scratch {\n    mut t = 1;\n    t + 1;\n}\n"
    );
    assert_eq!(
        format_source(&formatted).unwrap(),
        formatted,
        "not stable: {formatted}"
    );
    let anonymous = format_source("region { 1 }").unwrap();
    assert_eq!(anonymous, "region {\n    1;\n}\n");
    assert_eq!(
        evaluate(&formatted, 200).unwrap(),
        evaluate("region scratch { mut t = 1\nt + 1 }", 200).unwrap()
    );
}

#[test]
fn escape_warnings_fire_on_cell_carrying_values() {
    // Closure capture escaping via an outer binding.
    assert!(escape_warns(
        "mut g = () => 0\nregion r { mut n = 1\ng = () => n += 1 }"
    ));
    // Closure returned out of the region.
    assert!(escape_warns(
        "fn f() { region r { mut n = 1\nreturn () => n } }\nf()"
    ));
    // Values of unknown shape are treated conservatively.
    assert!(escape_warns(
        "fn f(x) { mut out = x\nregion r { mut tmp = x\nout = tmp }\nreturn out }\nf(1)"
    ));
}

#[test]
fn escape_warnings_stay_silent_for_scalars_and_local_data() {
    for source in [
        // Region-local value used locally.
        "region r { mut tmp = 1\ntmp + 1 }",
        // Same-region assignment.
        "region r { mut a = 1\nmut b = 2\na = b }",
        // Outer-to-outer assignment inside a region.
        "mut x = 1\nmut y = 2\nregion r { x = y }",
        // Outer value read inside a region, no assignment.
        "const z = 3\nregion r { mut t = z\nt + 1 }",
        // Return of outer data from inside a region.
        "mut x = 7\nfn f() { region r { return x } }\nf()",
        // Scalar copies never move a cell: no lint.
        "mut keep = 0\nregion r { mut tmp = 1\nkeep = tmp }",
        "mut keep = 0\nregion r { mut tmp = 1\nkeep = tmp + 1 }",
        "fn f() { region r { mut tmp = 1\nreturn tmp } }\nf()",
        // Collections of known scalars are copies too.
        "mut keep = [0]\nregion r { mut tmp = 1\nkeep = [tmp, tmp] }",
    ] {
        let result = analyze(source);
        assert!(
            result.diagnostics.is_empty(),
            "{source}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn strict_regions_reject_implicit_escape_and_accept_promote() {
    for source in [
        // Closure capture escapes a cell: hard error in a strict region.
        "mut g = () => 0\nstrict region r { mut n = 1\ng = () => n += 1 }",
        "fn f() { strict region r { mut n = 1\nreturn () => n } }\nf()",
        // Cells of the strict region escape through a lax parent, too.
        "mut g = () => 0\nregion a { strict region b { mut y = 1\ng = () => y } }",
        // Unknown shapes are treated conservatively.
        "fn f(x) { mut out = x\nstrict region r { mut tmp = x\nout = tmp }\nreturn out }\nf(1)",
    ] {
        let result = analyze(source);
        assert!(!result.is_valid(), "accepted: {source}");
        assert!(
            result.diagnostics.iter().any(|d| d.code == "region-escape"),
            "{source}: {:?}",
            result.diagnostics
        );
    }
    for source in [
        // promote(...) is the explicit escape hatch.
        "mut keep = 0\nstrict region r { mut tmp = 41\nkeep = promote(tmp + 1) }",
        "fn f() { strict region r { mut tmp = 1\nreturn promote(tmp) } }\nf()",
        "mut g = () => 0\nstrict region r { mut n = 1\ng = promote(() => n) }",
        // Nothing escapes.
        "strict region r { mut tmp = 1\ntmp + 1 }",
        // Scalar copies are values, not cells: allowed without promote.
        "mut keep = 0\nstrict region r { mut tmp = 41\nkeep = tmp + 1 }",
        "fn f() { strict region r { mut tmp = 1\nreturn tmp } }\nf()",
    ] {
        let result = analyze(source);
        assert!(
            result.is_valid() && result.diagnostics.is_empty(),
            "{source}: {:?}",
            result.diagnostics
        );
    }
    // Only a lax region is crossed by the escaping closure: non-fatal lint.
    let lax = analyze("mut g = () => 0\nregion a { mut x = 1\nstrict region b { g = () => x } }");
    assert!(lax.is_valid(), "{:?}", lax.diagnostics);
    assert!(
        lax.diagnostics.iter().any(|d| d.code == "region-escape"),
        "{:?}",
        lax.diagnostics
    );
}

#[test]
fn promote_is_identity_at_runtime_and_strict_regions_sweep() {
    assert_eq!(
        evaluate(
            "mut keep = 0\nstrict region r { mut tmp = 41\nkeep = promote(tmp + 1) }\nkeep",
            400
        )
        .unwrap(),
        Value::Number(42.0)
    );
    assert_eq!(
        evaluate(
            "fn f() { strict region r { mut tmp = 21\nreturn promote(tmp * 2) } }\nf()",
            400
        )
        .unwrap(),
        Value::Number(42.0)
    );
    // sum of i % 3 over 0..1000: 333 full cycles of 3 plus 0 = 999
    assert_eq!(
        evaluate(
            "mut s = 0\nfor i in range_iter(0, 1000) { strict region r { mut a = i\ns += a % 3 } }\ns",
            500_000
        )
        .unwrap(),
        Value::Number(999.0)
    );
}

#[test]
fn formatter_round_trips_strict_regions() {
    let formatted = format_source("strict region r { mut t=1\nt }").unwrap();
    assert_eq!(formatted, "strict region r {\n    mut t = 1;\n    t;\n}\n");
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    assert!(parse("strict region { 1 }").is_valid());
    assert!(parse("strict region Name { 1 }").is_valid());
}

#[test]
fn region_budget_caps_live_cells() {
    // Within budget.
    assert_eq!(
        evaluate("region r (2) { mut a = 1\nmut b = 2\na + b }", 400).unwrap(),
        Value::Number(3.0)
    );
    // Exceeded by the second cell.
    let failure = evaluate("region r (1) { mut a = 1\nmut b = 2 }", 400).unwrap_err();
    assert_eq!(failure.message, "Region cell budget exceeded");
    // Zero budget allows no cells at all.
    assert!(evaluate("region r (0) { mut a = 1 }", 400).is_err());
}

#[test]
fn region_budget_counts_live_cells_not_total_allocations() {
    // 100 iterations, one scratch cell each: freed cells stop counting.
    assert_eq!(
        evaluate(
            "mut s = 0\nregion r (2) { for i in range_iter(0, 100) { mut t = i\ns += t % 2 } }\ns",
            100_000
        )
        .unwrap(),
        Value::Number(50.0)
    );
    // Budget expression may be computed.
    assert_eq!(
        evaluate("region r (1 + 1) { mut a = 1\nmut b = 2 }", 400).unwrap(),
        Value::Number(2.0)
    );
}

#[test]
fn region_budget_validates_its_expression() {
    for (source, message) in [
        (
            "region r (-1) { 1 }",
            "Region budget must be a non-negative integer",
        ),
        (
            "region r (1.5) { 1 }",
            "Region budget must be a non-negative integer",
        ),
        // Statically known non-numeric budget is rejected by analysis.
        (
            "region r (\"wide\") { 1 }",
            "region-budget: Region budget must be a number",
        ),
    ] {
        let failure = evaluate(source, 400).unwrap_err();
        assert_eq!(failure.message, message, "{source}");
    }
    // Static type check when the budget shape is known to be non-numeric;
    // evaluate() above already fails it through the strict compile pass.
}

#[test]
fn promoted_cells_count_against_the_parent_budget() {
    // The closure escapes inner with its captured cell; outer has room for
    // exactly one live cell (f), so promotion of n overflows it.
    let failure = evaluate(
        "region outer (1) { mut f = () => 0\nregion inner { mut n = 5\nf = promote(() => n) } }",
        400,
    )
    .unwrap_err();
    assert_eq!(failure.message, "Region cell budget exceeded");
    // Same program with headroom runs.
    assert_eq!(
        evaluate(
            "region outer (2) { mut f = () => 0\nregion inner { mut n = 5\nf = promote(() => n) }\nf() }",
            400,
        )
        .unwrap(),
        Value::Number(5.0)
    );
}

#[test]
fn formatter_round_trips_region_budgets() {
    let formatted = format_source("region scratch (64) { 1 }").unwrap();
    assert_eq!(formatted, "region scratch (64) {\n    1;\n}\n");
    assert_eq!(format_source(&formatted).unwrap(), formatted);
    let strict = format_source("strict region (64 + 1) { 1 }").unwrap();
    assert_eq!(strict, "strict region (64 + 1) {\n    1;\n}\n");
    assert!(parse("region (1024) { 1 }").is_valid());
}

mod coroutine_regions {
    use themoretheless_tokenizer_rush::{
        CancellationToken, CoroutineState, ExecutionLimits, Program, Value,
    };
    const LIMITS: ExecutionLimits = ExecutionLimits::new(10_000);

    fn spawn(
        source: &'static str,
        args: &[Value<'static>],
    ) -> (
        themoretheless_tokenizer_rush::ScriptInstance<'static, 'static>,
        themoretheless_tokenizer_rush::CoroutineId,
    ) {
        let program = Program::compile(source).unwrap();
        let token: &'static CancellationToken = Box::leak(Box::new(CancellationToken::default()));
        let mut script = program
            .instantiate(LIMITS, token, &[], &[], &[], &[])
            .unwrap();
        let task = script.spawn_coroutine("work", args, LIMITS).unwrap();
        (script, task)
    }

    #[test]
    fn yield_inside_region_suspends_and_the_region_survives() {
        let (mut script, task) = spawn(
            "fn work() -> number { region r { mut t = 40; yield t; t += 2; yield t }; return 99 }",
            &[],
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(40.))
        );
        // The region did NOT end at the yield: t keeps its cell and value.
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(42.))
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(99.))
        );
    }

    #[test]
    fn arena_run_works_in_coroutines_but_its_callback_cannot_suspend() {
        // A non-suspending arena callback runs fine on the frame machine.
        let (mut script, task) = spawn(
            "const a = arena(\"slot\")\nfn body() { mut t = 40\nreturn t + 2 }\nfn work() -> number { mut r = arena_run(a, body)\nyield r\nreturn r }",
            &[],
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(42.))
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(42.))
        );
        // Like map/filter, arena_run drives its callback synchronously, so a
        // suspending callback is rejected at compile time.
        assert!(
            Program::compile(
                "const a = arena(\"slot\")\nfn body() { yield 1 }\nfn work() { arena_run(a, body) }",
            )
            .is_err()
        );
    }

    #[test]
    fn interleaved_tasks_keep_isolated_region_state() {
        let program = Program::compile(
            "fn work(n: number) -> number { region r { mut t = n; yield t; t += 100; yield t; return t } }",
        )
        .unwrap();
        let token = CancellationToken::default();
        let mut script = program
            .instantiate(LIMITS, &token, &[], &[], &[], &[])
            .unwrap();
        let a = script
            .spawn_coroutine("work", &[Value::Number(1.)], LIMITS)
            .unwrap();
        let b = script
            .spawn_coroutine("work", &[Value::Number(2.)], LIMITS)
            .unwrap();
        assert_eq!(
            script.resume_coroutine(a, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(1.))
        );
        assert_eq!(
            script.resume_coroutine(b, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(2.))
        );
        assert_eq!(
            script.resume_coroutine(a, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(101.))
        );
        assert_eq!(
            script.resume_coroutine(b, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(102.))
        );
        assert_eq!(
            script.resume_coroutine(a, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(101.))
        );
        assert_eq!(
            script.resume_coroutine(b, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(102.))
        );
    }

    #[test]
    fn break_and_return_inside_region_sweep_while_suspended_loop_lives() {
        let (mut script, task) = spawn(
            "fn work() -> number { mut s = 0; for i in range_iter(0, 10) { region r { mut t = i * 2; if t > 6 { break }; s += t; yield s } }; return s }",
            &[],
        );
        for expected in [0., 2., 6., 12.] {
            assert_eq!(
                script.resume_coroutine(task, LIMITS).unwrap(),
                CoroutineState::Yielded(Value::Number(expected))
            );
        }
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(12.))
        );

        let (mut script, task) = spawn(
            "fn work() -> number { region r { mut t = 1; yield t; return 42 }; return 0 }",
            &[],
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(1.))
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(42.))
        );
    }

    #[test]
    fn promoted_capture_outlives_the_region_across_suspension() {
        let (mut script, task) = spawn(
            "fn work() -> number { mut g = () => 0; region r { mut n = 10; g = () => n; yield n }; return g() }",
            &[],
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Yielded(Value::Number(10.))
        );
        assert_eq!(
            script.resume_coroutine(task, LIMITS).unwrap(),
            CoroutineState::Complete(Value::Number(10.))
        );
    }

    #[test]
    fn coroutine_regions_feed_the_same_stats() {
        let (mut script, task) = spawn(
            "fn work() -> number { region tick { mut t = 1; yield t; mut u = 2; yield u + t }; return 0 }",
            &[],
        );
        script.resume_coroutine(task, LIMITS).unwrap();
        script.resume_coroutine(task, LIMITS).unwrap();
        let stats = script.region_stats();
        let tick = stats
            .iter()
            .find(|s| s.name.as_deref() == Some("tick"))
            .unwrap_or_else(|| panic!("missing stats: {stats:?}"));
        assert_eq!(tick.entries, 1);
        assert_eq!(tick.allocated, 2);
        assert_eq!(tick.peak_live, 2);
    }

    #[test]
    fn region_budget_is_enforced_inside_coroutines() {
        let (mut script, task) = spawn(
            "fn work() -> number { region r (1) { mut a = 1; mut b = 2; yield a }; return 0 }",
            &[],
        );
        let error = script.resume_coroutine(task, LIMITS).unwrap_err();
        assert!(
            error.message.contains("Region cell budget exceeded"),
            "{:?}",
            error.message
        );
        // The task is terminated and cannot be resumed again.
        assert!(script.resume_coroutine(task, LIMITS).is_err());
    }
}

#[test]
fn region_stats_track_entries_peak_reuse_and_promotion() {
    let program = Program::compile(
        "mut keep = 0\n\
         for i in range_iter(0, 10) { region scratch { mut a = i\nmut b = a * 2\nkeep += b % 5 } }\n\
         region scratch { mut x = 1\nmut y = 2\nmut z = 3 }\n\
         mut g = () => 0\n\
         region lift { mut n = 7\ng = () => n }\n\
         region { mut q = 1 }\n\
         keep",
    )
    .unwrap();
    let token = CancellationToken::default();
    let script = program
        .instantiate(ExecutionLimits::new(100_000), &token, &[], &[], &[], &[])
        .unwrap();
    let stats = script.region_stats();
    let find = |name: Option<&str>| {
        stats
            .iter()
            .find(|s| s.name.as_deref() == name)
            .unwrap_or_else(|| panic!("missing stats for {name:?}: {stats:?}"))
    };
    let scratch = find(Some("scratch"));
    assert_eq!(scratch.entries, 11);
    assert_eq!(scratch.allocated, 23);
    assert_eq!(scratch.peak_live, 3);
    assert!(
        scratch.reused_slots >= 15,
        "expected slot reuse, got {scratch:?}"
    );
    assert_eq!(scratch.promoted, 0);
    let lift = find(Some("lift"));
    assert_eq!(lift.entries, 1);
    assert_eq!(lift.promoted, 1, "captured cell escapes via promotion");
    let anonymous = find(None);
    // One explicit anonymous region plus 10 implicit per-iteration arenas
    // from the for loop; only the explicit one allocated a cell.
    assert_eq!((anonymous.entries, anonymous.allocated), (11, 1));
}

#[test]
fn suggested_budget_gives_power_of_two_headroom() {
    let stats = |peak_live| themoretheless_tokenizer_rush::RegionStats {
        peak_live,
        ..Default::default()
    };
    assert_eq!(stats(0).suggested_budget(), 1);
    assert_eq!(stats(1).suggested_budget(), 2);
    assert_eq!(stats(2).suggested_budget(), 4);
    assert_eq!(stats(3).suggested_budget(), 4);
    assert_eq!(stats(4).suggested_budget(), 8);
    assert_eq!(stats(100).suggested_budget(), 128);
}

#[test]
fn move_takes_value_empties_cell_and_allows_revival() {
    assert_eq!(
        evaluate("mut a = 41\nmut b = move(a)\nb", 200).unwrap(),
        Value::Number(41.0)
    );
    // Reading the emptied binding is rejected statically.
    let failure = evaluate("mut a = 1\nmut b = move(a)\na", 200).unwrap_err();
    assert!(
        failure.message.contains("moved-value"),
        "{:?}",
        failure.message
    );
    // Plain reassignment revives the cell.
    assert_eq!(
        evaluate("mut a = 1\nmut b = move(a)\na = 5\na + b", 200).unwrap(),
        Value::Number(6.0)
    );
    // Capturing closures observe the emptied cell.
    assert_eq!(
        evaluate("mut a = 1\nmut f = () => a\nmut b = move(a)\nf()", 400).unwrap(),
        Value::Null
    );
    // Runtime argument validation.
    assert!(evaluate("mut a = 1\nmove(a, a)", 200).is_err());
    assert!(evaluate("move(1)", 200).is_err());
    assert!(evaluate("move(nope)", 200).is_err());
    let failure = evaluate("const c = 1\nmove(c)", 200).unwrap_err();
    assert!(
        failure.message.contains("move requires a mutable binding"),
        "{:?}",
        failure.message
    );
}

#[test]
fn move_is_checked_statically() {
    for (source, code) in [
        // Read after move.
        ("mut a = 1\nmut b = move(a)\nb + a", "moved-value"),
        // Double move.
        ("mut a = 1\nmut b = move(a)\nmut c = move(a)", "moved-value"),
        // Immutable target.
        ("const a = 1\nmove(a)", "move-immutable"),
        // Not a name.
        ("mut a = 1\nmove(a + 1)", "move-target"),
        // Arity.
        ("mut a = 1\nmove()", "move-arity"),
    ] {
        let result = analyze(source);
        assert!(!result.is_valid(), "accepted: {source}");
        assert!(
            result.diagnostics.iter().any(|d| d.code == code),
            "{source}: {:?}",
            result.diagnostics
        );
    }
    for source in [
        "mut a = 1\nmut b = move(a)\nb",
        // Revival: assign, then read.
        "mut a = 1\nmut b = move(a)\na = 2\nb + a",
    ] {
        let result = analyze(source);
        assert!(
            result.is_valid() && result.diagnostics.is_empty(),
            "{source}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn move_is_explicit_relocation_for_regions() {
    // Moving out of a strict region needs no promote wrapper.
    let result = analyze("mut keep = 0\nstrict region r { mut tmp = 1\nkeep = move(tmp) }");
    assert!(
        result.is_valid() && result.diagnostics.is_empty(),
        "{:?}",
        result.diagnostics
    );
    assert_eq!(
        evaluate(
            "mut keep = 0\nstrict region r { mut tmp = 1\nkeep = move(tmp) }\nkeep",
            400
        )
        .unwrap(),
        Value::Number(1.0)
    );
}

#[test]
fn implicit_iteration_regions_promote_escaping_cells() {
    // Closures capturing per-iteration cells survive their iteration:
    // the implicit arena promotes them exactly like an explicit `region`.
    assert_eq!(
        evaluate(
            "mut a = () => 0\nmut b = () => 0\nfor i in range_iter(0, 5) { mut n = i\nif i == 1 { a = () => n }\nif i == 3 { b = () => n } }\na() * 100 + b()",
            2000
        )
        .unwrap(),
        Value::Number(103.0)
    );
    assert_eq!(
        evaluate(
            "mut i = 0\nmut g = () => 0\nwhile i < 3 { mut n = i * 7\ng = () => n\ni += 1 }\ng()",
            2000
        )
        .unwrap(),
        Value::Number(14.0)
    );
    // Iteration scratch stays scratch: 10k iterations with 2 cells each do
    // not exhaust the default cell budget.
    assert_eq!(
        evaluate(
            "mut s = 0\nfor i in range_iter(0, 10000) { mut a = i\nmut b = a * 2\ns += b % 7 }\ns",
            2_000_000
        )
        .unwrap(),
        Value::Number(30000.0)
    );
}

#[test]
fn implicit_call_regions_promote_escaping_cells() {
    // A closure capturing a call-local cell survives the call: the implicit
    // per-call arena promotes it into the caller's region.
    assert_eq!(
        evaluate(
            "fn make() { mut n = 10\nreturn () => n += 1 }\nconst f = make()\nf()\nf()\nf()",
            2000
        )
        .unwrap(),
        Value::Number(13.0)
    );
    // Call scratch stays scratch: 10k calls with 2 local cells each do not
    // exhaust the cell budget.
    assert_eq!(
        evaluate(
            "fn f(x) { mut a = x\nmut b = a * 2\nreturn b % 7 }\nmut s = 0\nfor i in range_iter(0, 10000) { s += f(i) }\ns",
            2_000_000
        )
        .unwrap(),
        Value::Number(30000.0)
    );
    // Recursion still works: each frame gets its own micro-arena.
    assert_eq!(
        evaluate(
            "fn fib(n) { if n < 2 { return n }\nmut l = fib(n - 1)\nmut r = fib(n - 2)\nreturn l + r }\nfib(15)",
            2_000_000
        )
        .unwrap(),
        Value::Number(610.0)
    );
}

#[test]
fn arena_run_executes_callback_inside_a_named_region() {
    // Scratch dies at arena_run exit; escaping closures promote; the
    // callback's return value is the run's value.
    assert_eq!(
        evaluate(
            "const a = arena(\"frame\")\nmut keep = () => 0\nfn body() { mut n = 21\nkeep = () => n\nreturn n * 2 }\nconst r = arena_run(a, body)\nr + keep()",
            2000
        )
        .unwrap(),
        Value::Number(63.0)
    );
    // Stats aggregate under the arena name, including repeated runs.
    assert_eq!(
        evaluate(
            "const a = arena(\"frame\")\nfn body() { mut t = 1\nreturn t }\nfor i in range_iter(0, 3) { arena_run(a, body) }\nconst s = arena_stats(a)\ns.entries",
            2000
        )
        .unwrap(),
        Value::Number(3.0)
    );
}

#[test]
fn arena_budget_is_enforced_across_promotion() {
    // A budgeted arena fails when too many cells stay live...
    assert!(
        evaluate(
            "const a = arena(\"tight\", 1)\nfn body() { mut x = 1\nmut y = 2\nmut z = 3\nreturn null }\narena_run(a, body)",
            2000
        )
        .is_err()
    );
    // ...but cells that die inside the run do not count.
    assert_eq!(
        evaluate(
            "const a = arena(\"tight\", 1)\nfn body() { mut x = 1\nreturn x + 1 }\narena_run(a, body)",
            2000
        )
        .unwrap(),
        Value::Number(2.0)
    );
}

#[test]
fn arena_errors_are_clear() {
    for source in [
        "arena(1)",
        "arena(\"\")",
        "arena(\"a\", -1)",
        "arena_run(1, () => 1)",
        "arena_run(arena(\"a\"), 1)",
        "arena_stats({name: 1})",
    ] {
        assert!(evaluate(source, 200).is_err(), "{source}");
    }
    // A forged descriptor behaves exactly like the real one.
    assert_eq!(
        evaluate(
            "fn body() { mut t = 5\nreturn t }\narena_run({name: \"forged\", budget: null}, body)",
            200
        )
        .unwrap(),
        Value::Number(5.0)
    );
}
