//! Minimal file runner: rush script.r [name=number ...]. Polygon results print SVG.
use std::{env, fs, io::Write as _, process::ExitCode};
use themoretheless_tokenizer_rush::{
    CancellationToken, ExecutionLimits, Program, Value, analyze, analyze_calls, analyze_names,
    builtin_catalog,
};

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
    let first = arguments
        .next()
        .ok_or("Usage: rush [--check|--fmt|--fmt-check] script.r [name=number ...] [--steps N] [--depth N] [--items N] [--string-bytes N]")?;
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
    let mut limit_options = std::collections::HashSet::new();
    let mut values = Vec::new();
    let mut module_sources = Vec::new();
    let mut registered = std::collections::HashSet::new();
    while let Some(argument) = arguments.next() {
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
        module_programs.push(Program::compile(contents).map_err(|error| error.message)?);
    }
    let modules = module_sources
        .iter()
        .zip(&module_programs)
        .map(|((name, _, _), program)| (name.as_str(), program))
        .collect::<Vec<_>>();
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
        Program::compile(&source)
            .map_err(|error| error.message)?
            .validate_modules(&modules)
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
        let names = names.into_iter().collect::<Vec<_>>();
        let mut messages = Vec::new();
        for (file, contents) in std::iter::once((path.as_str(), source.as_str())).chain(
            module_sources
                .iter()
                .map(|(_, path, source)| (path.as_str(), source.as_str())),
        ) {
            let checked_names = analyze_names(contents, &names);
            let calls = analyze_calls(contents);
            messages.extend(
                checked_names
                    .diagnostics
                    .iter()
                    .chain(&calls.diagnostics)
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
    let program = Program::compile(&source).map_err(|error| error.message)?;
    let value = program
        .run_with_limits(
            limits,
            &CancellationToken::default(),
            &inputs,
            &[],
            &modules,
        )
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
            ExitCode::FAILURE
        }
    }
}
