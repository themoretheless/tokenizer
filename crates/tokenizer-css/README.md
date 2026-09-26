# themoretheless-tokenizer-css

A purpose-built CSS engine: a lossless, context-aware lexer, a recovering structural pass, and a
light rule list. It depends only on `themoretheless-tokenizer-core`.

## Surface

| Item | Purpose |
| --- | --- |
| `lex(source) -> Lexed` | CSS tokens with byte spans; `verify_lossless()`, `joined()`, `significant_tokens()` |
| `parse(source) -> Parse` | `Lexed` + diagnostics + rules; `is_valid()`, `diagnostics()`, `lexed()`, `rules()` |
| `validate(source) -> Vec<Diagnostic>` | Lexical and structural diagnostics only |
| `SyntaxKind`, `LexToken`, `Lexed`, `Parse`, `Rule`, `RuleKind` | Public vocabulary |
| `ENGINE`, `Host`, `DESCRIPTOR` | Host adapter (`HostLanguage`) registered by the facade crate |

## Why the lexer carries context

CSS reuses punctuation, so a generic C-like vocabulary cannot describe it. The lexer keeps a
brace-depth counter plus a selector-vs-declaration mode, and inside a block it scans ahead to the
first top-level `;`, `{` or `}` to decide whether the item is a nested rule or a declaration. That
one lookahead is what tells `:hover {` from `color: red;`, `#title` (id selector) from `#fff`
(value color), and `.btn` (class) from `1 .5` (number). The mode drives every ambiguous byte:
`--x` is a variable in a definition and in `var(--x)`, `&` is a nesting selector only in a
prelude, and a number's unit is emitted as its own `unit` token right after the `number`.

## Losslessness

Token texts concatenate back to the source byte for byte, with ascending, non-overlapping spans
and no gaps. Nothing is dropped or merged: an unterminated string, comment, block, `@import` or
`!important` marker becomes one error-flagged token that still covers its exact bytes, and every
diagnostic points at an existing token. All spans are UTF-8 byte offsets on char boundaries.

## Diagnostics

Stable kebab-case codes: `unclosed-block`, `unclosed-string`, `unclosed-comment`,
`unclosed-function`, `unclosed-paren`, `unclosed-bracket`, `unexpected-close-brace`,
`unexpected-close-paren`, `missing-semicolon`, `empty-selector`, `empty-declaration`,
`expected-colon`, `expected-value`, `invalid-hex-color`.

`empty-declaration` covers both an empty block and a `;` or `:` with nothing before it, so the
code always means "a block or declaration produced no content". Recovery never consumes or drops
bytes: a `;`, `{` or `}` resynchronizes the lexer, and the structural pass reports every delimiter
the lexer released.

## Capabilities and non-goals

`DESCRIPTOR` advertises LEX, PARSE, SEMANTIC and VALIDATE. There is no CST, cursor navigation or
visitor API here, and no formatting layer: the engine only answers "what are these bytes, and what
is wrong with them". Named colors are deliberately not hard-coded; `color` covers `#`-hex literals
in value position, and other color functions lex as `function` tokens.
