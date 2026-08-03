# themoretheless-tokenizer

[![CI](https://github.com/themoretheless/tokenizer/actions/workflows/ci.yml/badge.svg)](https://github.com/themoretheless/tokenizer/actions/workflows/ci.yml)

Dependency-free JSON engine for Rust: a strict lossless lexer, recovering
parser, parser-aware tokenizer, JSONC options, borrowing AST and source-position
utilities. The published library has no runtime dependencies and does not
integrate with `serde`, JavaScript, WebAssembly or a UI framework, so it works
in native, server-side and WASM code. `serde_json` is used only by development
tests and benchmarks as an independent reference implementation.

All public spans are half-open UTF-8 byte ranges. They can be used directly to
slice the original Rust string.

## Installation

From crates.io:

```toml
[dependencies]
themoretheless-tokenizer = "0.3"
```

The latest unreleased revision can instead be used directly from Git:

```toml
[dependencies]
themoretheless-tokenizer = { git = "https://github.com/themoretheless/tokenizer.git" }
```

The Cargo package uses hyphens, while the Rust crate name uses underscores:
`themoretheless_tokenizer`.

## Strict lossless lexer

`json::lex` applies the lexical rules of strict JSON. It distinguishes every
punctuation mark and literal, validates JSON strings and the complete number
grammar, and reports comments and an initial byte-order mark (BOM) as errors.
Whitespace and invalid fragments remain in the token stream. Comment and BOM
tokens keep their highlighting category even when diagnostics reject them;
malformed strings and numbers carry `TokenFlags::HAS_ERROR`.

```rust
use themoretheless_tokenizer::json::{SyntaxKind, lex};

let source = "{\n  \"name\": \"Москва\", \"value\": -1.25e+3\n}";
let lexed = lex(source);

assert!(!lexed.has_errors());
assert!(lexed
    .significant_tokens()
    .any(|token| token.kind == SyntaxKind::Number));

let rebuilt: String = lexed
    .tokens()
    .iter()
    .copied()
    .map(|token| token.text(source).unwrap())
    .collect();
assert_eq!(rebuilt, source);
```

The lossless invariant holds for every UTF-8 input, including malformed and
incomplete editor text:

```text
concat(token.text(source) for token in lex(source).tokens()) == source
```

Tokens are non-empty, ordered, contiguous and end on UTF-8 character
boundaries. `Lexed` borrows its source; token text is never copied. Use
`significant_tokens()` to skip whitespace, comments and BOM without discarding
them from the full stream.

### JSONC mode

`LexerOptions::jsonc()` allows `//` and `/* ... */` comments plus a BOM at the
start of the document. Individual allowances can also be enabled with the
builder methods.

```rust
use themoretheless_tokenizer::json::{LexerOptions, SyntaxKind, lex_with};

let source = "\u{feff}{ /* generated */ \"enabled\": true }";
let lexed = lex_with(source, LexerOptions::jsonc());

assert!(lexed.diagnostics().is_empty());
assert!(lexed
    .tokens()
    .iter()
    .any(|token| token.kind == SyntaxKind::BlockComment));

let comments_only = LexerOptions::strict()
    .allow_comments(true)
    .allow_bom(false);
assert!(comments_only.comments_allowed());
assert!(!comments_only.bom_allowed());
```

JSONC mode changes only comment and BOM handling. Strings, keywords and numbers
still use strict JSON syntax; it does not add single-quoted strings, unquoted
keys, hexadecimal numbers, `NaN` or `Infinity`.

### Diagnostics and recovery

Lexing returns `Lexed`, not `Result`. Problems are collected in
`diagnostics()` and scanning continues, which is useful for editors and batch
reporting. Each `LexDiagnostic` has an exact byte `span`; its
`LexDiagnosticKind` provides a stable kebab-case `code()` and implements
`Display`.

```rust
use themoretheless_tokenizer::json::{LexDiagnosticKind, SyntaxKind, lex};

let lexed = lex("[01, \"bad\\q\", true]");

assert!(lexed.has_errors());
assert!(lexed.diagnostics().iter().any(|diagnostic| {
    diagnostic.kind.code() == "number-leading-zero"
}));
assert!(lexed
    .significant_tokens()
    .any(|token| token.kind == SyntaxKind::True));
assert!(lexed.diagnostics().iter().any(|diagnostic| {
    diagnostic.kind == LexDiagnosticKind::InvalidEscape
}));
```

Recovery is lexical: the lexer always makes progress and preserves every input
byte. Use the parser to validate object/array structure, separators and the
single-root-value rule.

## Recovering parser

`json::parse` performs strict structural validation and builds a borrowing,
order-preserving AST. It reports errors without discarding the lossless lexer
output, so incomplete editor input remains inspectable.

```rust
use themoretheless_tokenizer::json::{Value, parse};

let parsed = parse(r#"{"city":"Москва","emoji":"\uD83D\uDE00","n":-1.2e3}"#);
assert!(parsed.is_valid());

let object = parsed.value().and_then(Value::as_object).unwrap();
assert_eq!(object.get("city").and_then(Value::as_str), Some("Москва"));
assert_eq!(object.get("emoji").and_then(Value::as_str), Some("😀"));
assert_eq!(
    object.get("n").and_then(Value::as_number).unwrap().as_f64(),
    Ok(-1200.0)
);
```

`ParseOptions::jsonc()` enables comments, BOM and trailing commas. Builders can
toggle each extension and bound input bytes, nesting, token count and
diagnostics. Nesting is always clamped to `MAX_SUPPORTED_DEPTH` to keep
recursive parsing and AST destruction stack-safe.

```rust
use themoretheless_tokenizer::json::{ParseOptions, parse_with};

let source = "{/* generated */ \"enabled\": true,}";
let parsed = parse_with(source, ParseOptions::jsonc());
assert!(parsed.is_valid());
```

## Borrowing AST and exact numbers

`json` exposes order-preserving `Value`, `Object`, `Member`, `Array`,
`StringValue`, `Number`, `Boolean` and `Null` types. Values carry source spans;
objects keep member order and duplicate keys (`get` returns the first match and
`get_all` returns every match). Raw strings and numbers borrow from the input.
A decoded string uses `Cow`, so an unescaped string can stay borrowed while an
escaped string can own only its decoded representation.

Numbers are stored as their exact source spelling rather than being eagerly
converted through `f64`:

```rust
use themoretheless_tokenizer::json::{Value, parse};

let parsed = parse("9007199254740993");
let number = parsed.value().and_then(Value::as_number).unwrap();

assert_eq!(number.as_str(), "9007199254740993");
assert_eq!(number.as_i64().unwrap(), 9_007_199_254_740_993);
```

Conversions are explicit and checked: `as_i64`, `as_u64` and `as_f64` return a
`Result`. They reject malformed recovery tokens, integer spelling/type errors,
overflow, non-finite results and floating-point underflow to zero. A finite
`f64` conversion can still round according to IEEE 754; use
`as_str()` with a decimal/big-integer library when decimal precision must be
preserved. The crate itself does not provide arbitrary-precision arithmetic.

Malformed strings remain represented for recovery, but their decoded value is
`None`; lexical/parser diagnostics explain why.

## Parser-aware tokenizer

`json::tokenize` produces lossless semantic tokens and parser diagnostics.
Unlike the legacy compatibility API, it labels a string as `Property` only when
the parser proves it is an object member key.

```rust
use themoretheless_tokenizer::json::{SemanticKind, tokenize};

let source = r#"{"name":"Denis"}"#;
let result = tokenize(source);
assert!(result.diagnostics.is_empty());
assert!(result.tokens.iter().any(|token| {
    token.kind == SemanticKind::Property && token.text(source) == Some("\"name\"")
}));
```

## Source positions

`Span` and lexer diagnostics use UTF-8 byte offsets. `LineIndex` converts them
to zero-based line/column pairs in any of three column encodings:

- `Utf8Bytes` — the same unit as `Span`;
- `UnicodeScalars` — Rust `char` values, not grapheme clusters;
- `Utf16CodeUnits` — suitable for LSP and browser integrations.

```rust
use themoretheless_tokenizer::{ColumnEncoding, LineColumn, LineIndex};

let source = "a😀\r\nМосква";
let index = LineIndex::new(source);
let offset = "a😀".len();

let position = index
    .line_column(offset, ColumnEncoding::Utf16CodeUnits)
    .unwrap();
assert_eq!(position, LineColumn::new(0, 3));
assert_eq!(
    index
        .offset(position, ColumnEncoding::Utf16CodeUnits)
        .unwrap(),
    offset
);
```

`LineIndex` borrows the exact source it indexes, making repeated conversions
independent of total document length. `LF`, `CRLF` and lone `CR` each count as
one line break. An offset between the two bytes of `CRLF`, a UTF-8 continuation
byte or an invalid UTF-16 column returns `PositionError`.

## Legacy semantic tokenizer

The original highlighting-oriented API remains available. It groups all
punctuation together and classifies a string followed by `:` as `Property`.
Existing users can keep using `JsonTokenizer`, `Tokenizer` and
`tokenize_json`:

```rust
use themoretheless_tokenizer::{JsonTokenizer, TokenKind, Tokenizer};

let source = r#"{"name":"Denis","active":true}"#;
let result = JsonTokenizer.tokenize(source);

assert!(result.diagnostics.is_empty());
assert!(result.tokens.iter().any(|token| {
    token.kind == TokenKind::Property && token.text(source) == Some("\"name\"")
}));
```

For new syntax tooling, prefer `json::lex`, `json::parse` or `json::tokenize`.
The legacy API is retained for compatibility.

## Local tokenization playground

An isolated Vue test UI in `playground/` visualizes output from the actual Rust
lexer and parser-aware tokenizer. It switches between strict JSON and JSONC,
exact syntax and semantic tokens, and shows UTF-8 byte spans plus recovery
diagnostics.

```sh
cd playground
npm install
npm run dev
```

Open `http://localhost:4173`. Vite invokes the `tokenizer-web-bridge` binary for
each local analysis request. Vue dependencies stay in `playground/`, and the
Rust bridge uses only the standard library and the crate's public API.

The same playground is published at
[`themoretheless.github.io/tokenizer`](https://themoretheless.github.io/tokenizer/).
GitHub Pages builds the adapter in `wasm/`, so production runs the actual Rust
tokenizer as WebAssembly entirely in the browser. The deployment workflow is
`.github/workflows/pages.yml` and runs after pushes to `main`.

## Current limits

- Input is a complete UTF-8 `&str`; there is no streaming/chunked lexer API.
- The new lexer/parser APIs default to a 16 MiB input limit plus bounded token
  and diagnostic vectors; token text itself is borrowed. The legacy
  `tokenize_json` facade remains unbounded for behavior compatibility.
- The parser validates JSON syntax, not application-specific schemas.
- JSONC support is limited to comments, BOM handling and trailing commas; it is
  not JSON5.
- The crate does not serialize JSON and does not integrate with `serde`.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or
the [MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project shall be dual licensed as above, without any
additional terms or conditions.
