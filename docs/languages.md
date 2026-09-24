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
| `top20` | 20 popular programming languages (`core::family::TOP20_IDS`) |
| `next20` | next 20 popular languages, wave7 (`core::family::NEXT20_IDS`) |
| `formats` | the 8 data/config/markup formats (`core::family::FORMAT_IDS`) |
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

`LanguageDescriptor.capabilities` is a coarse bitset and is close to
informational: of the 57 registered engines, **55 advertise the identical**
`LEX|PARSE|SEMANTIC|VALIDATE`. Only `json` (adds `CST|NAVIGATE|VISITOR`) and
`url` (`LEX|VALIDATE`) differ. So a capability column tells you the API
surface an engine implements, **not** how well it understands its format.

The real bar is **measured vocabulary depth**: how many token kinds an engine
emits that are specific to *its* format, i.e. outside the shared generic
vocabulary (`class, comment, function, identifier, keyword, number,
punctuation, string, type, variable, whitespace`). The playground matrix
computes this live by tokenizing each engine's sample and subtracting that
generic set; `playground/tests/catalog.test.js` pins the result as a tripwire.

For the 8 formats, measured on their playground samples:

| id | advertised caps | generic-kind-only? | format-specific kinds | engine |
|----|-----------------|--------------------|-----------------------|--------|
| json | lex\|parse\|semantic\|validate\|cst\|navigate\|visitor | no | `boolean`, `property` | hand-written recovering parser (deepest) |
| url | lex\|validate | no | `u-scheme`, `u-host`, `u-path`, `u-port`, `u-user`, `u-key`, `u-val`, `u-sep`, `u-frag` | hand-written URL structure |
| toml | lex\|parse\|semantic\|validate | no | `bare-key`, `basic-string`, `equals`, `newline`, `true` (+ full spec vocab) | hand-written recovering parser |
| yaml | lex\|parse\|semantic\|validate | no | `value-indicator`, `plain-scalar`, `line-break` (+ node vocab) | hand-written recovering parser |
| html | lex\|parse\|semantic\|validate | no | `tag`, `text` | markup AST (`core::markup_full`) |
| xml | lex\|parse\|semantic\|validate | no | `tag`, `text` | markup AST (`core::markup_full`) |
| markdown | lex\|parse\|semantic\|validate | no | `heading-marker`, `heading-text`, `code-span`, `code-fence-marker`, `fence-info`, `code-block-line`, `table-*`, `list-marker`, `blockquote-marker`, `footnote-*`, `link-*` (34-kind vocabulary) | hand-written recovering parser |
| css | lex\|parse\|semantic\|validate | no | `type-selector`, `class-selector`, `id-selector`, `pseudo-class`, `property`, `value`, `unit`, `color`, `at-rule`, `important`, `variable` (34-kind vocabulary) | hand-written recovering parser |

`markdown` and `css` were previously exactly that trap: a C-like
programming-language lexer pointed at prose and stylesheets, where `# Heading`
lexed as `comment`, `body { color: red }` could not tell a property from a
selector, and *valid* documents still emitted 9 and 3 spurious "unexpected
token" diagnostics while showing **zero** format-specific kinds. Both are now
hand-written recovering engines with their own `SyntaxKind` vocabularies; on
the same samples they report 8 and 7 format-specific kinds and **zero**
diagnostics, and `catalog.test.js` pins those kind sets so a regression back to
generic lexing fails the suite. Markdown has no comment syntax, so the
`comment` kind is intentionally absent from its vocabulary; CSS disambiguates
`#main` (id selector) from `#fff` (color) by brace-depth context.

## Top 20 formats (family axis)

`Family` splits engines by *what they tokenize*, orthogonal to the delivery
waves and the `top20`/`next20` language sets: `sql` is a query **language**
that ships in `wave2` beside data formats, while markup/config **formats** are
scattered across waves. The single source of truth is
`crates/tokenizer-core/src/family.rs` (`Family`, `FORMAT_IDS`, `Preset`); the
Cargo `formats` feature mirrors `FORMAT_IDS` by hand.

Shipped today (8): `json`, `yaml`, `toml`, `url`, `xml`, `html`, `css`,
`markdown` — json/toml/yaml/url/css/markdown are genuine hand-written engines
with their own vocabularies; html/xml share the `core::markup_full` engine.
All 8 now clear the format-specific-depth bar; none is generic-vocabulary
only.

Target list toward "top 20 formats" (text-in / spans-out only):

| planned id | kind | why it earns a slot |
|------------|------|---------------------|
| `json5`, `jsonc`, `jsonl` | data | JSON supertypes/streams already have a deep json engine to extend; high real-world edit surface |
| `csv`, `tsv` | tabular | the most-edited plain format with no engine yet (pilot for the vocabulary contract) |
| `ini`, `properties` | config | trivial grammar, huge surface, distinct `section`/`key`/`value` vocab |
| `logfmt` | log/record | `key=value` line format, pairs with structured logging |
| `hcl` | config-as-code | Terraform/Nomad; nested blocks + heredocs |
| `edn` | data | Clojure data literal, shares reader with `clojure` |
| `plist` | data | Apple config/preferences, XML- and binary- variants |
| `srt`, `vtt` | timed text | subtitle cues, near-identical grammar (vtt as a `srt` dialect) |
| `ics` | calendar | iCalendar line-folded `PROPERTY:params:value` |

Excluded by contract, not by popularity: **binary formats** (`msgpack`,
`cbor`, `parquet`, `avro`, `bson`, `xlsx`, `protobuf` wire) do not fit the
text-in / half-open UTF-8-byte-span-out interface every host adapter exposes.
Their framing is not a byte-slice of the source string, so tokenizing them
would break the losslessness invariant that `verify_lossless_spans` enforces
across this workspace. Protobuf `.proto` and Thrift `.thrift` *IDL* files are
text and remain eligible as languages, not formats.

Editor-grade recovering front-ends, not language-spec compilers.
