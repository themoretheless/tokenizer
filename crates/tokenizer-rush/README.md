# themoretheless-tokenizer-rush

Full rush engine (lex → parse → AST → semantic) for
[themoretheless-tokenizer](https://github.com/themoretheless/tokenizer).

rush is the single language of [ruos](https://github.com/themoretheless) and
open-scad-viewer: Python-style indentation blocks, `fn` headers, fluent
method chains, `foreach ... yield`, `match`, hash comments, `$variables`,
and shell pipe sugar (`|`). One grammar serves both the system shell and
the CAD workbench; domain libraries are the only difference.

```rust
let parsed = themoretheless_tokenizer_rush::parse(
    "if $status = 0:\n    echo ok\n"
);
assert!(parsed.diagnostics.is_empty());
```

File extensions: `.r` (ruos shell scripts), `.mg` (ModelGraph Text in the
CAD workbench — same core grammar).
