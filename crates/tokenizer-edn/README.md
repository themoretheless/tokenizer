# themoretheless-tokenizer-edn

**Full** EDN plugin: one lossless lexer, a recovering structure pass,
tag- and key-aware semantic kinds, and format-only diagnostics. EDN
(edn-format) is a data notation: values in collections, `#tag`s and `#_`
discards over a grammar where commas are whitespace, `;` starts a comment, and
`#{` opens a set. This engine lexes that grammar as written — `#inst`,
`#uuid`, `#b`, `#:ns` namespaced maps, radix integers (`16rFF`), ratios
(`3/4`), exact decimals (`7M`), bigints (`1000N`) and the special values
(`##Inf`, `##-Inf`, `##NaN`) each get their own kind — and keeps Clojure-only
reader macros (`'`, `` ` ``, `@`, `^`, `~`, `#(`, `#'`, `::kw`) out of the
data subset, flagging them instead.

## Layout

| File | Owns |
| --- | --- |
| `src/lexer.rs` | `SyntaxKind`, `TokenFlags`, `LexToken`, `lex` -> `Lexed` |
| `src/parser.rs` | `DiagnosticKind`, `Retag`, `parse` -> `Parse`, `validate` |
| `src/lib.rs` | Crate doc, re-exports, and the host adapter (`Host`/`ENGINE`) |

`Host::lex` runs the syntax layer; `Host::semantic_tokens` applies the
structure readings in place; `diagnose` projects `Parse::diagnostics`. The
descriptor advertises only what is real — `LEX`, `PARSE`, `SEMANTIC`,
`VALIDATE`. There is no node-identity tree, cursor, or visitor, so
`CST`/`NAVIGATE`/`VISITOR` stay off.

## Lossless contract

Token spans cover every input byte exactly once, in ascending order, with no
zero-width tokens: `Lexed::verify_lossless` (backed by
`core::lossless::verify_lossless_spans`) never fails, including for empty
input, whitespace-and-commas-only input, a leading BOM, a comment reaching EOF
without a newline, an unterminated collection or string, non-ASCII symbols,
and a trailing `#_` with nothing after it. Every dispatch branch consumes at
least one byte, so `#`, `\`, `#{`, `#b` and friends cannot stall. An
unterminated string flags its whole region and keeps its bytes; a broken
number or a stray byte becomes a flagged `error` token; `Parse::is_valid` is
`false` while the stream stays complete.

## Host kinds

Lex layer (33): `bom`, `whitespace` (spaces, tabs, newlines, form feed — and
commas), `comment`, `string` (the literal runs of a string, quotes included),
`string-escape` (one `\"` or `\uXXXX` pair), `character`, `symbol`,
`namespaced-symbol`, `keyword`, `namespaced-keyword`, `boolean-literal`,
`nil-literal`, `integer`, `bigint`, `radix-integer`, `float`, `decimal`,
`ratio`, `special-number`, `list-open`, `list-close`, `vector-open`,
`vector-close`, `map-open`, `map-close` (the `}` shared by maps and sets),
`set-open` (the two bytes `#{`), `discard` (`#_`), `tag`, `instant-tag`,
`uuid-tag`, `byte-tag`, `namespaced-map-prefix` (`#:ns`), plus `error` for
flagged spans.

Semantic layer: identical kinds and spans, except that structure re-reads
eight things in place — every token of a discarded form becomes
`discarded-form`; the first token of `#inst`/`#uuid`/`#b`/any-tag's value
becomes `instant-value`/`uuid-value`/`byte-value`/`tagged-value`; a repeated
map key or set element becomes `duplicate-key`/`duplicate-set-element`; and a
plain keyword directly under a `#:ns` prefix becomes `namespaced-map-key`.
The error flag wins over any reading. The fixture of the crate tests measures
31 non-generic kinds outside the repository's generic set.

## Diagnostic codes

Stable kebab-case codes (18): `discard-without-value`, `duplicate-key`,
`duplicate-set-element`, `incomplete-dispatch`, `invalid-escape`,
`invalid-number`, `invalid-radix`, `invalid-radix-digit`, `invalid-symbol`,
`invalid-tagged-value`, `malformed-char`, `mismatched-close`,
`namespaced-map-without-map`, `non-edn-construct`, `odd-map-entries`,
`tag-without-value`, `unterminated-collection`, `unterminated-string`. A
representative valid document — tagged values, discards, a namespaced map,
radix ints, ratios, `M` decimals and escapes — produces none of them.
Duplicate detection follows the spec's typed equality: `255` repeats `16rFF`
and `2` repeats `4/2`, but `1` never repeats `1.0`, and compound forms
(vectors, tagged values, error spans) are not compared.

## Deliberately out of scope

Top level is a stream: the spec defines no enclosing element, so several
top-level forms are accepted without warning and only their spans are
recorded (`Parse::top_level_forms`). Tag *handlers* are not interpreted —
`#my/tag` is recognized and its value located, never invoked — and `#inst`
payloads are not RFC 3339-validated. String and character identities for
duplicate detection compare raw source bytes, and float/decimal identities
canonicalize through `f64`, so exotic values beyond 64-bit range or
zero-padded bigints compare as distinct. `#=`/`#'`/vars, namespacing beyond
one map level, and any CST/tree are absent by design.
