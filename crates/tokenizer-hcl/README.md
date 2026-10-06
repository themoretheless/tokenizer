# themoretheless-tokenizer-hcl

Full HCL engine for [themoretheless-tokenizer](../../README.md): a lossless
lexer, a recovering structure pass, a role-aware semantic layer, and stable
diagnostic codes. Registered through `DESCRIPTOR` (`LanguageId::HCL`) with the
host adapter `Host` (`ENGINE`).

## What it parses

HCL bodies of `name = expression` attributes and `type "label" { ... }`
blocks: `#`/`//` line comments, `/* */` block comments, template strings with
`${...}` interpolations and `\n` / `\xNN` / `\uXXXX` escapes, heredocs
(`<<TAG`, `<<-TAG`, with interpolations split out of the body), numbers
(int, float, exponent), `true` / `false` / `null`, lists, nested objects,
operators `= == != ! && || + - * / % ? :`, and newline-delimited items — the
lexer keeps newlines as their own tokens because HCL delimits by line.

The semantic layer re-tags, without moving a span: identifiers before `=` on
the same line become `attribute-name`, a same-line `type "label" {` header
yields `block-type` and `block-label`, and the lexer already carries
`heredoc-open` / `heredoc-body` / `heredoc-close`, `interpolation`, `escape`,
`boolean`, `null`, `newline`, `line-comment`, `block-comment`, `operator`.

## Guarantees

- **Lossless**: half-open UTF-8 byte spans, never zero-length; concatenating
  token text rebuilds the input byte-for-byte in both layers, malformed input
  included.
- **Total**: no panics, no hangs — every lexer advance guarantees progress and
  truncations, stray bytes and unclosed constructs only change flags.
- **Stable diagnostics**: `unterminated-string`, `unterminated-heredoc`,
  `unterminated-interpolation`, `unterminated-comment`, `invalid-escape`,
  `unclosed-brace`, `unclosed-bracket`, `unclosed-paren`, `unexpected-token`.

## Simplifications (honest scope)

Interpolations are one opaque `interpolation` token — the `${...}` bytes are
balanced but not sub-lexed. There is no expression tree, so function calls,
`for` expressions and full ternary structure are recognized only at token
level (brackets balance, operators classify). `%{ ... }` template directives
are not part of the grammar.

## Example

```rust
use themoretheless_tokenizer_hcl::{parse, validate};

let parsed = parse("service \"api\" {\n  port = 8080\n}\n");
assert!(parsed.is_valid());
assert!(parsed.is_block_type(parsed.lexed().tokens()[0].span));

let codes: Vec<&str> = validate("a = \"oops").iter().map(|d| d.code).collect();
assert_eq!(codes, ["unterminated-string"]);
```
