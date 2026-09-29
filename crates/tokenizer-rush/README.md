# themoretheless-tokenizer-rush

Rush editor engine: lossless lexing, recovering parse, shared AST, semantic
highlighting and diagnostics, using the shared fullkit profile.

Rush uses `//` line comments, `/* ... */` block comments, `return`, and `.r`
files. The profile supports brace and indentation blocks, `$variables`,
fluent chains, `foreach ... yield`, `match`, and pipes.

```rust
let source = "fn main() { return 1; } // demo\n";
let parsed = themoretheless_tokenizer_rush::parse(source);
assert!(parsed.lexed.is_lossless(source));
```

The R engine also claims `.r`; hosts should select Rush explicitly by its
`rush` language id when the extension is ambiguous.
