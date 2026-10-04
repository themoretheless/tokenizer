# themoretheless-tokenizer-rush

Full rush engine (lex → parse → AST → semantic) for
[themoretheless-tokenizer](https://github.com/themoretheless/tokenizer).

rush is the scripting language of the ruos shell: Python-style indentation
blocks, `fn` headers, fluent method chains, `foreach ... yield`, `match`,
hash comments, `$variables`, and shell pipe sugar
(`ls | where size > 0 | select name`).

```rust
let parsed = themoretheless_tokenizer_rush::parse(
    "if $status = 0:\n    echo ok\n"
);
assert!(parsed.diagnostics.is_empty());
```

File extension: `.r`.
