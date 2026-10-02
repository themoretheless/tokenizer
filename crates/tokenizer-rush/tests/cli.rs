use std::process::Command;

#[test]
fn file_runner_exports_parameterized_flower_and_reports_errors() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/scripts/flower.r");
    let output = Command::new(env!("CARGO_BIN_EXE_rush"))
        .args([path, "radius=30", "petals=6", "time=0"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("<svg"));
    let output = Command::new(env!("CARGO_BIN_EXE_rush"))
        .arg(path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown name: radius"));
}

#[test]
fn check_mode_validates_without_running_and_reports_source_location() {
    let path = std::env::temp_dir().join(format!("rush-check-{}.r", std::process::id()));
    std::fs::write(&path, "while true {}\nsin(1,2)\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rush"))
        .arg("--check")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(":2:1: argument-count"), "{error}");
    assert!(error.contains("sin(1,2)\n^"));
    std::fs::write(&path, "while true {}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rush"))
        .arg("--check")
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn file_modules_work_in_run_and_check_modes() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/scripts/");
    let source = format!("{root}modular-flower.r");
    let module = format!("curves={root}curves.r");
    for check in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rush"));
        if check {
            command.arg("--check");
        }
        let output = command
            .args([&source, "--module", &module])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(if check {
            "checks passed"
        } else {
            "<svg"
        }));
    }
}

#[test]
fn suite_reports_all_files_and_fails_on_empty_or_failed_suite() {
    let root = std::env::temp_dir().join(format!("rush-suite-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_rush"))
            .arg("--test")
            .arg(&root)
            .output()
            .unwrap()
    };
    assert!(!run().status.success());
    std::fs::write(root.join("a-test.r"), "assert(false, 'expected failure')").unwrap();
    std::fs::write(root.join("b_test.r"), "assert(true)").unwrap();
    std::fs::write(root.join("ignored.r"), "assert(false)").unwrap();
    let output = run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 1 failed"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected failure"));
    std::fs::write(root.join("a-test.r"), "assert(true)").unwrap();
    let output = run();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 passed; 0 failed"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn formatting_prints_without_executing_or_overwriting_and_supports_check_mode() {
    let path = std::env::temp_dir().join(format!("rush-format-{}.r", std::process::id()));
    let source = "while true {} // not executed\n";
    std::fs::write(&path, source).unwrap();
    let run = |flag: &str| {
        Command::new(env!("CARGO_BIN_EXE_rush"))
            .arg(flag)
            .arg(&path)
            .output()
            .unwrap()
    };
    let output = run("--fmt");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    assert!(!run("--fmt-check").status.success());
    std::fs::write(&path, output.stdout).unwrap();
    assert!(run("--fmt-check").status.success());
    std::fs::write(&path, "fn bad(").unwrap();
    let failed = run("--fmt");
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failed.stderr).contains(":1:"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn file_runner_accepts_explicit_step_and_depth_limits() {
    let path = std::env::temp_dir().join(format!("rush-limits-{}.r", std::process::id()));
    std::fs::write(
        &path,
        "fn count(n) { if n == 0 { return 0 }; return count(n-1) }; count(4)",
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rush"))
            .arg(&path)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["--steps", "1000", "--depth", "64"]).status.success());
    for args in [vec!["--steps", "0"], vec!["--depth", "2"]] {
        let output = run(&args);
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(message.contains("Execution limit exceeded"), "{message}");
        assert!(message.contains(":1:"), "{message}");
        assert!(output.stdout.is_empty());
    }
    for args in [
        vec!["--depth", "65"],
        vec!["--steps", "-1"],
        vec!["--steps", "1.5"],
        vec!["--depth"],
        vec!["--steps", "3", "--steps", "4"],
        vec!["--unknown"],
    ] {
        assert!(!run(&args).status.success(), "{args:?}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rush"))
        .arg("--check")
        .arg(&path)
        .args(["--steps", "10"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not used by --check"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn test_runner_forwards_limits_to_each_script() {
    let directory = std::env::temp_dir().join(format!("rush-suite-limits-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("value-test.r");
    std::fs::write(&file, "1 + 2").unwrap();
    for (steps, success) in [("0", false), ("100", true)] {
        let output = Command::new(env!("CARGO_BIN_EXE_rush"))
            .arg("--test")
            .arg(&directory)
            .args(["--steps", steps, "--depth", "64"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(if success {
                "1 passed; 0 failed"
            } else {
                "0 passed; 1 failed"
            })
        );
    }
    std::fs::remove_file(file).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn data_limits_apply_to_files_and_test_suites() {
    let directory = std::env::temp_dir().join(format!("rush-data-limits-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("data-test.r");
    std::fs::write(&path, "[\"é\", \"é\" + \"é\"]").unwrap();
    for suite in [false, true] {
        for (items, bytes, success, message) in [
            ("2", "4", true, ""),
            ("1", "4", false, "Collection item limit exceeded"),
            ("2", "3", false, "String byte limit exceeded"),
        ] {
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_rush"));
            if suite {
                command.arg("--test").arg(&directory);
            } else {
                command.arg(&path);
            }
            let output = command
                .args(["--items", items, "--string-bytes", bytes])
                .output()
                .unwrap();
            assert_eq!(output.status.success(), success, "{output:?}");
            if !success {
                let combined = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(combined.contains(message), "{combined}");
            }
        }
    }
    for option in ["--items", "--string-bytes"] {
        for tail in [
            vec![option],
            vec![option, "-1"],
            vec![option, "2", option, "3"],
        ] {
            assert!(
                !std::process::Command::new(env!("CARGO_BIN_EXE_rush"))
                    .arg(&path)
                    .args(tail)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_rush"))
            .arg("--check")
            .arg(&path)
            .args([option, "2"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not used by --check"));
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn closed_output_reports_io_error_without_panicking() {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let path = std::env::temp_dir().join(format!("rush-output-error-{}.r", std::process::id()));
    std::fs::write(&path, "polygon([vec2(0,0),vec2(1,0),vec2(0,1)])").unwrap();
    let (writer, reader) = UnixStream::pair().unwrap();
    // Shutdown also affects descriptors briefly inherited by concurrent child launches.
    reader.shutdown(std::net::Shutdown::Both).unwrap();
    drop(reader);
    let fd: OwnedFd = writer.into();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rush"))
        .arg(&path)
        .stdout(std::process::Stdio::from(fd))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("Cannot flush output") || error.contains("Cannot write output"),
        "{error}"
    );
    assert!(!error.contains("panicked"), "{error}");
    std::fs::remove_file(path).unwrap();
}
#[test]
fn strict_check_requires_a_complete_function_contract() {
    let path = std::env::temp_dir().join(format!("rush-strict-{}.r", std::process::id()));
    std::fs::write(&path, "fn identity(x){return x}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rush"))
        .args(["--check", path.to_str().unwrap(), "--strict"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dynamic-contract"));
    std::fs::write(&path, "fn next(n){return n+1}").unwrap();
    assert!(
        Command::new(env!("CARGO_BIN_EXE_rush"))
            .args(["--check", path.to_str().unwrap(), "--strict"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::remove_file(path).unwrap();
}
