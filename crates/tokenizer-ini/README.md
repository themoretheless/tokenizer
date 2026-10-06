# themoretheless-tokenizer-ini

**Full** INI and Java `.properties` plugin: one dialect-parameterised lossless
lexer, a recovering section/entry pass, value-aware semantic kinds, and
format-only diagnostics. `.properties` is a thin dialect of the same shape, so
both engines live in one crate and differ only through `Options` — on rules that
change what the bytes *mean*, not merely what they are called. `[a]` is a section
header to INI and an ordinary key to `.properties`; `!c` comments only in
`.properties` and `;c` only in INI; a whitespace run separates key from value in
`.properties` and is part of the key in INI. No borrowed programming-language
vocabulary — there is no `identifier`, no `string` and no `number` kind, because
in these formats the structural names are `key`, `value`, `separator` and
`section-marker`.

## Layout

| File | Owns |
| --- | --- |
| `src/lexer.rs` | `SyntaxKind`, `TokenFlags`, `LexToken`, `Options`, `Dialect`, `lex` -> `Lexed` |
| `src/parser.rs` | `DiagnosticKind`, `Section`/`Entry`, `parse` -> `Parse`, `validate` |
| `src/lib.rs` | Crate doc, re-exports, and both host adapters (`Host`/`ENGINE`, `PropertiesHost`/`PROPERTIES_ENGINE`) |

`Host::lex` runs the syntax layer; `Host::semantic_tokens` re-tags whole value
spans in place; `diagnose` projects `Parse::diagnostics`. The descriptors
advertise only what is real — `LEX`, `PARSE`, `SEMANTIC`, `VALIDATE`. There is
no node-identity tree, cursor, or visitor, so `CST`/`NAVIGATE`/`VISITOR` stay
off.

## Lossless contract

Token spans cover every input byte exactly once, in ascending order, with no
zero-width tokens: `Lexed::verify_lossless` (and `is_lossless`) never fails for
`lex` output, including on BOM, CRLF, unclosed sections, unclosed quotes, stray
text after a closing quote, a `\` that ends the input, and empty input. Nothing
is synthesized, so recovery never costs bytes: `Parse::is_valid` goes `false`
while the stream stays complete. A zero-length key or value (`= orphan`,
`empty =`) produces no token and no span; the entry still exists structurally.

## Host kinds

Lex layer: `bom`, `section-marker` (each `[` and `]`), `section-name`, `key`,
`separator` (`=`, `:` or the whitespace separator), `value`, `quote` (each
individual `"`), `comment`, `escape-sequence` (a `\x` pair, `.properties` only),
`line-continuation` (the join itself — an indent in INI, a backslash-plus-break
in `.properties`), `padding` (intra-line blanks), `record-break` (the `\n`,
`\r` or CRLF itself), plus `error` for flagged spans.

Semantic layer: identical kinds and spans, except that a value token covering
its entry's whole value is re-tagged by what it holds — `integer-value`,
`decimal-value`, `boolean-value`. A value fragmented by an escape stays `value`,
and anything inside quotes becomes `quoted-value` (with `quoted-key` on the key
side), so `"42"` never reads as a number.

## Diagnostic codes

`unterminated-section`, `duplicate-section` (a warning; the document stays
valid), `key-outside-section` (a warning, reported once per document),
`unclosed-quote`, `text-after-closing-quote`, `text-after-section-header`,
`invalid-escape` (`.properties` only). INI reports no `invalid-escape` — it has
no escapes, so `\q` is value text.

## Deliberately out of scope

Value *decoding* (no escape expansion, no type coercion beyond the re-tag
above), subsection syntax (`[a:b]`), multi-file profiles, `${var}`
interpolation, and any CST/tree. Section membership is recorded as an index
range into `Parse::entries`, not as a nested structure.
