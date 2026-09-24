# Structure review: SOLID, DRY, modularity, pluggability

**Status:** measured audit of the workspace as it stands — `sync-main-stack-v2` at `030d2ae` ("Add
family data, catalog endpoint, and hand-written CSS and Markdown engines"). Every file:line citation
below resolves in that commit; re-verify them after further engine work lands.
**Baseline:** 34,429 lines of Rust; 59 workspace members = `tokenizer-core` + 57 engine crates + `wasm`, plus the root facade package
**Comparison target:** this project's own contract — `docs/plugin-api-design.md` hard rules (design §9), deny list (design §15), testing contract (design §12), per-crate doc contract (design §13), performance budget (design §14)

This document measures structure against declared intent. It is a review, not a design: the
mechanisms proposed in §9 are deliberately minimal-cost and stay inside the design contract's
§9 hard rules and §15 deny list. Section 6 records where this review's own first pass was wrong, and
§8 records decisions already taken so they are not relitigated.

Reference convention: **`design §N`** means a section of `docs/plugin-api-design.md`; a bare `§N`
means a section of this document.

---

## 1. Headline

| Finding | Measure |
|---|---|
| Mechanically identical code in the profile-driven engine crates | **6,338 lines = 74.3%** of those crates = **18.4% of all Rust** |
| Collapsing them to a table-driven crate would remove | **~8,650 lines (25.1%), 144 files** |
| Published-API cost of that collapse | **0** — nothing is on crates.io, including the facade and core |
| Lines of non-data logic in 49/49 engine crates | **0** |
| Rust-side proof that the 49 engines work | **none runs in the merge gate** |
| Correctness defects found underneath the duplication | **2** (§4) |

The duplication is real but it is not the costly part. The costly part is that the same absence of a
single choke point produced four divergent host-adapter preambles, one of which enforces no input
limit on any method — and that a working lossless sweep over all 57 engines exists only in
JavaScript, outside the Rust CI job that gates merges.

---

## 2. DRY — confirmed by hash, not by eye

The 49 profile-driven crates (`abap`, `ada`, `assembly`, `bash`, `c`, `clojure`, `cobol`, `cpp`,
`csharp`, `dart`, `delphi`, `dlang`, `elixir`, `erlang`, `fortran`, `fsharp`, `go`, `graphql`,
`groovy`, `haskell`, `java`, `javascript`, `julia`, `kotlin`, `lisp`, `lua`, `matlab`, `mongo`,
`nim`, `objectivec`, `ocaml`, `perl`, `php`, `powershell`, `prolog`, `python`, `r`, `ruby`, `rust`,
`scala`, `scheme`, `solidity`, `sql`, `swift`, `typescript`, `verilog`, `vhdl`, `visualbasic`,
`zig`) total 8,534 lines.

| Check | Result | Method |
|---|---|---|
| `impl HostLanguage for Host` block | md5-identical **49/49** (43 lines) | extract that range with `awk`, hash it → one hash across 49 files |
| `#[cfg(test)]` module | md5-identical **49/49** | same |
| Tail from `/// Lossless lexer` to EOF | exactly **101 lines in all 49** = 4,949 lines | line count + hash |
| Fixed scaffolding | +1,389 | 9 header/`use` lines + item skeleton |
| `macro_rules!` / `#[macro_export]` in repo | **0** | grep |
| Generic `HostLanguage` implementor over a profile | **none** | grep |

The only variance between the 49 files is data: `keywords`, `types`, comment markers, five booleans,
and five `full_descriptor` arguments. `crates/tokenizer-zig/src/lib.rs:102-202` is the canonical copy.

The duplicated test module is worse than duplication: every one of the 49 asserts on
`"fn main() { return 1; }"` and `"function f(x) { return x + 1; }"` — a Rust-shaped and a JS-shaped
string, in COBOL, Ada, ABAP and VHDL. The `parse_smoke` assertion passes generically because the
parser is recovering.

## 3. A dead abstraction where the guards should live

`crates/tokenizer-core/src/plugin_host.rs:54` `run_highlight_host` and `:70` `run_diagnose_host`
implement exactly the preamble every engine hand-inlines: dialect check → input-size check → convert.
**Consumers outside core: 0** (they are re-exported at `core/src/lib.rs:51` and never called).

Instead, engine crates inline `require_default_dialect` **159** times and
`opts.limits.exceeds_input_bytes` + a hand-built `HostError::InputTooLarge` **106** times. With no
choke point, five different preamble shapes evolved across 57 engines:

| Shape | Crates |
|---|---|
| `lex`: dialect+limits, `semantic_tokens`: dialect+limits, `diagnose`: dialect only | all 49 template crates, plus yaml, toml |
| `lex`: dialect+limits, `semantic_tokens`: delegates, `diagnose`: dialect+limits | css, markdown |
| **no limits check on any method** | **html, xml** (`crates/tokenizer-html/src/lib.rs:41-64`) |
| `impl HostLanguage` lives in the facade, not the crate | json, url (`src/plugins.rs:12-134`, `:136-220`) |

`diagnose()` checks no size limit in any of the 49 — while the dead `run_diagnose_host` does. The
dead helper was the correct code.

Also inside the dead-code radius: `langkit.rs` (575 lines) has **zero consumers outside
`tokenizer-core`** — verified by reference count across `crates/`, `src/`, `wasm/`, `tests/`,
`benches/`, `fuzz/`. Only `highlight_markup` (`langkit.rs:347`) survives, called once from
`markup_full.rs:103`. `CHANGELOG.md` explains the cause: css, markdown, yaml and toml were rewritten
as hand-written format engines, the migration off `langkit` completed, and the scaffolding was left
behind and exported as `pub`. So the frequently-proposed "unify the two competing lexers" item
collapses into a deletion, not a merge.

Public-surface hygiene in core: **358 `pub` lines, 0 `pub(crate)`**. AST types, wire DTOs and
identity tables share one flat namespace, and fullkit types are reachable through three paths (flat
root re-export, module path, and `markup_full.rs:400-402`).

## 4. Two correctness defects that duplication concealed

Both are violations of hard rule 8 (capability honesty) and were found by reading the copy-paste
divergences.

**4.1 `SEMANTIC` never fires for case-insensitive languages.** The lexer classifies keywords
case-insensitively via a lowercased `HashSet<String>` (`fullkit.rs:297-306`, `:445-456`), while the
parser matches with `at_text`, an **exact** comparison (`fullkit.rs:575-579`), against hardcoded
literals `"function" | "fn" | "def" | "func" | "fun"` and
`"class" | "struct" | "interface" | "trait" | "type" | "enum"` (`fullkit.rs:629-645`). For source
using `FUNCTION` / `PROCEDURE` — Fortran, COBOL, Ada, ABAP, VHDL, SQL, Visual Basic — item detection
never triggers, so the semantic layer returns lexer kinds unchanged. All 49 advertise
`FULL_CAPS = LEX|PARSE|SEMANTIC|VALIDATE`. Note also that parser behavior is decoupled from profile
data: a language whose function keyword is `proc` (Nim) works only by accident of the literal list.

The same class of over-claim is structural in `html` and `xml`: both advertise `FULL_CAPS`
(`tokenizer-html/src/lib.rs:32`, `tokenizer-xml/src/lib.rs:32`) while their `semantic_tokens` delegates
to `lex` verbatim (`:51`), i.e. no parser-aware layer exists behind the advertised bit.

**4.2 `analyze_full_host` discards diagnostics** (`fullkit.rs:1356-1365`):
`if sem.diagnostics.is_empty() { sem.diagnostics = parsed.diagnostics; }` — when the semantic pass
reports anything at all, lexer and parser diagnostics are dropped. Therefore
`semantic_tokens().diagnostics ⊅ diagnose()` today, which is an invariant any future test must first
*decide*, not merely observe.

Adjacent, and not a correctness defect: against design §14 ("no lexeme `String`", "`&'static str` on
hot paths"), `lex_full` allocates one `String` per identifier token (`fullkit.rs:449`) plus two
`HashSet<String>` per call (`:297-306`) — a per-call cost, since every engine rebuilds its profile
lookup on every invocation.

## 5. Modularity, dependency direction, pluggability

**Dependency direction is cleaner than it looks.** Across all 58 crate manifests the only edge is
`engine → core`, and core has zero Cargo dependencies. There is no "core depends on leaves" layering
violation in the build sense.

What core does carry is the *enumeration* of leaves as data: 57 `LanguageId` constants
(`language.rs:13-69`), `FORMAT_IDS` / `TOP20_IDS` / `NEXT20_IDS` (48 entries, `family.rs:56-113`),
and `JSON_FULL` / `URL_TODAY` naming specific languages inside a generic bitset module
(`capabilities.rs:20,31`) — roughly 115 lines. `LanguageId` is `pub struct LanguageId(pub &'static str)`
(`language.rs:10`), not an enum, so **no exhaustiveness is ever checked by the compiler**.

**What an "engine" actually is.** In 49/49 crates there is not one line of logic beyond data: exactly
nine top-level items. `FullProfile` (`fullkit.rs:80-92`) has 8 fields. There is no heredoc, no string
interpolation, no PHP-in-HTML, no C preprocessor, and indentation is one boolean. The 49 "independent
plugins" are 49 rows of a table, exploded across crates.

**Cost of adding one language** (measured on zig): 3 new files + **11 edit sites across 6 files** —
root `Cargo.toml` ×4 (`:51`, `:132`, `:192`, `:219`), `core/language.rs:60`, `src/lib.rs:128-129`,
`src/plugins.rs:414-417`, `playground/src/languages.js`, `playground/tests/language-cases.js`,
`docs/languages.md`, `tools/gen_language_crates.py` ×6. `wasm/src/lib.rs` and `src/web_bridge.rs`
need **zero** edits because they are registry-driven through `catalog()`. That is the genuinely
pluggable layer, and it sits above the engines, not inside them.

**`tools/gen_language_crates.py` (703 lines) is a live footgun.** `main()` (`:697-699`) iterates all
56 `LANGS` rows including xml, html, css, yaml, toml and markdown, and `gen_crate()` writes
`Cargo.toml` (`:450`), `README.md` (`:465`) and `src/lib.rs` (`:694`) **unconditionally**, with no
hand-written-engine guard. Running it today silently reverts the four rewritten format engines to
langkit-era templates.

## 6. Where the review contradicts its own first pass

Recorded so the plan is not built on a wrong premise.

1. **"The only drift guard is `assert_eq!(len, 57)`" — false.** `playground/tests/catalog.test.js:13-18`
   does real set equality of ids computed from the compiled wasm `catalog()`, and `:36-45` checks
   dialects per language.
2. **"No registry-driven lossless sweep exists" — false in form.**
   `playground/tests/tokenizer.test.js:89-118` iterates `ALL_LANGUAGE_IDS` and runs every case through
   `assertCase`, which asserts losslessness (`:70-74`), span sanity (`:76+`), `minTokens`,
   `expectValid`, `expectKinds` and `maxDiagnostics` — 229 cases across 57 ids, plus a syntax-layer and
   a semantic-layer smoke test per id. But it is a Node test requiring wasm artifacts
   (`playground/.wasm-test/tokenizer_wasm.js`), and `.github/workflows/ci.yml` has no playground job —
   its test step is `cargo test --locked --workspace --all-targets --all-features` (`:90`). So coverage
   exists outside the merge gate. This changes P0 from "write tests" to "port the existing sweep to
   native, where CI runs it".
3. **"Core depends on leaves" — wrong axis.** 0 Cargo edges; the issue is enumeration, not layering. A
   separate `tokenizer-languages` crate is **not recommended**: it converts 0 edges into 9-57 and moves
   ~115 lines at the cost of editing 57 manifests. Revisit only if a consumer outside this repo needs
   the id list.
4. **"Two competing lexers" — inverted.** `langkit` is dead, not a competitor (§3).

## 7. Contracts the project wrote and did not implement

| Contract (design doc) | Fact |
|---|---|
| §13 every language crate exports `pub const LANGUAGE_ID` | **0 of 57** |
| §13 `pub const DIALECT_SCOPE: DialectScope` | type `DialectScope` **does not exist** in any `.rs` |
| §12 `tests/fixtures/<id>/{valid,invalid,recover}/` | only **json** and **toml**; no crate has a `tests/` dir |
| §12 shared harness (LosslessOracle / ConformanceSuite / EngineSmoke) | **absent** natively (see §6.2) |
| §12 `PropertyRunner` beyond JSON | `tests/property_differential.rs` is JSON-only |
| Rule 12 wire names are explicit kebab-case, never `Debug` | violated by `debug_kebab` (`src/plugins.rs:497-507`, `format!("{:?}")`) |
| design §10 deferred: shared c-family crates "after rule-of-three" | rule-of-three satisfied **49 times** |

## 8. Decisions taken

- **Crate boundaries are open**, and the collapse to a table-driven `tokenizer-full` is accepted.
  Semver cost is **0**: `index.crates.io/th/em/themorelessly-tokenizer{,-core,-zig}` all return 404
  (control `se/rd/serde` returns 200), so `publish = ["crates-io"]` in 58 manifests is a registry
  whitelist, not a publication record. Decisively, `RELEASING.md:42` states the workspace publishes
  **four** crates — core, json, url, facade — and `publish.yml` names one package (`:53`, `:66`,
  `:99`). 55 of the 57 engine crates are already treated by the release pipeline as non-deliverable,
  and cost 49× manifest + README + version bump + `Cargo.lock` entry + docs.rs metadata.
- **P2 and P3 remain separate PRs.** Reviewable diff and bisectability outweigh the duplicated
  mechanical edit of 49 `lib.rs` files.
- **§4.1 is fixed rather than downgraded** (P7): profile-driven `fn_keywords` / `class_keywords` plus a
  case-folding `at_text`. Consequence: P7 lands per language family with re-baselined expectations,
  because it changes output for all 57 engines.
- **`HostLanguage` is not split.** See §9.5.

## 9. Proposed mechanisms

### 9.1 `FullEngine` as the single `HostLanguage` implementor

```rust
// crates/tokenizer-core/src/engine.rs (~170 lines)
pub struct FullEngine {
    descriptor: &'static LanguageDescriptor,
    profile:    &'static FullProfile,
    lookup:     OnceLock<KeywordLookup>,   // per-call allocation removal — what a macro alone cannot do
}
impl FullEngine {
    pub const fn new(descriptor: &'static LanguageDescriptor, profile: &'static FullProfile) -> Self;
    fn gate(&self, source: &str, opts: &HostAnalysisOptions) -> Result<(), HostError>; // dialect, then limits
}
impl HostLanguage for FullEngine { /* the only impl for fullkit engines */ }
```

Feasibility, checked: `full_descriptor` is a `const fn` (`plugin_host.rs:109`); every `FullProfile`
field is `&'static [...]`, `Option<&'static str>` or `bool`, so `static PROFILE` compiles today (a
`const fn FullProfile::new` is needed in place of the non-const `Default` at `fullkit.rs:94-107`);
`OnceLock::new()` in a `static` is already used at `src/plugins.rs:461`; object safety is untouched, so
`&'static FullEngine` still coerces to `&'static dyn HostLanguage` and `RegistryBuilder::register`
(`registry.rs:44`) is unchanged. `gate()` keeps dialect-before-limits so `HostError` precedence is
byte-preserved, meaning the only output change in P2 is `diagnose` gaining a size check.

A `#[macro_export]` `define_full_engine!` (~55 lines) emits data and statics only, never an impl.
Hygiene requirements that matter here: every path in the expansion is `$crate::`-prefixed (which is
what lets the 7-line, 17-item `use` prologue in all 49 crates disappear — itself a drift vector), item
names are passed in as identifiers rather than invented, and no `#[cfg(test)]` module is generated.

| Design | 49 crates `lib.rs` | Net workspace |
|---|---|---|
| today | 8,534 | 34,429 |
| macro only, 49 crates kept | 2,980 (−5,554) | −5,384 (−15.6%) |
| struct + macro + one `tokenizer-full` table | — | **−8,650 (−25.1%), −144 files** |

Macro-only does **not** fix §3: it leaves 49 copies of the guard in the binary and no single place to
test it. The struct additionally removes the per-call `String`/`HashSet` churn noted in §4.

### 9.2 Collapse, with acceptance criteria

Keep all 49 feature names (design rule 14) and all 57 `tokenizer::zig`-style facade aliases
(`src/lib.rs:59-…`) by emitting them from the table. `engine_version` is unchanged because all 58
manifests are `0.4.0`. The playground catalog needs no change (registry-driven, and `catalog.test.js`
compares ids, not crate names). The one real consumer-facing break is `cargo add …-zig` from git,
documented at `README.md:28`.

### 9.3 One table, generated outward

One row per language drives `static PROFILE` / `DESCRIPTOR` / `ENGINE`, the four typed free functions,
and **`LANGUAGE_ID` + `DIALECT_SCOPE`** — implementing design §13 for the first time. A `langctl` binary
(`--emit=cargo|js|md|fixtures|check`) generates the root `Cargo.toml` feature block,
`playground/src/languages.js`, `docs/languages.md` and fixtures, retiring the generator footgun by
deletion. `Cargo.toml` cannot read Rust constants (the comment at `family.rs:6-7` concedes this), so
generation is the honest answer; linkme/global registration stays forbidden per design §9/§15.

Completeness replaces `assert_eq!(len, 57)` with five set-equality checks over the filesystem, no new
dependencies: table ↔ registry, feature keys ↔ `dep:` ↔ members, feature ↔ registration site,
id ↔ playground + fixtures, presets ⊆ `ALL_IDS` and disjoint. Net effect: 11 edit sites across 6 files
→ one table row plus one emit.

### 9.4 Tests before any dedup

Do not author a corpus. `playground/tests/language-cases.js` (534 lines) already holds 57 language keys
carrying 229 language-real cases with expectations (`zig → 'pub fn main() void {}'`,
`nim → 'proc f(x: int): int = x + 1'`, `solidity → 'contract C { function f() public {} }'`) covering
design §12's classes: valid, incomplete, comments, unicode, invalid-with-expectation, empty. Emit each
`case.source` to `tests/fixtures/<id>/…` plus a `cases.json` carrying
`{mode, layer, expectValid, expectKinds, minTokens, maxDiagnostics}` — 229 fixtures, zero hand-written.
Then invert the source of truth so `language-cases.js` becomes generated output.

Native harness invariants, which are also what makes the dedup safe: pin the intended
`diagnose = dedup_by(code, span)(lex.diagnostics ++ parse.diagnostics)` rule *first* (otherwise a
golden diff blesses §4.2); `semantic.valid == lex.valid` for all inputs; span coverage; and a per-id
snapshot of `Family::of` / `presets_of` / `extensions`, since those are what the playground renders.

### 9.5 Interface segregation without breaking `dyn`

`HostLanguage` has 10 methods, 4 required (`host.rs:179-246`). Splitting into `Lexing` /
`SemanticAnalysis` / `Diagnostics` forces either three parallel lists that `registry.rs:94-138` must
re-associate by id (+~60 lines and a new agreement invariant), or a two-level trait object that moves
capability discovery out of the descriptor and into the type system — against design §4.3 and hard
rule 8.

Instead give `semantic_tokens` and `diagnose` default bodies routing through `lex`, reducing required
methods 4 → 2, deleting 7 stub bodies (`yaml:139-145`, `css`, `markdown`, `toml`, `url`, `html`, `xml`),
and making §3's missing-guard class structurally impossible. All 57 engines override today, so this is
a pure deletion. Real segregation belongs on the typed `LanguageEngine` side (design §4.1), where
monomorphization and exhaustiveness are free.

## 10. Phased plan

`noop` = provably byte-identical output; `BEHAV` = observable change.

| Phase | Content | Δ lines | Blast radius | Type |
|---|---|---|---|---|
| **P0** | Golden harness: `tests/engine_matrix.rs`, fixture emit from `language-cases.js`, `--dump-corpus`, pin the `diagnose` invariant (`#[ignore]` + note) | **+1,050** | additive, no production code | noop |
| **P1** | Delete dead `langkit` (keep `highlight_markup`, relocate to `markup_full`) and dead `plugin_host` helpers; `pub(crate)` the 5 zero-reference items | **−590** | core only | noop |
| **P2** | `FullEngine` + `KeywordLookup` + `define_full_engine!`; port the 49 crates, **crates kept** | core +225 / engines **−5,554** | 49 crates + core; `plugins.rs` untouched | noop for `lex`/`semantic`; **BEHAV** for `diagnose` |
| **P3** | Collapse 49 → `tokenizer-full` with the `language!` table; facade aliases via macro; delete 49 × (Cargo.toml, README, src) | **−3,100** (cumulative −8,650) | workspace manifest, facade, CI, docs | noop |
| **P4** | `langctl`; `LanguageId::ALL_IDS`; `tests/language_matrix.rs` replacing `assert_eq!(len, 57)`; design §13 realized; generator footgun deleted | tools −703 | tools, root `Cargo.toml`, playground, docs | noop |
| **P5** | 8 host conversions → 1 helper; `parse_body` replacing the 6 parser triads (`fullkit.rs:695,732,826,852,925,941`); named diagnostic codes; single severity path | −215 | core + 4 format engines + `plugins.rs` | noop |
| **P6** | `HostLanguage` defaults (§9.5), delete 7 stubs | −70 | core + 7 engines | **BEHAV, guarded**: html/xml gain limits |
| **P7** | Fix §4.1: profile `fn_keywords`/`class_keywords`, case-folding `at_text`, fix §4.2 diagnostic merge | +2 fields, −30 | semantic output of all 57 | **BEHAV, intentional** |

Dependencies: **P0 blocks everything**; P1 → P5; P2 → P3 → P4; P6 after P2 (defaults are safe only once
`FullEngine` overrides both); P7 after P0 and P4 (expectations must be per-id, not per-crate).

PR boundaries: `P0a` dump-corpus bin · `P0b` fixtures + sweep · `P1` · `P2a` core mechanism ·
`P2b…` the 49 crates in **seven wave-sized PRs of 7** so a regression bisects to a family · `P3` ·
`P4` · `P5a/b/c` · `P6` · `P7x` per family.

### Who is affected by `diagnose` gaining a size limit (P2)

`HostLanguage::diagnose` has five call sites, one of them production: `src/api.rs:139`, already gated
at `src/api.rs:133-138` with an identical `InputTooLarge` — so facade consumers see **zero** change, the
error just raises one frame earlier. The playground never calls `diagnose` (`src/web_bridge.rs:18-27`
routes to `tokenize_layer`; `analyze_host` is itself gated at `src/plugins.rs:488`). Only a direct
`ENGINE.diagnose(>16 MiB)` caller is affected — 0 in-repo and 0 reachable, since nothing is published.
Record as `### Fixed`, not `### Breaking`. The larger and real change from P2/P6 is html and xml going
from no `InputLimits` enforcement on any path to enforcement on all three.

## 11. Verification

1. **Gate for P1-P7:** `cargo run --bin tokenizer-web-bridge -- --dump-corpus > base.json`, re-dump after
   each noop phase and require byte equality (`cmp`); same for `--catalog`.
2. **Phase tests:** `cargo test --locked --workspace --all-targets --all-features` (the `ci.yml:90`
   merge gate), `cargo clippy --all-features -D warnings`, `cargo doc` clean of intra-doc bitrot after
   P1 deletions, `cargo tree` diff for P3 (exactly −49 packages), `cargo package --locked` on the four
   release crates.
3. **P2/P3:** `cargo expand` spot-check on three crates; assert zero fixtures exceed 16 MiB (which is
   what makes the P2 `diagnose` change provably invisible); benchmark `lex_full` for the expected win
   from the cached `KeywordLookup`.
4. **P4:** locally delete one feature and confirm `cargo test --test language_matrix` **names the
   missing id** rather than merely failing a count.
5. **P6:** feed html/xml a >16 MiB input and expect `InputTooLarge` — that is the intended change.
6. **P7:** re-baseline `catalog.test.js:59-79` floors and `language-cases.js`
   `maxDiagnostics`/`expectKinds`, one family per PR, with a measured kind delta in the PR body.
7. **Playground `npm test`:** after P0 only — it cross-checks against the Rust catalog and must stay
   green over the new fixtures.

## 12. Out of scope here

The `tokenizer-languages` extraction (rejected in §6.3); streaming/incremental, multi-file, CST
comment attachment, LSP modifiers (all design §10 deferred); and any change to the legacy JSON surface,
which design rule 11 freezes.

## 13. Open question for the project owner

Should README §1 and `docs/plugin-api-design.md` §1 keep advertising per-language crates as the plugin
unit, or be restated as "data-driven catalog + one shipped engine core"? The second phrasing matches
what the release pipeline already does (`RELEASING.md:42`: four crates).
