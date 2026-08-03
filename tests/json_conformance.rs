//! Opt-in conformance test for <https://github.com/nst/JSONTestSuite>.
//!
//! Set `JSON_TEST_SUITE` to either the repository root or its
//! `test_parsing` directory to run the corpus locally.

use std::{env, fs, path::PathBuf};

use themoretheless_tokenizer::json::parse;

#[test]
fn json_test_suite_conformance() {
    let Some(configured_path) = env::var_os("JSON_TEST_SUITE") else {
        eprintln!("JSON_TEST_SUITE is not set; skipping JSONTestSuite conformance test");
        return;
    };

    let configured_path = configured_path.into_string().unwrap_or_else(|path| {
        panic!("JSON_TEST_SUITE is not valid UTF-8: {path:?}");
    });
    assert!(
        !configured_path.trim().is_empty(),
        "JSON_TEST_SUITE must not be empty"
    );

    let configured_path = PathBuf::from(configured_path);
    assert!(
        configured_path.is_dir(),
        "JSON_TEST_SUITE must point to a directory, got {}",
        configured_path.display()
    );

    let nested_test_directory = configured_path.join("test_parsing");
    let test_directory = if nested_test_directory.is_dir() {
        nested_test_directory
    } else {
        configured_path
    };

    let mut cases = fs::read_dir(&test_directory)
        .unwrap_or_else(|error| {
            panic!(
                "failed to read JSONTestSuite directory {}: {error}",
                test_directory.display()
            )
        })
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to read an entry in {}: {error}",
                        test_directory.display()
                    )
                })
                .path()
        })
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    cases.sort();

    let mut accepted = 0_usize;
    let mut rejected = 0_usize;
    let mut implementation_defined = 0_usize;
    let mut failures = Vec::new();

    for path in cases {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let expectation = file_name.as_bytes().get(..2);
        if !matches!(expectation, Some(b"y_" | b"n_" | b"i_")) {
            continue;
        }

        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let source = std::str::from_utf8(&bytes);

        match expectation {
            Some(b"y_") => {
                accepted += 1;
                match source {
                    Ok(source) if parse(source).is_valid() => {}
                    Ok(source) => failures.push(format!(
                        "expected valid JSON: {} (diagnostics: {:?})",
                        path.display(),
                        parse(source).diagnostics()
                    )),
                    Err(error) => failures.push(format!(
                        "expected UTF-8 JSON in {}: {error}",
                        path.display()
                    )),
                }
            }
            Some(b"n_") => {
                rejected += 1;
                if source.is_ok_and(|source| parse(source).is_valid()) {
                    failures.push(format!("expected invalid JSON: {}", path.display()));
                }
            }
            Some(b"i_") => {
                implementation_defined += 1;
                if let Ok(source) = source {
                    let _ = parse(source);
                }
            }
            _ => unreachable!("the case prefix was filtered above"),
        }
    }

    let counts = (accepted, rejected, implementation_defined);
    assert_eq!(
        counts,
        (95, 188, 35),
        "unexpected JSONTestSuite corpus contents in {} (expected y=95, n=188, i=35)",
        test_directory.display()
    );
    let total = accepted + rejected + implementation_defined;
    assert!(
        total > 0,
        "no y_, n_, or i_ JSON cases found in {}",
        test_directory.display()
    );
    assert!(
        failures.is_empty(),
        "JSONTestSuite failures ({}/{total}; y={accepted}, n={rejected}, i={implementation_defined}):\n{}",
        failures.len(),
        failures.join("\n")
    );

    eprintln!(
        "JSONTestSuite passed {total} cases (y={accepted}, n={rejected}, i={implementation_defined})"
    );
}
