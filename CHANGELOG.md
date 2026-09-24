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
- Playground WASM case matrix covering every registered language id.
- Playground UI/UX for all language types: searchable grouped language
  combobox, shared `?lang=&mode=&layer=` state, per-language example case
  picker from the fixture matrix, token-kind legend with hover/pin
  filtering, inline colors for every engine kind (hashed fallback), line
  numbers, cursor `L:col` readout, and token-map render cap.

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
