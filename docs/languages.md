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
| `formats` | the 20 data/config/markup/timed-text formats (`core::family::FORMAT_IDS`) |
| `wave1`…`wave7` | delivery batches |
| `all-languages` | everything (69 engines) |

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

`LanguageDescriptor.capabilities` is a coarse bitset. It used to be almost
purely informational: **67 of the 69** registered engines advertised the
identical `LEX|PARSE|SEMANTIC|VALIDATE`, which told a host nothing.

That changed when the badge was measured rather than inherited. `VALIDATE`
claims that the grammar rejects input its own specification forbids. Measured
against the playground's own valid samples, the shared `fullkit` parser behind
the wave languages raised `unexpected-token` / `expected-token` on **25 of 69**
samples (valid go `s := "hi"` among them) while `function f( {` came back with
zero diagnostics — wrong in both directions, so it cannot sell a verdict.
`fullkit::FULL_ENGINE_CAPS` stops at `LEX|PARSE|SEMANTIC`, and an engine adds
`VALIDATE` to its own descriptor only when it has a grammar that earns it.

The parser was then made to keep that promise. Both diagnostic sites are gone:
an unrecognized token in expression position is *recovered over*, not reported,
because it is evidence of a construct the shared grammar does not model rather
than evidence of a broken document. What `fullkit` can still prove is
structural and language-independent, so that is what it reports: bracket
balance over the token stream (`unclosed-delimiter`, `stray-delimiter`, from
`fullkit::check_delimiters`) plus the literals the lexer already checked
(`unclosed-string`, `unclosed-block-comment`). Measured on the corpus each
engine ships as valid — 49 language fixtures + 20 format fixtures, both layers —
that is **0 diagnostics on 69 documents** (down from 76 across 25 engines), and
**11 of 11 broken shapes** across the brace, lisp-paren, sql-paren and shell
families come back flagged. Two gates hold it:
`every_engine_is_quiet_on_its_valid_fixture` and
`every_language_engine_flags_unbalanced_delimiters` in
`tests/wiring_completeness.rs`, with
`a picker sample is a valid document, and the language fixtures track it` in
`playground/tests/catalog.test.js` replaying the same claim against
`playground/src/languages.js` itself and against the byte-for-byte export
(`npm run export:language-fixtures`) the Rust gate reads. One real defect fell
out of that corpus: a javascript template literal used to lex as punctuation
plus identifiers, so `` `a{b` `` was reported as an unclosed `{`. A backtick now
opens a quoted span for the five profiles that spell variables with `$` — groovy,
javascript, php, perl, typescript — which leaves haskell's infix backticks, sql's
quoted identifiers and bash's command substitution as punctuation: a highlighting
gap in the last case, never a false error. A format that claims
validation must also parse its own dialect: `tokenizer-html` initially shared the strict XML reader in
`core::markup_full` and flagged 12 of 20 valid HTML documents, so
`parse_markup_as` takes a `MarkupFlavor` (`Xml` | `Html5`) and HTML got void elements, raw-text
`<script>`/`<style>` bodies, omitted end tags and case-folded names. The HTML picker sample and
`tests/fixtures/formats/html.txt` are that document, so the fixture gate and the truncation sweep cover
the dialect rather than a one-line `<div>`.

The partition is now:

| advertised caps | engines | who |
|-----------------|---------|-----|
| `LEX|PARSE|SEMANTIC` | 49 | every `fullkit` wave language |
| `LEX|PARSE|SEMANTIC|VALIDATE` | 18 | hand-written format engines |
| `LEX|PARSE|SEMANTIC|VALIDATE|CST|NAVIGATE|VISITOR` | 1 | `json` |
| `LEX|VALIDATE` | 1 | `url` |

i.e. **exactly the 20 formats advertise validation, and no language does** —
pinned twice, by `only_format_engines_advertise_validate` in
`tests/wiring_completeness.rs` against the registry, and by
`only the format family advertises validation` plus
`an engine that claims validation is quiet on valid input` in
`playground/tests/catalog.test.js`: the second one replays all 73
`expectValid: true` cases of the badge-holding engines and fails if any of them
comes back flagged. Those 73 cover 13 of the 20; the rest (markdown, css, toml,
yaml, html, xml, url) are covered instead by the Rust fixture gate, which
requires a zero-diagnostic parse of each format's own document. Either way the
badge is earned on corpus, not asserted in a table.

What a capability column still does *not* tell you is how well an engine
understands its format.

The real bar is **measured vocabulary depth**: how many token kinds an engine
emits that are specific to *its* format, i.e. outside the shared generic
vocabulary (`class, comment, function, identifier, keyword, number,
punctuation, string, type, variable, whitespace`). The playground matrix
computes this live by tokenizing each engine's sample and subtracting that
generic set; `playground/tests/catalog.test.js` pins the result as a tripwire.

For the 20 formats, measured on their playground samples:

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
| json5 | lex\|parse\|semantic\|validate | no | `unquoted-key`/`quoted-key`/`single-quoted-key` (syntax) or `property` (semantic), `single-quoted-string`, `hex-number`, `infinity`, `nan`, `trailing-comma`, `line-comment` | json engine with JSON5 lexer/parser options |
| jsonl | lex\|parse\|semantic\|validate | no | `record-break` plus the json structural kinds | json engine framed one strict record per line |
| csv | lex\|parse\|semantic\|validate | no | `header-field`, `field`, `quoted-field`, `integer-field`, `decimal-field`, `boolean-field`, `quote`, `escaped-quote`, `delimiter`, `record-break` | hand-written delimiter engine |
| tsv | lex\|parse\|semantic\|validate | no | same tabular vocabulary minus `quote`/`escaped-quote` (TSV has no quoting) | the csv crate with `Delimiter::Tab` |
| logfmt | lex\|parse\|semantic\|validate | no | `key`, `separator`, `bare-value`, `quoted-value`, `integer-value`, `boolean-value`, `null-value`, `record-break` | hand-written key=value engine |
| ini | lex\|parse\|semantic\|validate | no | `section-marker`, `section-name`, `key`, `separator`, `value`, `quote`, `quoted-value`, `padding`, `integer-value`, `boolean-value`, `record-break` | hand-written dialect-table engine |
| properties | lex\|parse\|semantic\|validate | no | `key`, `separator`, `value`, `padding`, `escape-sequence`, `line-continuation`, `integer-value`, `decimal-value`, `boolean-value`, `record-break` | the ini crate with the properties dialect |
| hcl | lex\|parse\|semantic\|validate | no | `block-type`, `block-label`, `attribute-name`, `heredoc-open`/`heredoc-body`/`heredoc-close`, `interpolation`, `escape`, `boolean`, `null`, `operator` | hand-written recovering parser |
| edn | lex\|parse\|semantic\|validate | no | `symbol`, `namespaced-symbol`/`namespaced-keyword`/`namespaced-map-*`, `list-open`/`vector-open`/`map-open`/`set-open`, `tag`/`instant-tag`/`uuid-tag`/`byte-tag`, `discard`/`discarded-form`, `radix-integer`, `ratio`, `bigint`, `character` | hand-written reader |
| srt | lex\|parse\|semantic\|validate | no | `cue-index`, `cue-identifier`, `cue-start`, `cue-end`, `timing-arrow`, `cue-text`, `cue-text-continuation`, `record-break`, `bom` | hand-written timed-text engine |
| vtt | lex\|parse\|semantic\|validate | no | srt's cue kinds plus `signature`, `signature-note`, `block-marker`, `header-*`, `style-*`, `region-*`, `setting-*`, `line-value`/`position-value`/`size-value`/`alignment-value`/`vertical-value`, `voice-tag`/`class-tag`/`emphasis-tag`/`closing-tag`/`timing-tag`, `markup-*` | the srt crate with the WebVTT dialect |
| ics | lex\|parse\|semantic\|validate | no | `structure-marker`, `component-name`, `property-name`, `value-delimiter`, `parameter-*`, `quoted-param-value`/`bare-param-value`, `date-value`/`date-time-value`/`duration-value`/`period-value`/`recurrence-value`, `uri-value`, `text-value`, `escaped-char`, `fold-marker`, `line-break` | hand-written recovering parser |

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

Shipped today (20): `json`, `json5`, `jsonl`, `yaml`, `toml`, `url`, `xml`,
`html`, `css`, `markdown`, `csv`, `tsv`, `logfmt`, `ini`, `properties`, `hcl`,
`edn`, `srt`, `vtt`, `ics` — all of them hand-written engines with their own
`SyntaxKind` vocabularies except `json5`/`jsonl`, which extend the json engine
(lexer/parser options and a record framer) rather than duplicating its 4.5k
lines, and `vtt`, which is a dialect of the srt crate. `properties` is a dialect
of the ini crate, and html/xml share `core::markup_full`.
Every one of the 20 clears the depth bar on its
`tests/fixtures/formats/<id>.txt` sample, which `every_format_engine_knows_its_own_vocabulary`
pins as valid, zero-diagnostic and non-generic-vocabulary:

| engine | format-specific kinds | diagnostics |
|--------|----------------------|-------------|
| `json5` | 13 (`unquoted-key`, `single-quoted-key`, `hex-number`, `infinity`, `nan`, `trailing-comma`, `line-comment`, …) | 0 |
| `jsonl` | 9 (`record-break` plus the json structural kinds) | 0 |
| `csv` | 9 (`header-field`, `quoted-field`, `integer-field`, `decimal-field`, `boolean-field`, `quote`, `escaped-quote`, `delimiter`, `record-break`) | 0 |
| `tsv` | 7 (same tabular vocabulary; `quote` cannot occur — TSV has no quoting) | 0 |
| `logfmt` | 8 (`key`, `separator`, `bare-value`, `quoted-value`, `integer-value`, `boolean-value`, `null-value`, `record-break`) | 0 |
| `ini` | 11 (`section-marker`, `section-name`, `key`, `separator`, `value`, `quote`, `quoted-value`, `padding`, `integer-value`, `boolean-value`, `record-break`) | 0 |
| `properties` | 10 (`key`, `separator`, `value`, `padding`, `escape-sequence`, `line-continuation`, `integer-value`, `decimal-value`, `boolean-value`, `record-break`) | 0 |
| `hcl` | 14 (`block-type`, `block-label`, `attribute-name`, `heredoc-open`, `heredoc-body`, `heredoc-close`, `interpolation`, `escape`, `line-comment`, `block-comment`, `boolean`, `null`, `operator`, `newline`) | 0 |
| `edn` | 31 (`symbol`, `namespaced-symbol`, `namespaced-keyword`, `namespaced-map-key`, `namespaced-map-prefix`, `list-open`, `vector-open`, `map-open`, `set-open`, `tag`, `instant-tag`, `uuid-tag`, `byte-tag`, `discard`, `discarded-form`, `radix-integer`, `ratio`, `bigint`, `decimal`, `character`, `string-escape`, …) | 0 |
| `srt` | 9 (`cue-index`, `cue-identifier`, `cue-start`, `cue-end`, `timing-arrow`, `cue-text`, `cue-text-continuation`, `record-break`, `bom`) | 0 |
| `vtt` | 32 (the srt cue kinds plus `signature`, `signature-note`, `block-marker`, `header-name`, `header-separator`, `style-rule`, `style-reference`, `region-property`, `region-value`, `region-reference`, `setting-name`, `setting-separator`, `line-value`, `position-value`, `size-value`, `vertical-value`, `voice-tag`, `class-tag`, `emphasis-tag`, `closing-tag`, `timing-tag`, `markup-punctuation`, `markup-value`, …) | 0 |
| `ics` | 20 (`structure-marker`, `component-name`, `property-name`, `value-delimiter`, `value-list-delimiter`, `parameter-delimiter`, `parameter-assignment`, `parameter-name`, `quoted-param-value`, `bare-param-value`, `date-value`, `date-time-value`, `duration-value`, `period-value`, `recurrence-value`, `uri-value`, `text-value`, `escaped-char`, `fold-marker`, `line-break`) | 0 |

`csv` and `tsv` are one crate (`tokenizer-csv`) with the delimiter as the only
parameter, and `tsv` is a Cargo alias of `csv`; `json5`/`jsonl` are features of
`tokenizer-json`. `ini` and `properties` are one crate (`tokenizer-ini`) whose
two hosts differ only through the dialect table in `Options` — sections, comment
introducers, escapes, and which join mechanism exists — and `properties` is a
Cargo alias of `ini`. `tokenizer-srt` repeats that shape for timed text: SubRip
and WebVTT share the cue-block lexer and parser, and `Dialect::{Srt, Vtt}` pick
one 10-flag `Options` table (signature required, `NOTE`/`STYLE`/`REGION` blocks,
optional hours, `,` vs `.` milliseconds, inline markup, ordinal indices), so
`vtt` is a Cargo alias of `srt`. Across the whole build-out that is 12 new
format ids on 7 new crates.

Those fixtures are also cut up, not just parsed whole.
`no_format_engine_panics_or_stalls_on_a_truncated_document` re-parses each of the
20 at every character boundary and requires the token stream to tile that prefix
exactly — no gap, no overlap, no zero-length token — inside a 40 ms-per-prefix
budget, which is what turns "recovers instead of hanging" from prose into a
merge gate. `every_format_diagnostic_code_is_kebab_case` walks the same corpus
and checks every code those truncations emit: kebab-case, non-empty message,
span inside the input. Its floor is aggregate (≥40 distinct codes over all 20)
rather than per-format, because a grammar that accepts every prefix of its own
sample is legitimate — `yaml` is one — and a per-format floor would only teach
the sweep to pick fixtures that produce noise.

The gate earned its keep on the first run: `tokenizer-url` emitted a zero-length
`Port` token for `http://host:`, the one shape where a slot is announced and
left empty. Since `verify_lossless_spans` forbids empty spans everywhere, the
empty port is now carried as `UrlTokenization::empty_port`, an `Option<Span>`
fact about the authority that `url-empty-port` still reports at the same offset,
while the colon keeps the `u-sep` token it already had.
A slot with no byte in it is a diagnostic, not a token.

The 20 slots are full (text-in / spans-out only). The candidates that were held
back are documented here so the next review does not re-litigate them:

| candidate | kind | why it did not earn a slot |
|-----------|------|----------------------------|
| `plist` | data | both variants are carriers for vocabularies already tokenized (`xml` for the text one, and the binary one is out of contract), so it earns a slot only if a real preference surface shows up |
| `jsonc` | data | already ships as a *dialect* of the json engine (`modes: strict | jsonc`); a format that is a dialect of a shipped engine does not need its own id |

Excluded by contract, not by popularity: **binary formats** (`msgpack`,
`cbor`, `parquet`, `avro`, `bson`, `xlsx`, `protobuf` wire) do not fit the
text-in / half-open UTF-8-byte-span-out interface every host adapter exposes.
Their framing is not a byte-slice of the source string, so tokenizing them
would break the losslessness invariant that `verify_lossless_spans` enforces
across this workspace. Protobuf `.proto` and Thrift `.thrift` *IDL* files are
text and remain eligible as languages, not formats.

Editor-grade recovering front-ends, not language-spec compilers.
