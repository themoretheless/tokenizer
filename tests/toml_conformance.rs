//! Conformance checks backed by project-owned TOML fixtures.

#![cfg(feature = "toml")]

use std::{fs, path::Path};

use themoretheless_tokenizer::toml::parse;

#[test]
fn toml_fixture_conformance() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toml");
    let valid = fixture_paths(&root.join("valid"));
    let invalid = fixture_paths(&root.join("invalid"));

    assert_eq!(valid.len(), 20, "unexpected number of valid TOML fixtures");
    assert_eq!(
        invalid.len(),
        20,
        "unexpected number of invalid TOML fixtures"
    );

    let mut failures = Vec::new();

    for path in &valid {
        let source = read_fixture(path);
        let parsed = parse(&source);
        if !parsed.is_valid() {
            failures.push(format!(
                "expected valid TOML: {} (diagnostics: {:?})",
                path.display(),
                parsed.diagnostics()
            ));
        }
    }

    for path in &invalid {
        let source = read_fixture(path);
        if parse(&source).is_valid() {
            failures.push(format!("expected invalid TOML: {}", path.display()));
        }
    }

    assert!(
        failures.is_empty(),
        "TOML fixture failures ({}/{}):\n{}",
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
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "toml")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn read_fixture(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {} as UTF-8: {error}", path.display()))
}
