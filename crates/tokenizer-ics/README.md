# themoretheless-tokenizer-ics

Engine crate for the `ics` format of [themoretheless-tokenizer](../../README.md).

An iCalendar (RFC 5545) engine: a lossless physical-line lexer, a recovering
logical content-line pass, a property-aware semantic layer and a validator. It
advertises `LEX | PARSE | SEMANTIC | VALIDATE` and nothing else — there is no
node-identity tree, cursor API or visitor here, so no `CST`, `NAVIGATE` or
`VISITOR` capability is claimed.

## What is actually true

1. **Byte-exact under folding.** The lexer never un-folds. A fold (`CRLF` plus
   one space or tab) is emitted as its own `fold-marker` token and every other
   span stays inside one physical line, so concatenating token text
   reconstructs the source byte-for-byte — for well-formed calendars and for
   broken ones alike. Bad spans are flagged, never dropped or synthesized, and
   no layer emits a zero-width span. The parser re-joins the physical runs to
   read a value without moving a single span.
2. **The vocabulary belongs to iCalendar.** `BEGIN`/`END` are
   `structure-marker`, a `:` is a `value-delimiter`, a `;` a
   `parameter-delimiter`, a quoted parameter value is `quoted-param-value`, and
   the bytes after `BEGIN:`/`END:` read as `component-name`. Values are typed by
   the property that owns them, with `VALUE=` overriding: `date-value`,
   `date-time-value`, `duration-value`, `period-value`, `recurrence-value`,
   `uri-value`, `text-value`. There is no `keyword`, `string`, `number`,
   `identifier` or `comment` kind, because iCalendar has no comments and no
   keywords beyond `BEGIN`/`END`.
3. **Seventeen error codes and one warning.** Diagnostics use stable kebab-case
   codes (`unclosed-component`, `component-mismatch`, `unexpected-end`,
   `missing-vcalendar-wrapper`, `property-before-begin`, `missing-component-name`,
   `invalid-content-line`, `missing-value-delimiter`, `nothing-to-fold`,
   `unterminated-quoted-param`, `invalid-escape`, `invalid-line-ending`,
   `malformed-date`, `malformed-date-time`, `malformed-duration`,
   `malformed-period`, `missing-required-property`). Required properties are
   errors, because RFC 5545 §3.6 states them with MUST. `non-uppercase-name` is
   the only warning, because names match case-insensitively and the document
   stays valid; `Parse::is_valid` looks at errors alone.
4. **It stops at the content line.** No recurrence expansion, no time-zone
   resolution, no `DTSTART`/`DUE` consistency rules, no `UTC-OFFSET`/`INTEGER`
   shape checks, no RFC 6868 parameter escaping. `RECUR`, `URI` and `TEXT`
   values are typed but not shape-checked, because the specification gives them
   no closed grammar to enforce here.

## Using it

```rust
use themoretheless_tokenizer_ics::{parse, validate};

let source = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Engine//EN\r\nEND:VCALENDAR\r\n";
let parsed = parse(source);
assert!(parsed.is_valid());
assert_eq!(parsed.components().len(), 1);
assert_eq!(parsed.lexed().joined(), source);
assert!(validate("END:VCALENDAR\r\n").iter().any(|d| d.code == "unexpected-end"));
```

For the host, register `DESCRIPTOR` and `ENGINE` from
`themoretheless_tokenizer_ics` (`Host` implements
`themoretheless_tokenizer_core::HostLanguage`).
