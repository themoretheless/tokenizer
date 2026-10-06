# themoretheless-tokenizer-logfmt

**Full** logfmt plugin: one lossless lexer, a recovering record pass,
pair-aware semantic kinds, and format-only diagnostics. logfmt is a
whitespace-separated stream of `key=value` pairs, one record per line: a bare
value holds no spaces, a quoted value may hold spaces, raw newlines and the
escapes `\"` and `\\`, and a token with no `=` after it is a key asserted by
presence alone (`-v`, `dry-run`). `=` splits at its first occurrence, so
`a=b=c` is key `a` with bare value `b=c`. No key name is special: `ts`, `level`
and `err` lex exactly like `a`, because this is a format engine, not a log
pipeline.

## Layout

| File | Owns |
| --- | --- |
| `src/lexer.rs` | `SyntaxKind`, `TokenFlags`, `LexToken`, `lex` -> `Lexed` |
| `src/parser.rs` | `DiagnosticKind`, `Record`/`Pair`/`Value`, `parse` -> `Parse`, `validate` |
| `src/lib.rs` | Crate doc, re-exports, and the host adapter (`Host`/`ENGINE`) |

`Host::lex` runs the syntax layer; `Host::semantic_tokens` re-tags value bytes
in place; `diagnose` projects `Parse::diagnostics`. The descriptor advertises
only what is real - `LEX`, `PARSE`, `SEMANTIC`, `VALIDATE`. There is no
node-identity tree, cursor, or visitor, so `CST`/`NAVIGATE`/`VISITOR` stay off.

## Lossless contract

Token spans cover every input byte exactly once, in ascending order, with no
zero-width tokens: `Lexed::verify_lossless` (and `is_lossless`) never fails for
`lex` output, including on CRLF, lone `\r`, leading BOM, unterminated quotes,
multi-byte characters, and empty input. Nothing is synthesized, so recovery
never costs bytes: an unterminated quoted value flags its opening run, a stray
`=` is kept as a flagged separator, and text welded onto a closing quote is
kept as an `error` run - `Parse::is_valid` is `false` while the stream stays
complete. A value with no bytes (`msg=`) contributes no token and no span; the
structure still counts the pair.

## Host kinds

Lex layer: `bom`, `key` (before a `=`), `separator` (the `=` itself),
`bare-value`, `quoted-value` (the literal runs of a quoted value, quotes
included), `escaped-char` (one `\"` or `\\` pair), `flag-key` (a bare token
with no `=`), `whitespace` (spaces and tabs inside a record), `record-break`
(the `\r\n`, `\n` or `\r` itself), plus `error` for flagged spans.

Semantic layer: identical kinds and spans, except that

* the `=` of a value-less pair is re-tagged `empty-value` - an empty value has
  no bytes, so inventing a zero-width token would break the lossless contract
  and the separator carries the reading instead, and
* a *bare* value is typed by what its bytes are: `integer-value` (`-?` digits),
  `float-value` (a `.` and/or an exponent), `boolean-value` (case-insensitive
  `true`/`false`), `null-value` (lowercase `null` or `nil`).

Quoted values are excluded from typing by construction, so a quoted `"007"`
stays `quoted-value` and never lights up as a number, and the reading of a
value depends on its own bytes only - never on its key.

## Diagnostic codes

`unterminated-value`, `missing-key`, `unexpected-token`. A representative valid
document produces none of them: blank lines, trailing spaces, empty values,
flag keys, quoted values spanning record breaks and repeated keys are all
ordinary logfmt.

## Deliberately out of scope

Key-name semantics (`level`/`ts`/`msg` conventions), log-level or timestamp
decoding, comment syntax (logfmt has none), spaces around `=` (`key = value`
is `missing-key`, not a variant of a pair), unescaping quoted values into
decoded buffers, and any CST/tree. `Lexed`/`Parse` never reorder, coerce, trim,
or synthesize bytes.
