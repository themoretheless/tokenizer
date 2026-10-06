# themoretheless-tokenizer-csv

**Full** CSV and TSV plugin: one delimiter-parameterised lossless lexer, a
recovering record pass, field-aware semantic kinds, and format-only diagnostics.
CSV is RFC 4180 with its usual leniencies (a field that merely contains a `"`
is ordinary text; a quoted field may span CRLFs). TSV is the same grammar with
the tab as delimiter and quoting **off**: a `"` is field text and a comma is
field text. No borrowed programming-language vocabulary — there is no
`comment`, no `string`, and no `whitespace` kind, because in this format a
space is field content and a newline is a `record-break`.

## Layout

| File | Owns |
| --- | --- |
| `src/lexer.rs` | `SyntaxKind`, `TokenFlags`, `LexToken`, `Delimiter`, `Options`, `lex` → `Lexed` |
| `src/parser.rs` | `DiagnosticKind`, `Record`/`Field`, `parse` → `Parse`, `validate` |
| `src/lib.rs` | Crate doc, re-exports, and both host adapters (`Host`/`ENGINE`, `TsvHost`/`TSV_ENGINE`) |

`Host::lex` runs the syntax layer; `Host::semantic_tokens` re-tags field text
in place; `diagnose` projects `Parse::diagnostics`. The descriptors advertise
only what is real — `LEX`, `PARSE`, `SEMANTIC`, `VALIDATE`. There is no
node-identity tree, cursor, or visitor, so `CST`/`NAVIGATE`/`VISITOR` stay off.

## Lossless contract

Token spans cover every input byte exactly once, in ascending order, with no
zero-width tokens: `Lexed::verify_lossless` (and `is_lossless`) never fails for
`lex` output, including on CRLF, lone `\r`, leading BOM, ragged rows, unclosed
quotes, and empty input. Nothing is synthesized, so recovery never costs bytes:
an unclosed quoted field flags its opening `"` and a stray run after a closing
`"` is kept as an `error` span — `Parse::is_valid` is `false` while the stream
stays complete. A zero-length field (`a,,b`) contributes no token and no span;
the structure still counts it as a field.

## Host kinds

Lex layer: `bom`, `header-field` (field text of the first record), `field`,
`quote` (each individual `"`), `escaped-quote` (a `""` pair inside a quoted
field), `delimiter`, `record-break` (the `\r\n`, `\n` or `\r` itself), plus
`error` for flagged spans.

Semantic layer: identical kinds and spans, except that a plain `field` token is
re-tagged by what it holds — `quoted-field` (so a quoted `"007"` stays text),
`integer-field` (`-?` digits), `decimal-field` (optional sign with a `.` and/or
exponent), `boolean-field` (case-insensitive `true`/`false`). `header-field`,
`quote`, `delimiter` and `record-break` never change.

## Diagnostic codes

`unclosed-quote`, `text-after-closing-quote`, `ragged-row`. TSV can only report
`ragged-row`: its quotes are literal text and cannot be unclosed.

## Deliberately out of scope

Column-name semantics, header inference for headerless files, other delimiters
(semicolon dialects arrive as options, not new kinds), BOM sniffing beyond one
leading U+FEFF, and any CST/tree. `ragged-row` compares against the first
record only; the engine never reorders, coerces, or trims field bytes.
