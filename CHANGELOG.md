# Changelog

All notable changes to this project are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-08-03

### Added

- JSONTestSuite conformance coverage, deterministic property and differential
  tests, scheduled fuzzing, and reproducible benchmarks.
- CI checks on stable Rust and the declared Rust 1.85 minimum version.
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

[Unreleased]: https://github.com/themoretheless/tokenizer/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/themoretheless/tokenizer/compare/v0.2.0...v0.3.1
[0.2.0]: https://github.com/themoretheless/tokenizer/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/themoretheless/tokenizer/releases/tag/v0.1.0
