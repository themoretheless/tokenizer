//! Minimal file runner: rush script.r [name=number ...]. Polygon results print SVG.
use std::{env, fs, io::Write as _, process::ExitCode};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, ModelProgram, Program, ReplCommand, ReplOutcome,
    ReplSession, Value, analyze, analyze_calls, analyze_names, builtin_catalog,
};

fn run_repl() -> Result<(), String> {
    use std::io::{self, BufRead, Write};

    println!(
        "Rush Interactive Shell (Rush {})",
        env!("CARGO_PKG_VERSION")
    );
    println!("Type :help for guidance, :vars for active bindings, :exit or Ctrl+D to quit.\n");

    let stdin = io::stdin();
    let mut session = ReplSession::new()?;
    let mut stdout = io::stdout();

    let mut line_buf = String::new();
    loop {
        if session.is_accumulating() {
            print!("... ");
        } else {
            print!(">>> ");
        }
        stdout.flush().map_err(|e| e.to_string())?;

        line_buf.clear();
        let bytes_read = stdin
            .lock()
            .read_line(&mut line_buf)
            .map_err(|e| e.to_string())?;
        if bytes_read == 0 {
            println!();
            break;
        }

        let line = line_buf.trim_end_matches(&['\r', '\n'][..]);
        match session.eval_line(line) {
            ReplOutcome::Value(val) => {
                println!("{val}");
            }
            ReplOutcome::Void => {}
            ReplOutcome::Incomplete => {}
            ReplOutcome::Command(ReplCommand::Help) => {
                println!("Rush REPL Commands:");
                println!("  :help         Show this help information");
                println!("  :vars         List active user variables and values");
                println!("  :reset        Clear all variables and reset environment");
                println!("  :exit, quit   Exit the REPL session");
                println!();
                println!("Rush Syntax Highlights:");
                println!("  let x = 10                  Define constant");
                println!("  mut arr = [1, 2, 3]         Define mutable variable");
                println!("  arr[0] = 99                 Index/field mutation");
                println!("  json_parse(str)             Parse JSON into Rush data");
                println!("  json_stringify(val)         Serialize Rush data to JSON");
                println!("  fn f(x) {{ return x + 1 }}    Define function");
            }
            ReplOutcome::Command(ReplCommand::Reset) => {
                println!("Environment reset.");
            }
            ReplOutcome::Command(ReplCommand::Vars(vars)) => {
                if vars.is_empty() {
                    println!("No user-defined variables.");
                } else {
                    for (k, v) in vars {
                        println!("  {k} = {v}");
                    }
                }
            }
            ReplOutcome::Command(ReplCommand::Exit) => {
                break;
            }
            ReplOutcome::Error(err) => {
                eprintln!("{err}");
            }
        }
    }
    Ok(())
}

fn diagnostic(path: &str, source: &str, offset: usize, message: &str) -> String {
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &source[..offset];
    let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let end = source[offset..]
        .find('\n')
        .map_or(source.len(), |i| offset + i);
    let column = source[start..offset].chars().count() + 1;
    format!(
        "{path}:{line}:{column}: {message}\n{}\n{}^",
        &source[start..end],
        " ".repeat(column - 1)
    )
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let Some(first) = arguments.next() else {
        return run_repl();
    };
    if first == "--repl" || first == "-i" {
        return run_repl();
    }
    if first == "--help" || first == "-h" {
        println!("Usage: rush [options] [script.r] [name=number ...] [-- script-args ...]\n");
        println!("Options:");
        println!(
            "  --repl, -i               Launch interactive REPL session (default if no script provided)"
        );
        println!("  --check                  Analyze and check script without executing");
        println!("  --fmt                    Format script source to stdout");
        println!("  --fmt-check              Verify script formatting matches canonical style");
        println!("  --model                  Instantiate a geometric/model program");
        println!("  --quiet                  Suppress final expression output");
        println!("  --module <name=path>     Register module source path");
        println!("  --steps <N>              Set maximum execution step budget");
        println!("  --process-timeout-ms <N> Set maximum subprocess execution timeout");
        return Ok(());
    }
    let quiet_prefix = first == "--quiet";
    let first = if quiet_prefix {
        arguments
            .next()
            .ok_or("Expected script path after --quiet")?
    } else {
        first
    };
    if first == "--fmt" || first == "--fmt-check" {
        let path = arguments
            .next()
            .ok_or("Expected a script path after formatting option")?;
        if arguments.next().is_some() {
            return Err("Formatting accepts one script path".into());
        }
        let source = fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
        let formatted = themoretheless_tokenizer_rush::format_source(&source)
            .map_err(|error| diagnostic(&path, &source, error.span.start, &error.message))?;
        if first == "--fmt-check" {
            if formatted != source {
                return Err(format!("{path}: formatting differs; run rush --fmt {path}"));
            }
        } else {
            print!("{formatted}");
        }
        return Ok(());
    }
    if first == "--model" {
        let path = arguments
            .next()
            .ok_or("Expected a model path after --model")?;
        let source = fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        let program = ModelProgram::compile(&source)
            .map_err(|e| diagnostic(&path, &source, e.span.start, &e.message))?;
        let token = CancellationToken::default();
        let limits = ExecutionLimits::new(1_000_000);
        let mut model = program
            .instantiate(limits, &token, &[])
            .map_err(|e| diagnostic(&path, &source, e.span.start, &e.message))?;
        let mut names = std::collections::HashSet::new();
        for argument in arguments {
            let (name, raw) = argument.split_once('=').ok_or("Expected name=number")?;
            let value = raw
                .parse::<f64>()
                .map_err(|_| "Expected a numeric parameter")?;
            if !value.is_finite() || !names.insert(name.to_owned()) {
                return Err(format!("Invalid or duplicate model parameter: {name}"));
            }
            model
                .set_parameter(name, Value::Number(value), limits)
                .map_err(|e| diagnostic(&path, &source, e.span.start, &e.message))?;
        }
        return write_output(model.output().clone());
    }
    if first == "--test" {
        let directory = arguments
            .next()
            .ok_or("Expected a directory after --test")?;
        return test_suite(&directory, arguments.collect());
    }
    let check_only = first == "--check";
    let path = if check_only {
        arguments
            .next()
            .ok_or("Expected a script path after --check")?
    } else {
        first
    };
    let source = fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
    let parsed = analyze(&source);
    if !parsed.is_valid() {
        let diagnostics = parsed
            .diagnostics
            .iter()
            .map(|d| {
                diagnostic(
                    &path,
                    &source,
                    d.span.start,
                    &format!("{}: {}", d.code, d.message),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("{path}: invalid program\n{diagnostics}"));
    }
    let mut limits = ExecutionLimits::new(1_000_000);
    let mut strict = false;
    let mut region_report = false;
    let mut limit_options = std::collections::HashSet::new();
    let mut values = Vec::new();
    let mut script_arguments = Vec::new();
    let mut quiet = quiet_prefix;
    let mut sys_limits = themoretheless_tokenizer_rush::sys::SysLimits::default();
    let mut module_sources = Vec::new();
    let mut registered = std::collections::HashSet::new();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            script_arguments.extend(arguments);
            break;
        }
        if argument == "--quiet" {
            quiet = true;
            continue;
        }
        if argument == "--process-timeout-ms" {
            if !limit_options.insert(argument.clone()) {
                return Err("Duplicate process timeout".into());
            }
            let timeout = arguments
                .next()
                .ok_or("Expected timeout milliseconds")?
                .parse::<u64>()
                .map_err(|_| "Invalid process timeout")?;
            sys_limits.timeout = std::time::Duration::from_millis(timeout);
            continue;
        }
        if argument == "--strict" {
            if strict {
                return Err("Duplicate option: --strict".into());
            }
            strict = true;
            continue;
        }
        if argument == "--region-report" {
            if region_report {
                return Err("Duplicate option: --region-report".into());
            }
            region_report = true;
            continue;
        }
        if matches!(
            argument.as_str(),
            "--steps" | "--depth" | "--items" | "--string-bytes"
        ) {
            if check_only {
                return Err("Execution limits are not used by --check".into());
            }
            if !limit_options.insert(argument.clone()) {
                return Err(format!("Duplicate option: {argument}"));
            }
            let raw = arguments
                .next()
                .ok_or_else(|| format!("Expected a nonnegative integer after {argument}"))?;
            let value = raw
                .parse::<usize>()
                .map_err(|_| format!("Expected a nonnegative integer after {argument}: {raw}"))?;
            match argument.as_str() {
                "--steps" => limits.steps = value,
                "--items" => limits.max_collection_items = value,
                "--string-bytes" => limits.max_string_bytes = value,
                "--depth" => {
                    if value > 64 {
                        return Err("Maximum evaluation depth cannot exceed 64".into());
                    }
                    limits.max_depth = value;
                }
                _ => unreachable!(),
            }
        } else if argument == "--module" {
            let spec = arguments
                .next()
                .ok_or("Expected name=path after --module")?;
            let (name, module_path) = spec.split_once('=').ok_or("Expected module name=path")?;
            if name.is_empty() || !registered.insert(name.to_owned()) {
                return Err("Empty or duplicate module name".into());
            }
            let contents = fs::read_to_string(module_path)
                .map_err(|error| format!("{module_path}: {error}"))?;
            module_sources.push((name.to_owned(), module_path.to_owned(), contents));
        } else if argument.starts_with("--") {
            return Err(format!("Unknown option: {argument}"));
        } else {
            values.push(argument);
        }
    }
    let arguments = values;
    let mut module_programs = Vec::new();
    for (_, module_path, contents) in &module_sources {
        let parsed = analyze(contents);
        if !parsed.is_valid() {
            return Err(parsed
                .diagnostics
                .iter()
                .map(|d| {
                    diagnostic(
                        module_path,
                        contents,
                        d.span.start,
                        &format!("{}: {}", d.code, d.message),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"));
        }
        module_programs.push(Program::compile(contents).map_err(|error| {
            diagnostic(module_path, contents, error.span.start, &error.message)
        })?);
    }
    let sys = themoretheless_tokenizer_rush::sys::SysHost::with_args(sys_limits, script_arguments);
    let sys_program = Program::compile(themoretheless_tokenizer_rush::sys::MODULE_SOURCE)
        .map_err(|e| format!("sys: {}", e.message))?;
    let hosts: Vec<_> = sys
        .registrations
        .iter()
        .map(|r| r.function.clone())
        .collect();
    let mut modules = module_sources
        .iter()
        .zip(&module_programs)
        .map(|((name, _, _), program)| (name.as_str(), program))
        .collect::<Vec<_>>();
    if !registered.contains("sys") {
        modules.push(("sys", &sys_program));
    }
    let inputs = arguments
        .iter()
        .map(|argument| {
            let (name, value) = argument
                .split_once('=')
                .ok_or_else(|| format!("Expected name=number: {argument}"))?;
            if name.is_empty() {
                return Err("Parameter name cannot be empty".into());
            }
            Ok((
                name,
                value
                    .parse::<f64>()
                    .map_err(|_| format!("Invalid number: {value}"))?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut names = std::collections::HashSet::new();
    for (name, value) in &inputs {
        if !value.is_finite()
            || !names.insert(*name)
            || builtin_catalog().iter().any(|(builtin, _)| builtin == name)
        {
            return Err(format!("Invalid or duplicate input parameter: {name}"));
        }
    }
    if check_only {
        let checked = Program::compile(&source)
            .map_err(|error| diagnostic(&path, &source, error.span.start, &error.message))?;
        (if strict {
            checked.validate_strict_modules(&modules)
        } else {
            checked.validate_modules(&modules)
        })
        .map_err(|error| {
            if let Some(module) = &error.module
                && let Some((_, module_path, contents)) =
                    module_sources.iter().find(|(name, _, _)| name == module)
            {
                diagnostic(module_path, contents, error.span.start, &error.message)
            } else {
                diagnostic(&path, &source, error.span.start, &error.message)
            }
        })?;
        let mut names = names.into_iter().collect::<Vec<_>>();
        names.extend(hosts.iter().map(|f| f.name));
        let mut messages = Vec::new();
        for (file, contents) in std::iter::once((path.as_str(), source.as_str())).chain(
            module_sources
                .iter()
                .map(|(_, path, source)| (path.as_str(), source.as_str())),
        ) {
            let checked_names = analyze_names(contents, &names);
            let calls = analyze_calls(contents);
            let host_calls = themoretheless_tokenizer_rush::analyze_host_calls(contents, &hosts);
            messages.extend(
                checked_names
                    .diagnostics
                    .iter()
                    .chain(&calls.diagnostics)
                    .chain(&host_calls.diagnostics)
                    .map(|d| {
                        diagnostic(
                            file,
                            contents,
                            d.span.start,
                            &format!("{}: {}", d.code, d.message),
                        )
                    }),
            );
        }
        if !messages.is_empty() {
            return Err(messages.join("\n"));
        }
        println!("{path}: checks passed");
        return Ok(());
    }
    let program = Program::compile(&source)
        .map_err(|error| diagnostic(&path, &source, error.span.start, &error.message))?;
    if strict {
        program.validate_strict_modules(&modules).map_err(|error| {
            if let Some(module) = &error.module
                && let Some((_, module_path, contents)) =
                    module_sources.iter().find(|(name, _, _)| name == module)
            {
                diagnostic(module_path, contents, error.span.start, &error.message)
            } else {
                diagnostic(&path, &source, error.span.start, &error.message)
            }
        })?;
    }
    let token = CancellationToken::default();
    let _signals = signal_cancellation(&token);
    let inputs: Vec<_> = inputs
        .iter()
        .map(|(name, value)| (*name, Value::Number(*value)))
        .collect();
    let instance = program
        .instantiate(limits, &token, &inputs, &[], &sys.registrations, &modules)
        .map_err(|error| {
            if let Some(module) = &error.module
                && let Some((_, module_path, contents)) =
                    module_sources.iter().find(|(name, _, _)| name == module)
            {
                diagnostic(module_path, contents, error.span.start, &error.message)
            } else {
                diagnostic(&path, &source, error.span.start, &error.message)
            }
        })?;
    if let Value::Variant("Err", error) = instance.initial_value() {
        return Err(format!("Script returned Err: {error:?}"));
    }
    if region_report {
        let stats = instance.region_stats();
        if stats.is_empty() {
            eprintln!("{path}: no regions executed");
        }
        for stats in stats {
            let name = stats.name.as_deref().unwrap_or("<anonymous>");
            eprintln!(
                "{path}: region {name}: {} entries, {} cells ({} reused), peak {} live, {} promoted",
                stats.entries, stats.allocated, stats.reused_slots, stats.peak_live, stats.promoted,
            );
            if stats.promoted > 0 {
                eprintln!(
                    "{path}: region {name}: {} cells escaped via promotion; the region does not bound their lifetimes",
                    stats.promoted,
                );
            }
            eprintln!(
                "{path}: region {name}: suggested budget {} -> region {name} ({}) {{ ... }}",
                stats.suggested_budget(),
                stats.suggested_budget(),
            );
        }
    }
    if quiet {
        return Ok(());
    }
    write_output(instance.initial_value().clone())
}

fn write_output(value: Value<'_>) -> Result<(), String> {
    let stdout = std::io::stdout();
    let mut output = IoTextWriter {
        writer: std::io::BufWriter::new(stdout.lock()),
        error: None,
    };
    let result = match value {
        Value::Mesh(mesh) => mesh
            .write_obj(&mut output)
            .map_err(|_| "OBJ output write failed"),
        Value::Polygon(polygon) => polygon.write_svg(&mut output),
        other => std::fmt::Write::write_fmt(&mut output, format_args!("{other:?}\n"))
            .map_err(|_| "Text output write failed"),
    };
    if let Some(error) = output.error {
        return Err(format!("Cannot write output: {error}"));
    }
    result?;
    output
        .writer
        .flush()
        .map_err(|error| format!("Cannot flush output: {error}"))
}

struct IoTextWriter<W> {
    writer: W,
    error: Option<std::io::Error>,
}
impl<W: std::io::Write> std::fmt::Write for IoTextWriter<W> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if self.error.is_some() {
            return Err(std::fmt::Error);
        }
        self.writer.write_all(text.as_bytes()).map_err(|error| {
            self.error = Some(error);
            std::fmt::Error
        })
    }
}
fn test_suite(directory: &str, arguments: Vec<String>) -> Result<(), String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| format!("{directory}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && (name.ends_with("-test.r") || name.ends_with("_test.r"))
        {
            files.push(entry.path());
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(format!("{directory}: no *-test.r or *_test.r files found"));
    }
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let mut passed = 0;
    let mut failed = 0;
    for file in files {
        let output = std::process::Command::new(&executable)
            .arg(&file)
            .args(&arguments)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            passed += 1;
            println!("PASS {}", file.display());
        } else {
            failed += 1;
            println!("FAIL {}", file.display());
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
    }
    println!("{passed} passed; {failed} failed");
    if failed > 0 {
        Err("Rush test suite failed".into())
    } else {
        Ok(())
    }
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            #[cfg(unix)]
            {
                let signal = SIGNAL.load(std::sync::atomic::Ordering::Relaxed);
                if signal != 0 {
                    return ExitCode::from((128 + signal) as u8);
                }
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(unix)]
static SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
#[cfg(unix)]
extern "C" fn interrupted(signal: libc::c_int) {
    SIGNAL.store(signal, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(unix)]
struct SignalGuard {
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
#[cfg(unix)]
impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(unix)]
fn signal_cancellation(token: &CancellationToken) -> SignalGuard {
    // The CLI owns these process-wide handlers. The handler only stores a
    // lock-free atomic; process cleanup runs on the normal execution thread.
    unsafe {
        libc::signal(libc::SIGINT, interrupted as *const () as libc::sighandler_t);
        libc::signal(
            libc::SIGTERM,
            interrupted as *const () as libc::sighandler_t,
        );
    }
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = stopped.clone();
    let token = token.clone();
    let worker = std::thread::spawn(move || {
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if SIGNAL.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                token.cancel();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    SignalGuard {
        stopped,
        worker: Some(worker),
    }
}
#[cfg(not(unix))]
fn signal_cancellation(_: &CancellationToken) {}
