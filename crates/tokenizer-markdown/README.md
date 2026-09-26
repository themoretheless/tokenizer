# themoretheless-tokenizer-markdown

**Full** Markdown plugin: a real lossless lexer, a recovering structural pass, and
Markdown-only diagnostics. No `fullkit` and no borrowed TOML/YAML vocabulary —
Markdown is a markup language, so `#` is a heading marker and there is no
`comment` kind at all.

## Layout

| File | Owns |
| --- | --- |
| `src/lexer.rs` | `SyntaxKind`, `TokenFlags`, `LexToken`, `lex` → `Lexed` |
| `src/parser.rs` | `BlockKind`/`Block`, `DiagnosticKind`, `parse` → `Parse`, `validate` |
| `src/lib.rs` | Crate doc, re-exports, and the host adapter (`Host`, `ENGINE`, `DESCRIPTOR`) |

`Host::lex`/`semantic_tokens`/`diagnose` all run `parse` and project it into
`HostTokenization`; the descriptor advertises only what is real — `LEX`, `PARSE`,
`SEMANTIC`, `VALIDATE`. There is no node-identity tree, cursor, or visitor, so
`CST`/`NAVIGATE`/`VISITOR` stay off.

## Lossless contract

Token spans cover every input byte exactly once, in ascending order, with no
zero-width tokens: `Lexed::verify_lossless` (and `is_lossless`) never fails for
`lex` output, including on CRLF, tabs, truncated multibyte input, and empty input.
Nothing is synthesized, so a recovery never costs bytes: an unclosed construct
keeps its own kind with `TokenFlags::HAS_ERROR` — the host projects flagged tokens
to `"error"` — and `Parse::is_valid` is `false` while the stream stays complete.

## Host kinds

Block: `heading-marker`, `heading-text`, `setext-underline`, `thematic-break`,
`blockquote-marker`, `list-marker`, `code-fence-marker`, `fence-info`,
`code-block-line`, `table-delimiter`, `table-pipe`, `table-cell`, `link-label`,
`link-destination`, `front-matter-delimiter`, `front-matter`, `html-block`.
Inline and trivia: `hard-break`, `line-break`, `whitespace`, `strong`,
`emphasis`, `strikethrough`, `code-span`, `link-text`, `image-marker`, `autolink`,
`email-autolink`, `footnote-ref`, `footnote-definition-label`, `html-inline`,
`punctuation`, `text`, plus `error` for flagged spans.

A whole inline construct is one token (its markers included), a `*` or `-` at line
start is a list marker rather than emphasis, `---` under a paragraph is a setext
underline rather than a thematic break, and a fenced body is never lexed inline.

## Diagnostic codes

`unclosed-code-fence`, `unclosed-emphasis`, `unclosed-code-span`,
`unclosed-html-tag`, `unclosed-link`, `invalid-heading-level`, `empty-heading`,
`table-column-mismatch`, `undefined-footnote-reference`,
`duplicate-link-definition`.

## Deliberately out of scope

CommonMark reference-link resolution, indented code blocks, backslash escapes,
nested emphasis inside table cells, and inline lexing across a line break. An
unclosed construct is reported on the line that opened it.
