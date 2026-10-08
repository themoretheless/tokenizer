#![cfg(unix)]
use std::{thread, time::Duration};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, Program, Value,
    sys::{MODULE_SOURCE, SysHost, SysLimits},
};
fn run(source: &str, limits: SysLimits, token: &CancellationToken) -> Value<'static> {
    let module = Program::compile(MODULE_SOURCE).unwrap();
    let program = Program::compile(source).unwrap();
    let host = SysHost::new(limits);
    let instance = program
        .instantiate(
            ExecutionLimits::new(100_000),
            token,
            &[],
            &[],
            &host.registrations,
            &[("sys", &module)],
        )
        .unwrap();
    // Test output contains only owned scalar/tuple/variant data.
    fn own(v: &Value<'_>) -> Value<'static> {
        match v {
            Value::Number(n) => Value::Number(*n),
            Value::String(s) => Value::String(s.clone()),
            Value::Bool(b) => Value::Bool(*b),
            Value::Tuple(v) => Value::Tuple(v.iter().map(own).collect()),
            Value::List(v) => Value::List(v.iter().map(own).collect()),
            Value::Variant(n, v) => Value::Variant(n, v.iter().map(own).collect()),
            _ => panic!("unsupported test value"),
        }
    }
    own(instance.initial_value())
}
fn output(v: Value<'static>) -> Vec<Value<'static>> {
    let Value::Variant("Ok", mut v) = v else {
        panic!("{v:?}")
    };
    let Value::Tuple(t) = v.remove(0) else {
        panic!()
    };
    t
}
#[test]
fn arguments_are_literal_and_nonzero_exit_is_a_normal_result() {
    let token = CancellationToken::default();
    let values = output(run(
        "import sys; sys.run('/usr/bin/printf', ['%s', 'a b;$(exit 1)'], '')",
        Default::default(),
        &token,
    ));
    assert_eq!(values[1], Value::String("a b;$(exit 1)".into()));
    let values = output(run(
        "import sys; sys.run('/bin/sh', ['-c', 'printf error >&2; exit 7'], '')",
        Default::default(),
        &token,
    ));
    assert_eq!(values[0], Value::Number(7.0));
    assert_eq!(values[2], Value::String("error".into()));
}
#[test]
fn real_pipelines_transfer_stdin_and_report_upstream_failure() {
    let token = CancellationToken::default();
    let values = output(run(
        "import sys; sys.pipe([['/bin/cat'], ['/usr/bin/tr', 'a-z', 'A-Z']], 'abc')",
        Default::default(),
        &token,
    ));
    assert_eq!(values[1], Value::String("ABC".into()));
    let values = output(run(
        "import sys; sys.pipe([['/bin/sh', '-c', 'exit 8'], ['/bin/cat']], '')",
        Default::default(),
        &token,
    ));
    assert_eq!(values[0], Value::Number(8.0));
}
#[test]
fn output_limits_timeout_and_missing_commands_return_err() {
    let token = CancellationToken::default();
    for (source, limits) in [
        (
            "import sys; sys.run('/usr/bin/yes', [], '')",
            SysLimits {
                bytes: 128,
                ..Default::default()
            },
        ),
        (
            "import sys; sys.run('/bin/sleep', ['2'], '')",
            SysLimits {
                timeout: Duration::from_millis(30),
                ..Default::default()
            },
        ),
        (
            "import sys; sys.run('/rush-command-that-does-not-exist', [], '')",
            Default::default(),
        ),
    ] {
        assert!(matches!(
            run(source, limits, &token),
            Value::Variant("Err", _)
        ));
    }
}
#[test]
fn cancellation_kills_waiting_child_and_releases_pipes() {
    let token = CancellationToken::default();
    let cancel = token.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        cancel.cancel();
    });
    let module = Program::compile(MODULE_SOURCE).unwrap();
    let program =
        Program::compile("import sys; sys.run('/bin/sh', ['-c', 'sleep 20 & wait'], '')").unwrap();
    let host = SysHost::new(Default::default());
    let start = std::time::Instant::now();
    let error = program
        .instantiate(
            ExecutionLimits::new(100_000),
            &token,
            &[],
            &[],
            &host.registrations,
            &[("sys", &module)],
        )
        .err()
        .unwrap();
    worker.join().unwrap();
    assert!(error.message.contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn filesystem_and_environment_are_available_through_typed_module() {
    let dir = std::env::temp_dir().join(format!("rush-sys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "import sys; fn main()->Result[str,str] {{ sys.cd('{}')?; sys.mkdir('nested')?; sys.write('file.txt', 'hello')?; assert(sys.exists('file.txt')?); assert(sys.list_dir('.')? == ['file.txt', 'nested']); sys.rmdir('nested')?; sys.set_env('RUSH_SYS_TEST', 'local')?; assert(sys.env('RUSH_SYS_TEST') == Some('local')); let text = sys.read('file.txt')?; sys.remove('file.txt')?; return Ok(text) }}; main()",
        dir.display()
    );
    assert_eq!(
        run(&source, Default::default(), &CancellationToken::default()),
        Value::Variant("Ok", vec![Value::String("hello".into())])
    );
    assert!(std::env::var("RUSH_SYS_TEST").is_err());
    std::fs::remove_dir(dir).unwrap();
}
#[test]
fn imported_argument_contracts_reject_dead_branches_before_effects() {
    let module = Program::compile(MODULE_SOURCE).unwrap();
    let bad = Program::compile("import sys; if false { sys.run(1, [], '') }").unwrap();
    let host = SysHost::new(Default::default());
    assert!(
        bad.instantiate(
            ExecutionLimits::new(10_000),
            &CancellationToken::default(),
            &[],
            &[],
            &host.registrations,
            &[("sys", &module)]
        )
        .is_err()
    );
}

#[test]
fn shebang_is_lossless_and_stays_first_after_formatting() {
    let source = include_str!("../examples/automation/uppercase.r");
    let parsed = themoretheless_tokenizer_rush::parse(source);
    assert!(parsed.is_valid());
    let formatted = themoretheless_tokenizer_rush::format_source(source).unwrap();
    assert!(formatted.starts_with("#!/usr/bin/env -S rush --quiet\n"));
    assert!(Program::compile(&formatted).is_ok());
}

#[test]
fn named_outputs_preserve_fields_and_checked_failures_keep_diagnostics() {
    let token = CancellationToken::default();
    let source = "import sys; fn main()->Result[tuple[number,str,str],str] { let output: sys.ProcessOutput = sys.exec('/bin/sh', ['-c', 'printf out; printf err >&2; exit 7'], '')?; return Ok((output.code, output.stdout, output.stderr)) }; main()";
    assert_eq!(
        output(run(source, Default::default(), &token)),
        vec![
            Value::Number(7.0),
            Value::String("out".into()),
            Value::String("err".into())
        ]
    );
    let source = "import sys; fn main()->Result[bool,str] { let output = sys.exec('/bin/sh', ['-c', 'exit 7'], '')?; return Ok(match sys.checked(output) { Ok(value) => false, Err(value) => value.code == 7 }) }; main()";
    assert_eq!(
        run(source, Default::default(), &token),
        Value::Variant("Ok", vec![Value::Bool(true)])
    );
    let source = "import sys; fn main()->Result[str,str] { let output = sys.pipeline([['/bin/cat'], ['/usr/bin/tr', 'a-z', 'A-Z']], 'abc')?; assert(output.code == 0); return Ok(output.stdout) }; main()";
    assert_eq!(
        run(source, Default::default(), &token),
        Value::Variant("Ok", vec![Value::String("ABC".into())])
    );
}

#[test]
fn named_output_fields_are_checked_before_execution_and_helpers_are_private() {
    let module = Program::compile(MODULE_SOURCE).unwrap();
    for source in [
        "import sys; fn unused(output: sys.ProcessOutput)->number { return output.missing }; 0",
        "import sys; if false { sys.checked(1) }; 0",
        "import sys; if false { sys.process_output((0, '', '')) }; 0",
    ] {
        let program = Program::compile(source);
        if let Ok(program) = program {
            assert!(
                program.validate_modules(&[("sys", &module)]).is_err(),
                "{source}"
            );
        }
    }
}

#[test]
fn per_command_options_do_not_change_host_directory_or_environment() {
    let dir = std::env::temp_dir().join(format!("rush-command-options-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        r#"import sys; fn main()->Result[str,str] {{
        let before = sys.cwd();
        sys.set_env('RUSH_OPTIONS_TEST', 'base')?;
        let options: sys.CommandOptions = sys.CommandOptions({{cwd: Some('{}'), env: [('RUSH_OPTIONS_TEST', 'scoped')]}});
        let directory = sys.exec_with('/bin/pwd', [], '', options)?;
        let output = sys.pipeline_with([['/bin/sh', '-c', 'printf "$RUSH_OPTIONS_TEST"'], ['/usr/bin/tr', 'a-z', 'A-Z']], '', options)?;
        assert(output.stdout == 'SCOPED');
        assert(sys.cwd() == before);
        assert(sys.env('RUSH_OPTIONS_TEST') == Some('base'));
        let inherited = sys.exec_with('/bin/sh', ['-c', 'printf "$RUSH_OPTIONS_TEST"'], '', sys.command_options())?;
        assert(inherited.stdout == 'base');
        return Ok(directory.stdout)
}}; main()"#,
        dir.display()
    );
    assert_eq!(
        run(&source, Default::default(), &CancellationToken::default()),
        Value::Variant(
            "Ok",
            vec![Value::String(format!(
                "{}\n",
                std::fs::canonicalize(&dir).unwrap().display()
            ))]
        )
    );
    std::fs::remove_dir(dir).unwrap();
}
#[test]
fn invalid_scoped_environment_is_a_result_and_types_are_checked() {
    let value = run(
        "import sys; sys.exec_with('/bin/cat', [], '', sys.CommandOptions({cwd: None(), env: [('BAD=KEY', 'value')]}))",
        Default::default(),
        &CancellationToken::default(),
    );
    assert!(
        matches!(value, Value::Variant("Err", ref values) if matches!(&values[0], Value::String(s) if s.contains("Invalid environment")))
    );
    let value = run(
        "import sys; sys.exec_with('/bin/cat', [], '', sys.CommandOptions({cwd: None(), env: [('KEY', 'this-value-exceeds-the-small-environment-budget')]}))",
        SysLimits {
            bytes: 16,
            ..Default::default()
        },
        &CancellationToken::default(),
    );
    assert!(
        matches!(value, Value::Variant("Err", ref values) if matches!(&values[0], Value::String(s) if s.contains("Environment byte limit")))
    );
    let module = Program::compile(MODULE_SOURCE).unwrap();
    let program =
        Program::compile("import sys; if false { sys.exec_with('/bin/cat', [], '', 42) }").unwrap();
    assert!(program.validate_modules(&[("sys", &module)]).is_err());
}

#[test]
fn duplicate_scoped_keys_use_the_last_value_without_double_charging() {
    let first = "a".repeat(100);
    let last = "b".repeat(120);
    let source = format!(
        "import sys; fn main()->Result[str,str] {{ let options = sys.CommandOptions({{cwd: None(), env: [('KEY', '{first}'), ('KEY', '{last}')]}}); let output = sys.exec_with('/bin/sh', ['-c', 'printf \"$KEY\"'], '', options)?; return Ok(output.stdout) }}; main()"
    );
    assert_eq!(
        run(
            &source,
            SysLimits {
                bytes: 128,
                ..Default::default()
            },
            &CancellationToken::default()
        ),
        Value::Variant("Ok", vec![Value::String(last)])
    );
}
