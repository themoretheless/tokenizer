# Language plugins

## Host API (any enabled language)

```rust
use themoretheless_tokenizer::Source;

Source::new("python", code).highlight()?;
Source::new("fortran", code).lex()?;
Source::new("html", markup).errors()?;
```

Typed full pipeline (most languages):

```rust
use themoretheless_tokenizer::python;
python::lex(src);
python::parse(src);    // Module / Item / Stmt / Expr AST
python::tokenize(src); // semantic
```

HTML/XML:

```rust
use themoretheless_tokenizer::html;
html::parse("<div/>"); // MarkupDoc AST
```

## Feature groups

| Feature | Contents |
|---------|----------|
| `default` | `json`, `url` |
| `top20` | TIOBE-style 20 programming languages (full) |
| `next20` | next 20 popular languages (wave7) |
| `wave1`…`wave7` | delivery batches |
| `all-languages` | everything (json+url+55 plugins) |

```toml
themoretheless-tokenizer = { version = "0.4", features = ["top20"] }
# or
features = ["next20"]
# or
features = ["all-languages"]
```

## Top 20 (feature `top20`)

All **fullkit** (or native) editor engines:

| # | id | Notes |
|---|-----|--------|
| 1 | python | fullkit |
| 2 | c | fullkit |
| 3 | cpp | fullkit |
| 4 | java | fullkit |
| 5 | csharp | fullkit |
| 6 | javascript | fullkit |
| 7 | visualbasic | fullkit |
| 8 | sql | fullkit |
| 9 | go | fullkit |
| 10 | fortran | fullkit |
| 11 | matlab | fullkit |
| 12 | php | fullkit |
| 13 | rust | fullkit |
| 14 | r | fullkit |
| 15 | ruby | fullkit |
| 16 | kotlin | fullkit |
| 17 | swift | fullkit |
| 18 | typescript | fullkit |
| 19 | delphi | fullkit |
| 20 | assembly | fullkit |

## Next 20 (feature `next20` / `wave7`)

| # | id | Notes |
|---|-----|--------|
| 1 | groovy | fullkit |
| 2 | haskell | fullkit |
| 3 | elixir | fullkit |
| 4 | erlang | fullkit |
| 5 | clojure | fullkit |
| 6 | fsharp | fullkit |
| 7 | ocaml | fullkit |
| 8 | lisp | fullkit (Common Lisp) |
| 9 | scheme | fullkit |
| 10 | solidity | fullkit |
| 11 | zig | fullkit |
| 12 | nim | fullkit |
| 13 | dlang | fullkit (D) |
| 14 | cobol | fullkit |
| 15 | ada | fullkit |
| 16 | prolog | fullkit |
| 17 | abap | fullkit |
| 18 | vhdl | fullkit |
| 19 | verilog | fullkit |
| 20 | graphql | fullkit |

Plus markup/data: json (deepest), url, html, xml, css, yaml, toml, markdown, mongo, bash, powershell, dart, scala, lua, perl, objectivec, julia, …

## Capability honesty

| Engine | Level |
|--------|--------|
| json | Full native: lex/parse/AST/CST/nav/visitor |
| html/xml | Full markup AST |
| fullkit langs | LEX\|PARSE\|SEMANTIC\|VALIDATE via shared AST |
| url | LEX\|VALIDATE (URL structure) |

Editor-grade recovering front-ends, not language-spec compilers.
