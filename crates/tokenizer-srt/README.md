# themoretheless-tokenizer-srt

One crate, two registered engines: **SubRip (`.srt`)** and **WebVTT (`.vtt`)**.
WebVTT is modelled as a dialect of the same lexer and parser, the way `properties`
is a dialect of `ini` — the two formats disagree only through the ten flags in
[`Options`], so the disagreement is a table rather than duplicated code.

Part of [themoretheless-tokenizer](../../README.md). Both engines are registered under
`LanguageId::SRT` and `LanguageId::VTT`.

## Layout

| File | Role |
| --- | --- |
| `src/lexer.rs` | line-oriented scanner, `Dialect`, the `Options` dialect table, `SyntaxKind` |
| `src/parser.rs` | cue structure, header fields, timing/ordering checks, `DiagnosticKind` |
| `src/lib.rs` | the two `HostLanguage` adapters, semantic retagging, `LanguageDescriptor`s, tests |

Public surface for the facade:

- `pub static ENGINE: Host` with `DESCRIPTOR` (`srt`)
- `pub static VTT_ENGINE: VttHost` with `VTT_DESCRIPTOR` (`vtt`)
- `pub use {Dialect, Options, SyntaxKind, Lexed, LexToken, TokenFlags, lex, Cue, Parse, Setting, Timestamp, HeaderField, DiagnosticKind, parse, validate}`

## Contract

Half-open UTF-8 byte spans, never zero-length, and every layer concatenates back
to the source byte for byte — checked by `verify_lossless` over the two fixtures,
an 18-case corpus (BOM, CRLF against LF, emoji, unterminated blocks) and a
14-case pathological corpus (`-->`, `00:`, `<i`, `0:,0`, a lone BOM pair), under
`every_corpus_entry_lexes_to_a_lossless_stream_for_both_ids`,
`both_host_layers_reconstruct_the_source` and
`malformed_input_never_panics_or_stalls`. Capabilities are `LEX | PARSE | SEMANTIC | VALIDATE`;
no CST, cursor or visitor is offered, so none is claimed.

The two layers share one span per token: the semantic layer only re-tags the
syntax layer's tokens, which is what `the_semantic_layer_never_moves_a_span` asserts.

## Where the dialects disagree

| Axis | SubRip | WebVTT |
| --- | --- | --- |
| Document header | none; `WEBVTT` would be prose | `WEBVTT` signature, optional note, `Name: value` programmatic headers |
| Blocks | none | `NOTE`, `STYLE` (`::cue` rules), `REGION` |
| Cue identity | digits-only line is an ordinal `cue-index` | same bytes are an opaque `cue-id` |
| Timestamp | `hh:mm:ss,mmm`, hours and millis required | `mm:ss[.SSS]` allowed, `.` separator, millis optional |
| Cue settings | not a thing (`unexpected-timing-text`) | `align:`, `position:`, `size:`, `line:`, `vertical:` |
| Inline markup | tags are prose | `<v>`, `<c.class>`, `<i>`, `<u>`, `<ruby>`, `<00:00:33.500>` timing tags |

## Lexical vocabulary

- `bom`
- `cue-identifier`
- `cue-index`
- `cue-id`
- `cue-text`
- `record-break`
- `signature`
- `signature-comment`
- `header-name`
- `header-separator`
- `header-value`
- `block-marker`
- `region-property`
- `region-separator`
- `region-value`
- `style-content`
- `timing-arrow`
- `time-hour`
- `time-minute`
- `time-second`
- `time-separator`
- `millisecond`
- `millisecond-separator`
- `setting-name`
- `setting-separator`
- `setting-value`
- `markup-punctuation`
- `markup-name`
- `markup-value`
- `whitespace`
- `error`

## Semantic roles

SubRip — 9 kinds the generic vocabulary has no word for:

- `bom`
- `cue-identifier`
- `cue-index`
- `cue-text`
- `cue-start`
- `cue-end`
- `cue-text-continuation`
- `record-break`
- `timing-arrow`

WebVTT — 32:

- `alignment-value`
- `block-marker`
- `class-tag`
- `closing-tag`
- `cue-end`
- `cue-id`
- `cue-start`
- `cue-text`
- `cue-text-continuation`
- `emphasis-tag`
- `header-name`
- `header-separator`
- `line-value`
- `markup-punctuation`
- `markup-value`
- `position-value`
- `record-break`
- `region-property`
- `region-reference`
- `region-separator`
- `region-value`
- `setting-name`
- `setting-separator`
- `signature`
- `signature-note`
- `size-value`
- `style-reference`
- `style-rule`
- `timing-arrow`
- `timing-tag`
- `vertical-value`
- `voice-tag`

## Diagnostics

| Code | Severity | Dialect | Meaning |
| --- | --- | --- | --- |
| `malformed-timestamp` | error | both | timestamp does not match this dialect's grammar |
| `missing-arrow` | error | both | timing line has no `-->` arrow |
| `missing-blank-line` | warning | both | cues must be separated by an empty line |
| `missing-signature` | error | vtt | WebVTT must open with a `WEBVTT` signature line |
| `missing-timestamp` | error | both | timing arrow needs exactly two timestamps |
| `missing-timing-line` | error | both | cue block has no timing line |
| `non-monotonic-index` | warning | srt | cue index does not follow the previous index |
| `overlapping-cues` | warning | both | cue starts before the previous cue ends |
| `start-after-end` | error | both | cue does not end after it starts |
| `text-before-timing` | error | both | cue text appears before the timing line |
| `unexpected-header-text` | error | vtt | text after `WEBVTT` must be a note after a hyphen |
| `unexpected-timing-text` | warning | srt | SubRip timing lines carry no settings |
| `unknown-block-type` | warning | vtt | block type is not one WebVTT defines |
| `unknown-cue-setting` | warning | vtt | cue setting is not one WebVTT defines |
| `unterminated-markup` | error | vtt | cue text tag is never closed |

Codes are kebab-case, each round-trips through `from_code` with its severity, and
`DiagnosticKind::ALL` is ordered by code — asserted by
`every_diagnostic_code_round_trips_and_has_a_severity` and
`dialect_only_codes_are_declared_as_such`.

## Representative documents

`srt`: BOM, two numbered cues and one `coda` identity line, with em dashes and an
emoji in cue text (`SRT_SAMPLE`). Both fixtures report zero diagnostics on both
layers (`both_representative_fixtures_are_clean`), and the semantic sets above
clear the depth bar in `both_semantic_layers_clear_the_depth_bar` while
`neither_layer_borrows_a_generic_vocabulary` keeps the rename-cheating out.
`vtt`: signature with note, both header fields, all three block types, three cues
carrying voice/class/timing tags and every setting kind, plus an hours-less
timestamp (`VTT_SAMPLE`). Each yields zero diagnostics on both layers.

## Gates

```
cargo test -p themoretheless-tokenizer-srt          # 45 unit tests + 2 doctests
cargo clippy -p themoretheless-tokenizer-srt --all-targets
cargo fmt -p themoretheless-tokenizer-srt
```
