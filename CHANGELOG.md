# Changelog

All notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Format family as first-class core data: `core::family` (`Family`,
  `FORMAT_IDS`, `TOP20_IDS`, `NEXT20_IDS`, `Preset`, `presets_of`) plus a
  `formats` cargo feature; registry presets are derived from these tables.
- `catalog()` endpoint (bin `--catalog`, WASM `wasm_catalog`, dev
  `/api/catalog`) exposing per-engine family, presets, dialects, extensions
  and capabilities so the playground consumes registry truth, not restated lists.
- Playground presets (top20 / next20 / formats), two-in-one compare view with a
  KIND DIFF pane, and a measured engine-depth matrix with batch run.
- TOML and YAML host adapters now emit their own spec vocabularies
  (`bare-key`, `plain-scalar`, `value-indicator`, …) instead of collapsing into
  the generic core syntax kinds, so format-specific tokens reach the UI.
- `markdown` and `css` are now hand-written recovering format engines with
  their own `SyntaxKind` vocabularies (34 kinds each) instead of generic
  fullkit wrappers: measured format-specific kinds went 0 -> 8 (markdown) and
  0 -> 7 (css), and spurious "unexpected token" diagnostics on *valid*
  documents went 9 -> 0 and 3 -> 0. Both advertise honest `LEX|PARSE|SEMANTIC|VALIDATE`.
- Formats batch 1: `json5`, `jsonl`, `csv` and `tsv` join the format family,
  taking `FORMAT_IDS` / the `formats` feature from 8 ids to 12 and the
  playground picker from 57 to 61 engines.
  - `tokenizer-csv` (new crate) is one delimiter-parameterised engine: RFC 4180
    quoting with embedded delimiters and CRLF, `header-field` / `quoted-field` /
    `integer-field` / `decimal-field` / `boolean-field` / `quote` /
    `escaped-quote` / `delimiter` / `record-break` kinds, and `ragged-row`,
    `unclosed-quote` and `text-after-closing-quote` recovery. Measured on its
    fixture: 9 format-specific kinds for csv, 7 for tsv, 0 diagnostics.
    `tsv` is a cargo alias of `csv`, so four ids cost one new crate.
  - `json5` and `jsonl` extend `tokenizer-json` instead of duplicating its
    4,574-line engine: JSON5 adds lexer/parser options (bare-word keys,
    single-quoted strings, hex and point-padded numbers, `Infinity`/`NaN`,
    trailing commas, string line continuations) and JSONL adds a strict
    per-line record framer whose `record-break` spans have no JSON analogue.
    Measured on their fixtures: 13 and 9 format-specific kinds, 0 diagnostics.
    Both collapse keys to `property` on the semantic layer and name how each key
    was written (`quoted-key`, `single-quoted-key`, `unquoted-key`) on syntax.
- Formats batch 2: `logfmt`, `ini` and `properties` join the format family,
  taking `FORMAT_IDS` / the `formats` feature from 12 ids to 15 and the
  playground picker from 61 to 64 engines.
  - `tokenizer-logfmt` (new crate) is a `key=value` record engine: `key`,
    `separator`, `bare-value`, `quoted-value`, `escaped-char`, `flag-key`,
    `record-break` on the lex layer plus `empty-value`, `integer-value`,
    `float-value`, `boolean-value` and `null-value` on the semantic layer, with
    `unterminated-value`, `missing-key` and `unexpected-token` recovery.
    Measured on its fixture: 8 format-specific kinds, 0 diagnostics.
  - `tokenizer-ini` (new crate) is one dialect-parameterised engine behind two
    hosts: INI gets sections, `;`/`#` comments, quoted spans and indented
    continuation lines; `.properties` gets `#`/`!` comments, `:`/whitespace
    separators, escape sequences and backslash joins. Its vocabulary is
    `section-marker`, `section-name`, `key`, `separator`, `value`, `quote`,
    `padding`, `escape-sequence`, `line-continuation`, `record-break`, retagged
    to `integer-value` / `decimal-value` / `boolean-value` when a value token
    covers its entry's whole value. Measured on the fixtures: 11 specific kinds
    for ini, 10 for properties, 0 diagnostics. `properties` is a cargo alias of
    `ini`, so two ids cost one new crate.
- Formats batch 3 closes the top-20 formats goal: `hcl`, `edn`, `srt`, `vtt` and
  `ics` join the family, taking `FORMAT_IDS` / the `formats` feature from 15 ids
  to the full 20 and the playground picker from 64 to 69 engines. `core::family`
  gains a count test so the preset cannot silently drift below 20 again.
  - `tokenizer-hcl` (new crate, 2.2k lines) is a config-as-code engine: nested
    blocks with labels, `attribute = value`, heredocs (`<<EOT` / `<<-EOT`),
    `${}` interpolation, ternaries, `#`/`//`/`/* */` comments. 18 lex kinds and
    21 semantic kinds — `block-type`, `block-label`, `attribute-name`,
    `heredoc-open`/`heredoc-body`/`heredoc-close`, `interpolation`, `escape` —
    with 9 kebab-case recovery codes. Measured on its fixture: 14
    format-specific kinds, 0 diagnostics.
  - `tokenizer-edn` (new crate, 3.5k lines) is a Clojure reader: collections
    `()` `[]` `{}` `#{}`, symbols and namespaced symbols/keywords, tagged
    values `#inst`/`#uuid`/`#my/tag`/`#B`, `#_` discard, radix integers,
    ratios, bigints, chars, special numbers. 33 lex kinds and 41 semantic kinds
    (31 outside the generic vocabulary) with 18 recovery codes. Measured on its
    fixture: 31 format-specific kinds, 0 diagnostics.
  - `tokenizer-srt` (new crate, 3.9k lines) is one timed-text engine behind two
    hosts: SubRip and WebVTT share the cue-block lexer and parser, and
    `Dialect::{Srt, Vtt}` select a 10-flag `Options` table (required `WEBVTT`
    signature, `NOTE`/`STYLE`/`REGION` blocks, optional hours, `,` vs `.`
    milliseconds, cue settings, inline markup, ordinal indices). 33 lex kinds;
    the semantic layer tells a cue's start timestamp from its end, a first text
    line from its continuations, and a timing-tag timestamp from a cue's own.
    Measured on the fixtures: 9 specific kinds for srt, 32 for vtt, 0
    diagnostics — so `vtt` is a cargo alias of `srt` and two ids cost one crate.
    15 recovery codes, five of them WebVTT-only.
  - `tokenizer-ics` (new crate, 3.7k lines) is an iCalendar engine: `BEGIN:` /
    `END:` component nesting, `NAME;PARAM=…:value` content lines with quoted
    parameter values, line folding, `\,` / `\;` / `\N` escapes, CRLF records,
    and typed values (`date`, `date-time`, `duration`, `period`, `recur`,
    `uri`, `text`). 15 lex kinds and 23 semantic kinds (20 outside the generic
    vocabulary) with 18 recovery codes, including `missing-vcalendar-wrapper`
    and per-component required-property checks. Measured on its fixture: 20
    format-specific kinds, 0 diagnostics.
- Playground palette now names a colour for every kind the batch-3 formats emit
  (+67 kinds, including the lex-only srt/vtt and ini vocabularies that only the
  syntax layer shows), gated by a new `catalog.test.js` check that tokenizes
  every picker sample on both layers and fails if any kind falls through to the
  hashed fallback colour. The only allowed fallbacks are url's `u-*` parts,
  which are matched by prefix rule on purpose. `catalog.test.js` also pins the
  measured specific-kind vocabulary of the five new formats, and
  `language-cases.js` adds 48 fixture cases across them (482 JS tests pass).
- `tests/wiring_completeness.rs`: the executable half of the structure review's
  §9.3 checklist. Thirteen checks make a half-wired engine fail the merge gate —
  the ten described in this bullet, plus `only_format_engines_advertise_validate`
  under *Changed* and the valid/broken pair in the next two bullets:
  every plugin crate reachable through a named feature, every `dep:` target
  present, every optional dependency gated, `formats`/`top20`/`wave7` equal to
  the core tables, `all-languages` transitively covering every preset, the
  generic-kind baseline shared with `playground/src/catalog.js`, the playground
  picker and fixture matrix equal to the registry, and every format engine
  parsing its own `tests/fixtures/formats/<id>.txt` with zero diagnostics while
  emitting at least one non-generic kind. Two more sweep those fixtures by
  truncation: `no_format_engine_panics_or_stalls_on_a_truncated_document` cuts
  each fixture at every character boundary and requires the token stream to tile
  that prefix exactly inside a warm-up-excluded time budget, so "recovers instead
  of hanging" is a measured claim rather than prose (measured: ICS is linear —
  695 KB in 80 ms — and a warm prefix costs 0.1 ms, while a cold first touch on
  a loaded machine measured 61 ms, so the budget is 1 s per prefix plus 30 s per
  format sweep rather than a tight per-call wall clock);
  `every_format_diagnostic_code_is_kebab_case` walks the same corpus and checks
  that every emitted code is kebab-case with a non-empty message and an
  in-bounds span, then asserts the sweep as a whole reaches 40 distinct codes so
  the recovery vocabulary cannot pass by silently reporting nothing.
- `tests/fixtures/languages/<id>.txt`: a valid document for each of the 49 wave
  languages, exported from the playground's own picker samples by the new
  `playground/scripts/export-language-fixtures.mjs` (`npm run
  export:language-fixtures`). Before this corpus existed the shared fullkit
  pipeline was untested in both directions on languages: the fixture matrix
  marks 73 `expectValid` and 30 `expectValid: false` cases, and all 103 of them
  are formats. `catalog.test.js` adds a matching gate that requires each file to
  still equal its picker sample byte-for-byte and runs all 69 samples through
  their engines at both layers, expecting no diagnostics on any of them (487 JS
  tests pass).
- Two gates over that corpus, one per direction.
  `every_engine_is_quiet_on_its_valid_fixture` runs all 69 registered engines on
  their valid document at both layers and requires zero diagnostics;
  `every_language_engine_flags_unbalanced_delimiters` covers the other half,
  asserting 11 broken shapes across go, python, lisp, scheme, sql, java,
  powershell, assembly and zig come back flagged with the expected code and an
  in-bounds span, and that 6 interpolation shapes (`${…}`, `#{…}`, `{$…}`,
  `$item->{…}` and a brace inside a template literal) stay quiet — an
  `unclosed-delimiter` that fired inside a string would be worse than none.
- `core::markup_full::MarkupFlavor`, so HTML is parsed as HTML. `parse_markup`
  keeps strict XML rules and `parse_markup_as(source, MarkupFlavor::Html5)` adds
  the four dialect rules the XML reading cannot express: 14 void elements end at
  their `>`, four raw-text elements (`script`, `style`, `textarea`, `title`) run
  to their own close tag whatever the body holds, 14 elements may omit their end
  tag (`<li>a<li>b`, `<td>`, `<p>`), and tag names fold ASCII case. `tokenizer-html`
  switches to the new flavor; `tokenizer-xml` keeps the strict one and now has
  matrix cases proving it rejects what HTML allows (`<br>`, `<A></a>`).
- `langkit::highlight_markup`'s `htmlish` parameter does something. It was
  accepted and discarded (`let _ = htmlish;`), so an HTML `<script>` body was
  lexed as markup at the first `<` inside it and the playground showed
  `< 2) console.log("ok");</script>` as one `tag` token — while the parser, and
  therefore the verdict, said the document was fine. Raw-text bodies are now one
  `text` token in both readers, from a shared `langkit::HTML_RAW_TEXT` +
  `langkit::find_close_tag`, and `lex_markup` takes the flavor so XML keeps
  reading the same bytes as markup. **Breaking for `tokenizer-core` direct
  users:** `lex_markup(source)` became `lex_markup(source, MarkupFlavor)`; no
  crate in the workspace called it.
- `stray-close-tag`: a close tag with no element open, reachable at document
  level where nothing else can report it.
- Six `expectValid` and four invalid html cases plus two xml cases in
  `playground/tests/language-cases.js`, seven `markup_full` unit tests pinning
  the dialect in the tree, the token stream and the gap between the two flavors. The playground's HTML sample and
  `tests/fixtures/formats/html.txt` are now a real document (head with void
  `meta`/`link`, a `<style>` rule block, omitted `</p>`/`</li>`, a `<script>`
  body containing `<`), so the format fixture gate and the per-prefix truncation
  sweep exercise those paths. JS matrix 499 tests.
- Full multi-language pipeline in `core::fullkit` (lex → recovering parse →
  shared AST → semantic tokens) for programming/data languages.
- Full markup AST pipeline in `core::markup_full` for HTML/XML.
- 55 language crates + feature groups `wave1`…`wave7`, `top20`, `next20`,
  `all-languages`.
- TIOBE-style top 20 as first-class `top20` feature (full engines).
- Next 20 popular languages as `next20` / `wave7`: groovy, haskell, elixir,
  erlang, clojure, fsharp, ocaml, lisp, scheme, solidity, zig, nim, dlang,
  cobol, ada, prolog, abap, vhdl, verilog, graphql.
- Docs: `docs/languages.md`.
- Docs: `docs/structure-review.md` — measured audit of the workspace against
  this project's own contract (SOLID, DRY, modularity, pluggability), with the
  phased cleanup plan it implies.
- Playground WASM case matrix covering every registered language id.
- Playground UI/UX for all language types: searchable grouped language
  combobox, shared `?lang=&mode=&layer=` state, per-language example case
  picker from the fixture matrix, token-kind legend with hover/pin
  filtering, inline colors for every engine kind (hashed fallback), line
  numbers, cursor `L:col` readout, and token-map render cap.

### Changed

- `VALIDATE` is now earned by a grammar instead of inherited from a shared
  descriptor. `fullkit::FULL_ENGINE_CAPS` drops it, so the 49 wave languages
  advertise `LEX|PARSE|SEMANTIC` and the badge belongs to exactly the 20
  format engines: measured on the playground's own valid samples, the shared
  recovering parser raised `unexpected-token` / `expected-token` on 25 of them
  (valid go `s := "hi"` among them) while reporting nothing at all for broken
  `function f( {` and `SELECT * FROM`. The capability partition is now
  49 `lex|parse|semantic`, 18 `lex|parse|semantic|validate`, `json`
  (`+cst|navigate|visitor`) and `url` (`lex|validate`).
  - `tokenizer-toml`, `tokenizer-yaml`, `tokenizer-html` and `tokenizer-xml`
    keep `VALIDATE` by declaring their own capability set rather than borrowing
    `full_descriptor` / `FULL_CAPS`: each rejects a real spec violation
    (`[s`, `a: [1,`, a mismatched or unclosed element) and stays silent on the
    valid document, which is what the badge claims.
  - Two gates pin it: `only_format_engines_advertise_validate` in
    `tests/wiring_completeness.rs` (check 11, registry vs `FORMAT_IDS`) and
    `only the format family advertises validation` plus
    `an engine that claims validation is quiet on valid input` in
    `playground/tests/catalog.test.js`, which replays all 73 `expectValid`
    cases of the badge-holding engines and fails if any comes back flagged.
  - The playground stops rendering a verdict it did not earn: the status pill
    reads `VALID` / `N DIAGS` only for a validating engine, and `TOKENIZED` /
    `N FLAGS · NOT VALIDATED` otherwise; the same split applies to the batch
    matrix row styling, the compare-view verdict and the diagnostics panel copy.
  - The new badge gate immediately found a wrong fixture rather than a wrong
    engine: `json/jsonc-comment` carried a trailing comma, which JSONC accepts,
    so it is now two cases — `jsonc-allows-trailing-comma` (valid) and
    `trailing-comma-rejected-in-strict` (invalid) — and the JS matrix is 486
    tests.
- `core::fullkit` is recovery-first: the parser recovers in silence, and only
  the token stream is allowed to report a problem. Dropping `VALIDATE` from the
  badge made the noise visible but did not remove it, so the two parser
  diagnostic sites are gone (`expect_text` no longer returns a diagnostic, and an
  unrecognized construct in `parse_prefix` bumps and yields `Expr::Error`),
  `Parser` lost its `diagnostics` field, and every diagnostic in the shared
  pipeline now comes from the lex + delimiter pass. Measured on the 69 valid
  documents: 76 diagnostics across 25 of them before, 0 after, with 11/11 broken
  shapes still flagged.
  - In their place, `check_delimiters` proves bracket balance from the token
    stream alone — language-independent, so it needs no per-profile grammar:
    `unclosed-delimiter` points at the opener that never closed, and a closer
    with no matching opener raises `stray-delimiter` and drops that opener, so
    one typo does not cascade into a diagnostic per remaining token. Delimiters
    inside literals are not counted, which is what makes the claim safe on
    `"total: $item->{n}"`. `FULL_ENGINE_CAPS` now documents what the pipeline can
    actually prove: `s := "hi"` is accepted and `func f( {` is rejected, but so
    is `SELECT * FROM`.

### Fixed

- `tokenizer-html` stops flagging valid HTML. It advertises `VALIDATE`, and under
  the XML rules of `parse_markup` that claim was false on the ordinary shapes: of
  20 hand-written documents, **12 came back with `mismatched-tag` /
  `unclosed-element`** — a `<meta>` in `<head>` produced three diagnostics,
  `<ul><li>a<li>b</ul>` produced three, `<script>if (a < b)` produced two, and
  `<DIV>x</div>` one — while a document the spec ends early,
  `<script>var s = "</script>";</script>`, was accepted. After
  `MarkupFlavor::Html5`: 0 of 19 valid shapes flagged (the `"</script>"` case
  moved to the invalid list, where it is now reported as `stray-close-tag`), with
  the broken direction intact — `<div>a`, `<div`, `<script>var a = 1;` and `</p>`
  still fail, and `<div>a</span></div>` now yields exactly one
  `mismatched-tag` because an unmatched end tag is ignored per spec instead of
  closing the element and turning the author's real `</div>` into a second
  mistake. XML gets stricter by the same change: `<a></b>` reports both the
  mismatch and the element that stayed open.
- `fullkit` no longer mis-lexes a template literal. The valid-document gate
  caught it from the other side: for javascript and typescript, backticks were
  punctuation, so a `const t = \`x${y}z\`;` sample highlighted as identifiers
  and — worse, once the delimiter pass existed — a brace inside an unfinished
  template string was reported as `unclosed-delimiter`. A backtick now opens one
  `StringLit` span (escapes honored, an unmatched one left as ordinary
  punctuation rather than swallowing the rest of the file) scoped to the five
  profiles where `$` names a variable: groovy, javascript, php, perl and
  typescript. Languages that also write backticks without that flag are
  untouched by design — haskell uses them for an infix call, sql for a quoted
  identifier, and bash command substitution stays punctuation, which is a
  highlighting gap the flag does not cover rather than a false error. All 49
  language fixtures contain zero backticks, so the change is corpus-neutral.
- `tokenizer-url` no longer emits a zero-length token for `http://host:`, where
  a `host:` announces a port slot and leaves it empty. `verify_lossless_spans`
  forbids empty spans workspace-wide, and the new truncation sweep reached the
  same input from the other side: it failed on url's fixture at 13 bytes. The
  empty slot is now carried as `UrlTokenization::empty_port`, an
  `Option<Span>` fact about the authority, while the colon stays the `u-sep`
  separator token it already was. The `url-empty-port` diagnostic fires from
  exactly that span, so hosts see the same error at the same offsets with a
  token stream that now tiles the source. `tokenizer.test.js`'s span check went
  from `end >= start` to `end > start` to match, and passes for all 69 engines on
  both layers; `language-cases.js` adds an `empty-port-flagged-not-typed` url
  case so the shape stays covered in the playground matrix (482 JS tests pass).

## [0.4.0] - 2026-08-08

### Added

- Workspace crate `themoretheless-tokenizer-core` with shared `Span`, source
  positions, diagnostics, language/dialect ids, capability flags, input limits,
  lossless helpers, object-safe `HostLanguage`, and a static language registry.
- Workspace crates `themoretheless-tokenizer-json` and
  `themoretheless-tokenizer-url` as the first language plugins.
- Facade helpers `register_builtins`, `builtin_registry`, and `analyze_host`.
- Multi-language playground bridge `tokenization(language, source, mode, layer)`
  plus WASM `tokenize`; JSON-only entry remains for compatibility.
- Convenience API `api::Source` / `Analysis` (`syntax`, `highlight`, `errors`)
  plus `api::quick` helpers and `api::prelude`.
- Concepts glossary: `docs/concepts-and-api.md`.
- Design contract for multi-language Cargo plugins in `docs/plugin-api-design.md`.

### Changed

- Facade re-exports core primitives and language crates behind features
  `json` (default) and `url` (default).
- Package layout is a multi-crate workspace; consumers can still depend on the
  facade crate alone.

## [0.3.1] - 2026-08-03

### Added

- JSON fixture conformance coverage, deterministic property and differential
  tests, and reproducible benchmarks.
- CI checks on stable Rust.
- Dual licensing under MIT or Apache-2.0.
- Crates.io publication checks and a checksum-verified release workflow.
- Offset-based AST navigation with stable paths through duplicate object keys.
- A deterministic, object-safe AST visitor with subtree skipping and early exit.
- A lossless, AST-backed concrete syntax tree plus validated batch text edits.
- A Vue and WebAssembly playground for visually inspecting JSON tokenization.
- GitHub Pages deployment through the shared organization workflow templates.

### Changed

- Package metadata and installation documentation now describe the complete
  JSON/JSONC syntax engine.

## [0.2.0] - 2026-08-01

### Added

- A strict, lossless JSON lexer with exact token spans and bounded diagnostics.
- A recovering parser with a borrowing, order-preserving AST, duplicate-key
  preservation, exact number spellings, and checked numeric conversions.
- JSONC options for comments, an initial BOM, and trailing commas.
- Parser-aware semantic tokens and UTF-8, Unicode-scalar, and UTF-16 source
  position conversion.
- Configurable input, token, nesting-depth, and diagnostic limits.

### Changed

- Expanded the crate from a highlighting tokenizer into a JSON/JSONC syntax
  engine while retaining `JsonTokenizer`, `Tokenizer`, and `tokenize_json` for
  compatibility.

## [0.1.0] - 2026-08-01

### Added

- Initial dependency-free JSON tokenizer with UTF-8 byte spans, diagnostics,
  semantic token categories, and the `JsonTokenizer` compatibility facade.

[Unreleased]: https://github.com/themoretheless/tokenizer/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/themoretheless/tokenizer/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/themoretheless/tokenizer/compare/v0.2.0...v0.3.1
[0.2.0]: https://github.com/themoretheless/tokenizer/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/themoretheless/tokenizer/releases/tag/v0.1.0
