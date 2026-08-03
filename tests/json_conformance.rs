//! Conformance checks backed by project-owned JSON fixtures.

use std::{fs, path::Path};

use themoretheless_tokenizer::json::parse;

#[test]
fn json_fixture_conformance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/json");
    let valid = fixture_paths(&root.join("valid"));
    let invalid = fixture_paths(&root.join("invalid"));

    assert_eq!(valid.len(), 15, "unexpected number of valid JSON fixtures");
    assert_eq!(
        invalid.len(),
        20,
        "unexpected number of invalid JSON fixtures"
    );

    let mut failures = Vec::new();

    for path in &valid {
        let source = read_fixture(path);
        let parsed = parse(&source);
        if !parsed.is_valid() {
            failures.push(format!(
                "expected valid JSON: {} (diagnostics: {:?})",
                path.display(),
                parsed.diagnostics()
            ));
        }
    }

    for path in &invalid {
        let source = read_fixture(path);
        if parse(&source).is_valid() {
            failures.push(format!("expected invalid JSON: {}", path.display()));
        }
    }

    assert!(
        failures.is_empty(),
        "JSON fixture failures ({}/{}):\n{}",
        failures.len(),
        valid.len() + invalid.len(),
        failures.join("\n")
    );
}

fn fixture_paths(directory: &Path) -> Vec<std::path::PathBuf> {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to read an entry in {}: {error}",
                        directory.display()
                    )
                })
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn read_fixture(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {} as UTF-8: {error}", path.display()))
}
