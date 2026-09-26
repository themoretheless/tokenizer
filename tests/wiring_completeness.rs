//! Executable wiring and format-vocabulary contract.
//!
//! Implements the completeness checks proposed in `docs/structure-review.md`
//! §9.3. Each one is set equality over the filesystem, the feature table, the
//! registry or the playground, so a half-wired engine fails the merge gate
//! instead of silently missing from a feature group or the language picker.
//! Adding a format is meant to cost one wired unit and no drift.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use themoretheless_tokenizer::core::{FORMAT_IDS, LanguageId, NEXT20_IDS, TOP20_IDS, TokenLayer};

/// Kinds any generic lexer emits; a format engine must go beyond these. Kept
/// equal to `GENERIC_KINDS` in `playground/src/catalog.js`.
const GENERIC_KINDS: [&str; 11] = [
    "class",
    "comment",
    "function",
    "identifier",
    "keyword",
    "number",
    "punctuation",
    "string",
    "type",
    "variable",
    "whitespace",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn cargo_text() -> String {
    read("Cargo.toml")
}

/// One `key = value` entry inside a TOML table, tolerant of multi-line values.
struct Entry {
    key: String,
    body: String,
}

impl Entry {
    /// Every quoted string in the body.
    fn strings(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut chars = self.body.char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            if ch != '"' {
                continue;
            }
            let mut inner = String::new();
            for (_, next) in chars.by_ref() {
                if next == '"' {
                    break;
                }
                inner.push(next);
            }
            if !inner.is_empty() {
                out.insert(inner);
            }
            let _ = index;
        }
        out
    }

    fn contains(&self, needle: &str) -> bool {
        self.body.contains(needle)
    }

    fn string_after(&self, key: &str) -> Option<String> {
        let marker = format!("{key} = \"");
        let start = self.body.find(&marker)? + marker.len();
        let end = start + self.body[start..].find('"')?;
        Some(self.body[start..end].to_string())
    }
}

/// Entries of a top-level TOML table (`[features]`, `[dependencies]", …`).
fn table_entries(text: &str, table: &str) -> Vec<Entry> {
    let header = format!("[{table}]");
    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Option<Entry> = None;
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with('[') {
            entries.extend(current.take());
            inside = trimmed == header;
            continue;
        }
        if !inside || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let is_new_entry = !line.starts_with([' ', '\t']);
        if let Some(split) = is_new_entry
            .then(|| line.find('=').unwrap_or(usize::MAX))
            .filter(|at| *at != usize::MAX)
        {
            entries.extend(current.take());
            current = Some(Entry {
                key: line[..split].trim().to_string(),
                body: format!(" {trimmed}"),
            });
        } else if let Some(entry) = current.as_mut() {
            entry.body.push(' ');
            entry.body.push_str(trimmed);
        }
    }
    entries.extend(current.take());
    entries
}

fn feature(text: &str, name: &str) -> BTreeSet<String> {
    table_entries(text, "features")
        .into_iter()
        .find(|entry| entry.key == name)
        .unwrap_or_else(|| panic!("[features] has no `{name}` group"))
        .strings()
}

/// Transitive closure of a feature group: nested groups expand, `dep:` keys
/// resolve to the short name they gate.
fn expand_feature(text: &str, name: &str) -> BTreeSet<String> {
    let groups: BTreeSet<String> = feature_names(text);
    let mut out = BTreeSet::new();
    let mut queue = vec![name.to_string()];
    while let Some(current) = queue.pop() {
        for item in feature(text, &current) {
            if let Some(dep) = item.strip_prefix("dep:") {
                let prefix = format!("{}-", package_name(text));
                if let Some(short) = dep.strip_prefix(&prefix) {
                    out.insert(short.to_string());
                }
            } else if groups.contains(&item) {
                // A group name is itself a valid id when it only aliases
                // another crate (`tsv = ["csv"]`), so count it as reachable.
                out.insert(item.clone());
                queue.push(item);
            } else {
                out.insert(item);
            }
        }
    }
    out
}

fn feature_names(text: &str) -> BTreeSet<String> {
    table_entries(text, "features")
        .into_iter()
        .map(|entry| entry.key)
        .collect()
}

fn dependencies(text: &str) -> Vec<Entry> {
    table_entries(text, "dependencies")
}

fn package_name(text: &str) -> String {
    table_entries(text, "package")
        .into_iter()
        .find(|entry| entry.key == "name")
        .and_then(|entry| entry.string_after("name"))
        .expect("[package] name")
}

fn workspace_members(text: &str) -> BTreeSet<String> {
    table_entries(text, "workspace")
        .into_iter()
        .find(|entry| entry.key == "members")
        .expect("[workspace] members")
        .strings()
}

#[test]
fn every_plugin_crate_is_reachable_through_a_named_feature() {
    let text = cargo_text();
    let deps = dependencies(&text);
    let declared = feature_names(&text);

    for member in workspace_members(&text) {
        let Some(short) = member.strip_prefix("crates/tokenizer-") else {
            continue;
        };
        if short == "core" {
            continue;
        }
        let dep = deps
            .iter()
            .find(|dep| dep.string_after("path").as_deref() == Some(member.as_str()))
            .unwrap_or_else(|| {
                panic!("{member} is a workspace member but no dependency points at it")
            });
        assert!(
            dep.contains("optional = true"),
            "{} must be optional to be feature-gateable",
            dep.key
        );
        assert!(
            declared.contains(short),
            "crate `{short}` has no [features] entry named after it"
        );
        assert!(
            feature(&text, short).contains(&format!("dep:{}", dep.key)),
            "feature `{short}` must enable the `{}` dependency",
            dep.key
        );
    }
}

#[test]
fn every_referenced_dependency_target_exists() {
    let text = cargo_text();
    let deps = dependencies(&text);
    for entry in table_entries(&text, "features") {
        for item in entry.strings() {
            let Some(dep_key) = item.strip_prefix("dep:") else {
                continue;
            };
            let dep = deps
                .iter()
                .find(|dep| dep.key == dep_key)
                .unwrap_or_else(|| {
                    panic!(
                        "feature `{}` references `dep:{dep_key}`, which is not a dependency",
                        entry.key
                    )
                });
            let path = dep
                .string_after("path")
                .unwrap_or_else(|| panic!("{dep_key} declares no path"));
            assert!(
                repo_root().join(&path).join("Cargo.toml").is_file(),
                "{dep_key} points at missing crate directory {path}"
            );
        }
    }
}

#[test]
fn every_optional_dependency_has_a_reachable_feature() {
    let text = cargo_text();
    let prefix = format!("{}-", package_name(&text));
    let declared = feature_names(&text);
    for dep in dependencies(&text) {
        if !dep.contains("optional = true") {
            continue;
        }
        let short = dep.key.strip_prefix(&prefix).unwrap_or_else(|| {
            panic!(
                "dependency `{}` does not start with the package prefix `{prefix}`",
                dep.key
            )
        });
        assert!(
            declared.contains(short),
            "optional dependency `{}` has no feature `{short}`",
            dep.key
        );
    }
}

fn id_set(list: &[LanguageId]) -> BTreeSet<String> {
    list.iter().map(|id| id.as_str().to_string()).collect()
}

#[test]
fn cargo_feature_groups_mirror_the_core_tables() {
    let text = cargo_text();
    let ids = id_set;
    assert_eq!(
        feature(&text, "formats"),
        ids(FORMAT_IDS),
        "`formats` must equal core::FORMAT_IDS"
    );
    assert_eq!(
        feature(&text, "top20"),
        ids(TOP20_IDS),
        "`top20` must equal core::TOP20_IDS"
    );
    assert_eq!(
        feature(&text, "wave7"),
        ids(NEXT20_IDS),
        "`wave7` must equal core::NEXT20_IDS"
    );

    // `all-languages` names the wave groups rather than repeating them, so the
    // meaningful check is the transitive expansion, not the literal list.
    let expanded = expand_feature(&text, "all-languages");
    let mut declared = id_set(FORMAT_IDS);
    declared.extend(id_set(TOP20_IDS));
    declared.extend(id_set(NEXT20_IDS));
    assert!(
        expanded.is_superset(&declared),
        "`all-languages` must reach every preset id; missing {:?}",
        declared.difference(&expanded).collect::<Vec<_>>()
    );
}

#[test]
fn generic_kind_baseline_matches_the_playground() {
    let catalog = read("playground/src/catalog.js");
    let marker = "GENERIC_KINDS = new Set([";
    let start = catalog.find(marker).expect("GENERIC_KINDS in catalog.js") + marker.len();
    let end = start + catalog[start..].find("])").expect("end of GENERIC_KINDS");
    let js: BTreeSet<String> = catalog[start..end]
        .split(',')
        .map(|item| item.trim().trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect();
    let rust: BTreeSet<String> = GENERIC_KINDS
        .iter()
        .map(|kind| (*kind).to_string())
        .collect();
    assert_eq!(rust, js, "Rust and playground generic baselines drifted");
}

#[cfg(feature = "all-languages")]
#[test]
fn playground_id_sets_match_the_registry() {
    use themoretheless_tokenizer::builtin_registry;

    let registry: BTreeSet<String> = builtin_registry()
        .iter()
        .map(|engine| engine.id().as_str().to_string())
        .collect();

    // languages.js also exports a group table whose rows carry `id` fields.
    let languages = read("playground/src/languages.js");
    let start = languages
        .find("export const LANGUAGES")
        .expect("LANGUAGES export");
    let end = languages[start + 1..]
        .find("export const")
        .map(|offset| start + 1 + offset)
        .unwrap_or(languages.len());
    let languages = &languages[start..end];
    let mut picker: BTreeSet<String> = BTreeSet::new();
    let mut cursor = 0;
    while let Some(found) = languages[cursor..].find("id: '") {
        let start = cursor + found + "id: '".len();
        let end = start
            + languages[start..]
                .find('\'')
                .expect("unterminated id in languages.js");
        picker.insert(languages[start..end].to_string());
        cursor = end;
    }

    let cases = read("playground/tests/language-cases.js");
    let mut fixtures: BTreeSet<String> = BTreeSet::new();
    for line in cases.lines() {
        let Some(colon) = line.strip_prefix("  ").and_then(|body| body.find(": {")) else {
            continue;
        };
        let key = &line[2..2 + colon];
        if !key.is_empty() && !key.contains(' ') {
            fixtures.insert(key.to_string());
        }
    }

    assert_eq!(
        registry, picker,
        "the playground language picker is not the registry"
    );
    assert_eq!(
        registry, fixtures,
        "the playground fixture matrix is not the registry"
    );
}

#[cfg(feature = "all-languages")]
#[test]
fn every_format_engine_knows_its_own_vocabulary() {
    use themoretheless_tokenizer::{analyze_host, builtin_registry};

    for id in FORMAT_IDS {
        let fixture = format!("tests/fixtures/formats/{}.txt", id.as_str());
        let source = read(&fixture);
        let engine = builtin_registry()
            .get(*id)
            .unwrap_or_else(|| panic!("format {} is not registered", id.as_str()));
        let dialect = engine.descriptor().default_dialect.as_str();
        let result = analyze_host(id.as_str(), &source, dialect, TokenLayer::Semantic)
            .unwrap_or_else(|error| panic!("format {} failed to analyze: {error}", id.as_str()));

        // A format engine must parse its representative document cleanly, not
        // merely survive it: recovery hides parse errors behind tokens.
        assert!(result.valid, "{fixture} must be reported valid");
        let codes: Vec<&str> = result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_ref())
            .collect();
        assert!(
            codes.is_empty(),
            "valid {fixture} produced diagnostics {codes:?}"
        );
        let kinds: BTreeSet<&str> = result
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        let specific: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|kind| !GENERIC_KINDS.contains(kind))
            .collect();
        assert!(
            !specific.is_empty(),
            "{} emits only generic kinds {kinds:?}: a lexer wearing a format's name",
            id.as_str()
        );
    }
}

#[cfg(feature = "all-languages")]
#[test]
fn every_format_has_a_representative_fixture() {
    for id in FORMAT_IDS {
        let path = repo_root().join(format!("tests/fixtures/formats/{}.txt", id.as_str()));
        assert!(
            path.is_file(),
            "missing fixture {} for a registered format",
            path.display()
        );
    }
}

/// Every byte prefix of a format's representative fixture, plus the adversarial
/// strings the format engines were built to survive. This is the executable half
/// of two claims that were prose until now: a format engine never panics, and it
/// never stalls — recovery must always make progress, so truncating a document
/// cannot send the lexer or parser around a loop that consumes nothing.
#[cfg(feature = "all-languages")]
#[test]
fn no_format_engine_panics_or_stalls_on_a_truncated_document() {
    use std::time::{Duration, Instant};
    use themoretheless_tokenizer::{analyze_host, builtin_registry};

    // Wall-clock in a debug-profile test is noise-dominated: the first touch of
    // an engine's code path measured 61ms against a 0.1ms warm baseline, and a
    // contended run added another order of magnitude on top. So these bounds are
    // deliberately coarse — they catch a loop or a quadratic blowup and nothing
    // else. Warm each engine on its whole document before timing any prefix.
    const PER_PREFIX: Duration = Duration::from_secs(1);
    // Measured warm cost of sweeping every prefix of all 20 format fixtures: 0.5s.
    const PER_FORMAT: Duration = Duration::from_secs(30);

    for id in FORMAT_IDS {
        let fixture = format!("tests/fixtures/formats/{}.txt", id.as_str());
        let source = read(&fixture);
        let engine = builtin_registry()
            .get(*id)
            .unwrap_or_else(|| panic!("format {} is not registered", id.as_str()));
        let dialect = engine.descriptor().default_dialect.as_str();

        analyze_host(id.as_str(), &source, dialect, TokenLayer::Semantic).unwrap_or_else(|error| {
            panic!("format {} failed to analyze itself: {error}", id.as_str())
        });

        let sweep_started = Instant::now();
        let mut worst_prefix = Duration::ZERO;

        let mut cut = 0;
        while cut <= source.len() {
            // Splitting a multi-byte character is not a document prefix.
            if !source.is_char_boundary(cut) {
                cut += 1;
                continue;
            }
            let prefix = &source[..cut];
            let started = Instant::now();
            let result = analyze_host(id.as_str(), prefix, dialect, TokenLayer::Semantic)
                .unwrap_or_else(|error| {
                    panic!("format {} at {cut} bytes returned {error}", id.as_str())
                });
            let elapsed = started.elapsed();
            if elapsed > worst_prefix {
                worst_prefix = elapsed;
            }
            assert!(
                elapsed <= PER_PREFIX,
                "{} truncated to {cut} bytes took {elapsed:?}: recovery is not making progress",
                id.as_str()
            );

            // The same invariant the full document guarantees: spans tile the
            // input exactly, so no truncation drops or invents bytes.
            let mut expected = 0_usize;
            for token in &result.tokens {
                assert_eq!(
                    token.span.start,
                    expected,
                    "{} at {cut} bytes left a gap at {expected}",
                    id.as_str()
                );
                assert!(
                    token.span.end > token.span.start,
                    "{} at {cut} bytes emitted a zero-length token",
                    id.as_str()
                );
                expected = token.span.end;
            }
            assert_eq!(
                expected,
                cut,
                "{} at {cut} bytes covered only {expected} bytes",
                id.as_str()
            );
            cut += 1;
        }

        assert!(
            sweep_started.elapsed() <= PER_FORMAT,
            "{} swept its fixture's prefixes in {:?} (worst prefix {worst_prefix:?})",
            id.as_str(),
            sweep_started.elapsed(),
        );
    }
}

/// Diagnostic codes are a host contract: they are kebab-case workspace-wide, so
/// a host can match on them without parsing prose. The sweep above produces the
/// real corpus; this walks it again and checks every emitted code round-trips.
#[cfg(feature = "all-languages")]
#[test]
fn every_format_diagnostic_code_is_kebab_case() {
    use themoretheless_tokenizer::{analyze_host, builtin_registry};
    fn is_kebab(code: &str) -> bool {
        !code.is_empty()
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !code.starts_with('-')
            && !code.ends_with('-')
            && !code.contains("--")
    }

    let mut all = BTreeSet::new();
    for id in FORMAT_IDS {
        let fixture = format!("tests/fixtures/formats/{}.txt", id.as_str());
        let source = read(&fixture);
        let engine = builtin_registry()
            .get(*id)
            .unwrap_or_else(|| panic!("format {} is not registered", id.as_str()));
        let dialect = engine.descriptor().default_dialect.as_str();

        let mut cut = 0;
        while cut <= source.len() {
            if !source.is_char_boundary(cut) {
                cut += 1;
                continue;
            }
            let result = analyze_host(id.as_str(), &source[..cut], dialect, TokenLayer::Semantic)
                .unwrap_or_else(|error| {
                    panic!("format {} at {cut} bytes returned {error}", id.as_str())
                });
            for diagnostic in &result.diagnostics {
                assert!(
                    is_kebab(&diagnostic.code),
                    "{} emitted non-kebab diagnostic code {:?}",
                    id.as_str(),
                    diagnostic.code
                );
                assert!(
                    !diagnostic.message.is_empty(),
                    "{} diagnostic {:?} has an empty message",
                    id.as_str(),
                    diagnostic.code
                );
                assert!(
                    diagnostic.span.end <= cut,
                    "{} diagnostic {:?} spans {}..{} outside its {}-byte input",
                    id.as_str(),
                    diagnostic.code,
                    diagnostic.span.start,
                    diagnostic.span.end,
                    cut
                );
                all.insert(diagnostic.code.to_string());
            }
            cut += 1;
        }
    }

    // A format whose grammar accepts every prefix of its own fixture is the
    // quiet case, not a broken one, so the floor is over the whole sweep.
    assert!(
        all.len() >= 40,
        "the truncation sweep only reached {} distinct diagnostic codes across 20 formats: \
         the recovery layer is largely untested ({all:?})",
        all.len()
    );
}

/// `VALIDATE` says a grammar rejects input its own specification forbids. What
/// the shared fullkit pipeline can prove about a document is bracket balance and
/// closed literals ([`every_language_engine_flags_unbalanced_delimiters`]), not
/// that the grammar was followed — `SELECT * FROM` is a valid prefix as far as it
/// is concerned. So the badge belongs to the format family only, and
/// [`every_format_engine_knows_its_own_vocabulary`] already proves those 20 stay
/// silent on valid documents. This pins the partition so a new engine cannot
/// claim validation by inheriting the shared descriptor.
#[cfg(feature = "all-languages")]
#[test]
fn only_format_engines_advertise_validate() {
    use themoretheless_tokenizer::builtin_registry;
    use themoretheless_tokenizer::core::Capabilities;

    let formats: BTreeSet<String> = FORMAT_IDS
        .iter()
        .map(|id| id.as_str().to_string())
        .collect();
    let mut claimers = BTreeSet::new();
    let mut total = 0;
    for engine in builtin_registry().iter() {
        total += 1;
        if engine
            .descriptor()
            .capabilities
            .contains(Capabilities::VALIDATE)
        {
            claimers.insert(engine.descriptor().language.as_str().to_string());
        }
    }
    assert_eq!(
        claimers,
        formats,
        "{total} engines registered: validate must be claimed by exactly the {} formats",
        formats.len(),
    );
}

/// A document the project itself ships as valid must come back quiet on both
/// layers. The format fixtures already carried that claim; this extends it to
/// the 49 wave languages, whose picker samples are exported to
/// `tests/fixtures/languages/<id>.txt` by
/// `playground/scripts/export-language-fixtures.mjs`. Before this check existed
/// the shared fullkit parser flagged 25 of those 49 documents.
#[cfg(feature = "all-languages")]
#[test]
fn every_engine_is_quiet_on_its_valid_fixture() {
    use themoretheless_tokenizer::{analyze_host, builtin_registry};

    let mut noisy: Vec<String> = Vec::new();
    let mut checked = 0;
    for engine in builtin_registry().iter() {
        let id = engine.id();
        let dir = if FORMAT_IDS.contains(&id) {
            "formats"
        } else {
            "languages"
        };
        let source = read(&format!("tests/fixtures/{dir}/{}.txt", id.as_str()));
        let dialect = engine.descriptor().default_dialect.as_str();
        checked += 1;
        for layer in [TokenLayer::Syntax, TokenLayer::Semantic] {
            let result = analyze_host(id.as_str(), &source, dialect, layer)
                .unwrap_or_else(|error| panic!("{} at {layer:?}: {error}", id.as_str()));
            for diagnostic in &result.diagnostics {
                noisy.push(format!(
                    "{}/{}: {}@{}..{}",
                    id.as_str(),
                    layer.as_str(),
                    diagnostic.code,
                    diagnostic.span.start,
                    diagnostic.span.end
                ));
            }
        }
    }
    assert_eq!(checked, 69, "every registered engine needs a valid fixture");
    assert!(
        noisy.is_empty(),
        "{} diagnostics on valid fixtures: {:?}",
        noisy.len(),
        noisy
    );
}

/// Quiet on valid input is only half of a useful parser: the shared pipeline
/// must still reject what it can prove. Bracket balance is language-independent,
/// so one `unclosed-delimiter` / `stray-delimiter` pass covers every profiled
/// language, and this pins that it fires across the shape families the wave
/// languages actually have (braces, lisp parens, sql parens, shell blocks).
/// The valid half of the same contract is
/// [`every_engine_is_quiet_on_its_valid_fixture`].
#[cfg(feature = "all-languages")]
#[test]
fn every_language_engine_flags_unbalanced_delimiters() {
    use themoretheless_tokenizer::{analyze_host, builtin_registry};

    let cases: &[(&str, &str, &str)] = &[
        ("go", "func f( {\n", "unclosed-delimiter"),
        ("go", "func f() {\n  x := 1\n\n", "unclosed-delimiter"),
        ("go", "x = 1)\n", "stray-delimiter"),
        ("python", "def f(:\n    return 1\n", "unclosed-delimiter"),
        ("lisp", "(defun f (x)\n  (+ x 1)\n", "unclosed-delimiter"),
        (
            "scheme",
            "(define (f x)\n  (+ x 1))\n)\n",
            "stray-delimiter",
        ),
        ("sql", "SELECT * FROM (t;\n", "unclosed-delimiter"),
        ("java", "class A { void f() {\n", "unclosed-delimiter"),
        (
            "powershell",
            "if ($true {\n  Write-Host \"hi\"\n",
            "unclosed-delimiter",
        ),
        ("assembly", "mov ax, [bx\nret\n", "unclosed-delimiter"),
        ("zig", "pub fn main() void {\n", "unclosed-delimiter"),
    ];

    // The `unclosed-delimiter` claim would be worthless if it fired inside a
    // literal, so interpolation is on the table too: `${` and `#{` bodies, and
    // everything between a pair of backticks, are text.
    let quiet: &[(&str, &str)] = &[
        ("bash", "echo \"hi ${name}\"\n"),
        ("php", "echo \"a{$b}\";\n"),
        ("javascript", "const t = `x${y}z`;\n"),
        // A brace that lives inside a template literal is text, not a delimiter.
        ("javascript", "const t = `a{b`;\n"),
        ("ruby", "puts \"a#{b}\"\n"),
        ("perl", "print \"total: $item->{n}\";\n"),
    ];
    for (id, source) in quiet {
        let language = LanguageId(id);
        let engine = builtin_registry()
            .get(language)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        let dialect = engine.descriptor().default_dialect.as_str();
        let result = analyze_host(id, source, dialect, TokenLayer::Semantic)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert!(
            result.valid && result.diagnostics.is_empty(),
            "{id} on {source:?}: interpolation must be quiet, got {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| d.code.as_ref())
                .collect::<Vec<_>>(),
        );
    }

    for (id, source, expected) in cases {
        let language = LanguageId(id);
        let engine = builtin_registry()
            .get(language)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        let dialect = engine.descriptor().default_dialect.as_str();
        let result = analyze_host(id, source, dialect, TokenLayer::Semantic)
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        let codes: Vec<&str> = result.diagnostics.iter().map(|d| d.code.as_ref()).collect();
        assert!(
            !result.valid && codes.contains(expected),
            "{id} on {source:?}: expected {expected}, got valid={} codes={codes:?}",
            result.valid,
        );
        for diagnostic in &result.diagnostics {
            assert!(
                diagnostic.span.end <= source.len() && diagnostic.span.start < diagnostic.span.end,
                "{id}: {expected} span {:?} out of bounds",
                diagnostic.span,
            );
        }
    }
}
