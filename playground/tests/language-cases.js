/**
 * Fixtures for every registered language: valid, incomplete, comments, unicode.
 * Used by playground WASM bridge tests.
 */

/** @typedef {{ name: string, source: string, mode?: string, layer?: string, expectValid?: boolean, expectKinds?: string[], minTokens?: number, maxDiagnostics?: number }} Case */

/**
 * iCalendar content lines only validate inside a complete VCALENDAR wrapper,
 * so the ics cases below share one that carries VERSION and PRODID.
 */
const inCalendar = (body) =>
  `BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tokenizer//playground//EN\r\n${body}END:VCALENDAR\r\n`

/** @type {Record<string, { modes: string[], cases: Case[] }>} */
export const LANGUAGE_CASES = {
  json: {
    modes: ['strict', 'jsonc'],
    cases: [
      { name: 'empty-object', source: '{}', mode: 'strict', expectValid: true, expectKinds: ['punctuation'] },
      { name: 'unicode-property', source: '{"city":"Тбилиси"}', mode: 'strict', expectValid: true, expectKinds: ['property', 'string'] },
      { name: 'array-numbers', source: '[1,2,3]', mode: 'strict', expectValid: true, expectKinds: ['number'] },
      { name: 'leading-zero-invalid', source: '{"n":01}', mode: 'strict', expectValid: false },
      { name: 'jsonc-comment', source: '{// c\n"a":1}', mode: 'jsonc', expectValid: true, expectKinds: ['comment'] },
      { name: 'jsonc-allows-trailing-comma', source: '{// c\n"a":1,}', mode: 'jsonc', expectValid: true, expectKinds: ['comment'] },
      { name: 'trailing-comma-rejected-in-strict', source: '{"a":1,}', mode: 'strict', expectValid: false },
      { name: 'jsonc-rejected-in-strict', source: '{// c\n"a":1}', mode: 'strict', expectValid: false },
      { name: 'incomplete-object', source: '{"a":', mode: 'strict', expectValid: false, minTokens: 1 },
      { name: 'empty-source', source: '', mode: 'strict', minTokens: 0 },
    ],
  },
  json5: {
    modes: ['default'],
    cases: [
      { name: 'bare-word-key', source: '{a:1}', layer: 'syntax', expectValid: true, expectKinds: ['unquoted-key'] },
      { name: 'bare-word-key-is-a-property', source: '{a:1}', expectValid: true, expectKinds: ['property'] },
      { name: 'single-quoted', source: "{'a':'b'}", layer: 'syntax', expectValid: true, expectKinds: ['single-quoted-key', 'single-quoted-string'] },
      { name: 'hex-and-infinity', source: '{h:0xFF,i:Infinity}', expectValid: true, expectKinds: ['hex-number', 'infinity'] },
      { name: 'trailing-comma', source: '[1,2,3,]', expectValid: true, expectKinds: ['trailing-comma'] },
      { name: 'comment', source: '{// c\n"a":1}', expectValid: true, expectKinds: ['line-comment'] },
      { name: 'signed-nan-invalid', source: '{x:-NaN}', expectValid: false },
      { name: 'incomplete-object', source: '{a:', expectValid: false, minTokens: 2 },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  jsonl: {
    modes: ['default'],
    cases: [
      { name: 'two-records', source: '{"a":1}\n{"b":2}', expectValid: true, expectKinds: ['record-break'] },
      { name: 'scalar-per-line', source: '1\n2\n3', expectValid: true, expectKinds: ['number'] },
      { name: 'blank-lines', source: '{"a":1}\n\n{"b":2}\n', expectValid: true },
      { name: 'trailing-comma-rejected', source: '{"a":1,}', expectValid: false },
      { name: 'comment-rejected', source: '{// c\n"a":1}', expectValid: false },
      { name: 'broken-record', source: '{"a":1}\n{"b":', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  csv: {
    modes: ['default'],
    cases: [
      { name: 'header-and-row', source: 'a,b\n1,2\n', expectValid: true, expectKinds: ['header-field', 'delimiter', 'record-break'] },
      { name: 'quoted-comma', source: 'a,b\n1,"x,y"\n', expectValid: true, expectKinds: ['quote', 'quoted-field'] },
      { name: 'escaped-quote', source: 'a,b\n1,"x""y"\n', expectValid: true, expectKinds: ['escaped-quote'] },
      { name: 'typed-fields', source: 'a,b,c\n1,2.5,true\n', expectValid: true, expectKinds: ['integer-field', 'decimal-field', 'boolean-field'] },
      { name: 'embedded-newline', source: 'a,b\n"x\ny",2\n', expectValid: true },
      { name: 'unclosed-quote', source: 'a,"b', expectValid: false },
      { name: 'ragged-row', source: 'a,b\nc\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  tsv: {
    modes: ['default'],
    cases: [
      { name: 'header-and-row', source: 'a\tb\n1\t2\n', expectValid: true, expectKinds: ['header-field', 'delimiter'] },
      { name: 'quotes-are-text', source: 'a\tb\n"q"\t2\n', expectValid: true, expectKinds: ['field'] },
      { name: 'commas-are-text', source: 'a\tb\nx,y\t2\n', expectValid: true, expectKinds: ['field'] },
      { name: 'ragged-row', source: 'a\tb\nc\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  url: {
    modes: ['default'],
    cases: [
      { name: 'https', source: 'https://example.com/path?q=1#f', expectKinds: ['u-scheme', 'u-host'] },
      { name: 'path-only', source: '/api/v1/items', expectKinds: ['u-path'] },
      { name: 'incomplete-scheme', source: 'https:', minTokens: 1 },
      { name: 'whitespace-diag', source: 'https://ex.com/a b', expectValid: false },
      { name: 'port-range', source: 'https://h:99999/', expectValid: false },
      { name: 'empty-port-flagged-not-typed', source: 'https://host:', expectValid: false, expectKinds: ['u-sep', 'u-host'], maxDiagnostics: 1 },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  xml: {
    modes: ['default'],
    cases: [
      { name: 'root', source: '<root a="1"/>', expectKinds: ['tag'] },
      { name: 'nested', source: '<a><b>x</b></a>', expectKinds: ['tag', 'text'] },
      { name: 'comment', source: '<!-- c --><e/>', expectKinds: ['comment'] },
      { name: 'unclosed', source: '<open>', expectValid: false, minTokens: 1 },
      // XML has no void elements and no case folding, so both stay errors where
      // HTML accepts them.
      { name: 'html-void-name', source: '<div><br></div>', expectValid: false },
      { name: 'case-mismatch', source: '<A></a>', expectValid: false },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  html: {
    modes: ['default'],
    cases: [
      { name: 'div', source: '<div class="box">hi</div>', expectKinds: ['tag', 'text'] },
      { name: 'void-self-closing', source: '<br/>', expectKinds: ['tag'] },
      // The shapes HTML allows and a strict XML parser rejects.
      { name: 'void-unclosed', source: '<p>a<br>b</p>', expectValid: true },
      {
        name: 'head-void-elements',
        source:
          '<html><head><meta charset="utf-8"><link rel="stylesheet" href="a.css"></head><body>x</body></html>',
        expectValid: true,
      },
      { name: 'omitted-end-tags', source: '<ul><li>a<li>b</ul><p>one<p>two', expectValid: true },
      {
        name: 'table-cells-omitted',
        source: '<table><tr><td>1<td>2</td></tr></table>',
        expectValid: true,
      },
      { name: 'raw-text-script', source: '<script>if (a < b) f();</script>', expectValid: true },
      { name: 'case-insensitive-tags', source: '<DIV>x</div>', expectValid: true },
      { name: 'comment', source: '<!--x--><p></p>', expectKinds: ['comment'] },
      { name: 'incomplete', source: '<div', expectValid: false, minTokens: 1 },
      { name: 'close-without-open', source: '<div>a</span></div>', expectValid: false },
      { name: 'stray-close', source: '</p>', expectValid: false },
      // Not a quirk: per spec the body ends at the first `</script`, so the
      // trailing `";` and the second close tag really are markup errors.
      {
        name: 'script-ends-in-string',
        source: '<script>var s = "</script>";</script>',
        expectValid: false,
      },
      { name: 'unicode', source: '<span>Москва</span>', expectKinds: ['text'] },
    ],
  },
  css: {
    modes: ['default'],
    cases: [
      { name: 'rule', source: 'body { color: red; }', expectKinds: ['type-selector', 'property'] },
      { name: 'comment', source: '/* c */ .x{}', expectKinds: ['comment', 'class-selector'] },
      { name: 'string', source: 'a { content: "hi"; }', expectKinds: ['string', 'property'] },
      { name: 'hash-disambiguation', source: 'a#main { color: #fff; }', expectKinds: ['id-selector', 'color'] },
      { name: 'at-rule-and-unit', source: '@media (max-width: 600px) { a { margin: 2rem; } }', expectKinds: ['at-rule', 'unit'] },
      { name: 'custom-property', source: ':root { --brand: #0a6; }\na { color: var(--brand); }', expectKinds: ['variable'] },
      { name: 'important', source: 'a { color: red !important; }', expectKinds: ['important'] },
      { name: 'unclosed-block-recovers', source: 'a { color: red', expectValid: false, minTokens: 1 },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  yaml: {
    modes: ['default'],
    cases: [
      { name: 'mapping', source: 'name: Denis\nage: 1\n', minTokens: 1 },
      { name: 'comment', source: '# hi\nkey: value\n', expectKinds: ['comment'] },
      { name: 'bool', source: 'ok: true\n', expectKinds: ['value-indicator', 'plain-scalar'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  toml: {
    modes: ['default'],
    cases: [
      { name: 'key-value', source: 'name = "tok"\n', expectKinds: ['bare-key', 'basic-string'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'bool', source: 'flag = true\n', expectKinds: ['bare-key', 'true'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  markdown: {
    modes: ['default'],
    cases: [
      { name: 'heading', source: '# Title\n\npara\n', expectKinds: ['heading-marker', 'heading-text'] },
      { name: 'fence', source: '```js\ncode\n```\n', expectKinds: ['code-fence-marker', 'fence-info', 'code-block-line'] },
      { name: 'inline-code', source: 'use `x` here\n', expectKinds: ['code-span'] },
      { name: 'emphasis', source: 'Some **bold** and *em* text\n', expectKinds: ['strong', 'emphasis'] },
      { name: 'list-and-link', source: '- [link](https://example.com)\n', expectKinds: ['list-marker', 'link-text', 'link-destination'] },
      { name: 'blockquote-table', source: '> quoted\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n', expectKinds: ['blockquote-marker', 'table-delimiter', 'table-cell'] },
      { name: 'unclosed-fence-recovers', source: '```js\nno end\n', expectValid: false, minTokens: 1 },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  logfmt: {
    modes: ['default'],
    cases: [
      { name: 'record', source: 'ts=2026-01-01T00:00:00Z level=info status=200\n', expectValid: true, expectKinds: ['key', 'bare-value', 'integer-value', 'record-break'] },
      { name: 'typed-values', source: 'ok=true ratio=1.5 count=7 err=null\n', expectValid: true, expectKinds: ['boolean-value', 'float-value', 'integer-value', 'null-value'] },
      { name: 'quoted-with-escape', source: 'msg="GET /api\\"v1"\n', expectValid: true, expectKinds: ['quoted-value', 'escaped-char'] },
      { name: 'flag-keys', source: 'cache warm\n', expectValid: true, expectKinds: ['flag-key'] },
      { name: 'empty-value', source: 'msg=\n', expectValid: true, expectKinds: ['empty-value'] },
      { name: 'syntax-layer-stays-untyped', source: 'count=7 ok=true\n', layer: 'syntax', expectValid: true, expectKinds: ['bare-value'] },
      { name: 'missing-key', source: 'a=1 =oops\n', expectValid: false },
      { name: 'unterminated-value', source: 'msg="open\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  ini: {
    modes: ['default'],
    cases: [
      { name: 'section-and-keys', source: '[owner]\nname = Grace\nbirth = 1906-12-09\n', expectValid: true, expectKinds: ['section-marker', 'section-name', 'key', 'separator', 'value'] },
      { name: 'typed-values', source: '[s]\nport = 8080\nsecure = true\n', expectValid: true, expectKinds: ['integer-value', 'boolean-value'] },
      { name: 'syntax-layer-stays-untyped', source: '[s]\nport = 8080\n', layer: 'syntax', expectValid: true, expectKinds: ['value'] },
      { name: 'quoted-value-and-comment', source: '[s]\nbanner = "welcome, all" ; note\n', expectValid: true, expectKinds: ['quote', 'quoted-value', 'comment'] },
      { name: 'indented-continuation', source: '[s]\nk = one\n  two\n', expectValid: true, expectKinds: ['line-continuation'] },
      { name: 'unterminated-section', source: '[oops\n', expectValid: false, minTokens: 1 },
      { name: 'key-before-first-section-warns', source: 'a = 1\n[s]\nc = 3\n', expectValid: true, maxDiagnostics: 1 },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  properties: {
    modes: ['default'],
    cases: [
      { name: 'flat-keys', source: 'app.name = Tokenizer\napp.version = 42\n', expectValid: true, expectKinds: ['key', 'separator', 'value', 'integer-value'] },
      { name: 'escaped-separator-and-value', source: 'name\\:full = Grace\\tHopper\n', expectValid: true, expectKinds: ['escape-sequence', 'key'] },
      { name: 'backslash-continuation', source: 'k = one\\\n  two\n', expectValid: true, expectKinds: ['line-continuation'] },
      { name: 'comment-introducers', source: '# build\n! also\nk = v\n', expectValid: true, expectKinds: ['comment'] },
      { name: 'colon-separator', source: 'ratio: 1.5\n', expectValid: true, expectKinds: ['separator', 'decimal-value'] },
      { name: 'section-header-is-a-key-here', source: '[a]\nk = v\n', expectValid: true, expectKinds: ['key', 'value'] },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  hcl: {
    modes: ['default'],
    cases: [
      { name: 'labeled-block-and-attribute', source: 'service "api" {\n  port = 8080\n}\n', expectValid: true, expectKinds: ['block-type', 'block-label', 'attribute-name', 'number'] },
      { name: 'typed-literals', source: 'a = true\nb = null\nc = -1.5e3\n', expectValid: true, expectKinds: ['boolean', 'null', 'number'] },
      { name: 'interpolation', source: 'greeting = "hello ${local.name}!"\n', expectValid: true, expectKinds: ['interpolation', 'string'] },
      { name: 'heredoc', source: 'note = <<EOT\n  cluster online\n  EOT\n', expectValid: true, expectKinds: ['heredoc-open', 'heredoc-body', 'heredoc-close'] },
      { name: 'both-comment-shapes', source: '# line\n/* block */\nk = v\n', expectValid: true, expectKinds: ['line-comment', 'block-comment'] },
      { name: 'escape-inside-string', source: 'q = "said \\"hi\\"\\n"\n', expectValid: true, expectKinds: ['escape'] },
      { name: 'syntax-layer-is-unlabeled', source: 'service "api" {\n  port = 8080\n}\n', layer: 'syntax', expectValid: true, expectKinds: ['identifier'] },
      { name: 'unclosed-brace', source: 'service "api" {\n  port = 8080\n', expectValid: false },
      { name: 'unterminated-string', source: 'k = "open\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  edn: {
    modes: ['default'],
    cases: [
      { name: 'map-with-namespaced-key', source: '{:a 1, :user/id "two"}\n', expectValid: true, expectKinds: ['map-open', 'map-close', 'keyword', 'namespaced-keyword', 'integer', 'string'] },
      { name: 'symbols-and-lists', source: '(foo bar 1 2)\n', expectValid: true, expectKinds: ['list-open', 'list-close', 'symbol', 'integer'] },
      { name: 'set-and-vector', source: '#{1 2} [3 4]\n', expectValid: true, expectKinds: ['set-open', 'vector-open', 'vector-close', 'map-close'] },
      { name: 'reader-tags', source: '#inst "2026-01-01T00:00:00Z"\n#uuid "f81d4fae-7dec-11d0-a765-00a0c91e6bf6"\n', expectValid: true, expectKinds: ['instant-tag', 'instant-value', 'uuid-tag', 'uuid-value'] },
      { name: 'radix-ratio-bigint', source: '[16rFF 3/4 2N]\n', expectValid: true, expectKinds: ['radix-integer', 'ratio', 'bigint'] },
      { name: 'discard-reads-the-whole-form', source: '[1 #_(hidden 2) 3]\n', expectValid: true, expectKinds: ['discard', 'discarded-form'] },
      { name: 'characters-and-namespaces', source: '[\\newline foo/bar]\n', expectValid: true, expectKinds: ['character', 'namespaced-symbol'] },
      { name: 'tagged-value', source: '#my/tag {:k 1}\n', expectValid: true, expectKinds: ['tag', 'tagged-value'] },
      { name: 'nil-and-specials', source: '[nil ##Inf -0.5]\n', expectValid: true, expectKinds: ['nil-literal', 'special-number', 'float'] },
      { name: 'unclosed-collection', source: '{:a 1\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  srt: {
    modes: ['default'],
    cases: [
      { name: 'one-cue', source: '1\n00:00:01,000 --> 00:00:04,500\nHello there\n', expectValid: true, expectKinds: ['cue-index', 'cue-start', 'cue-end', 'timing-arrow', 'cue-text'] },
      { name: 'continuation-line', source: '1\n00:00:01,000 --> 00:00:04,000\nline one\nline two\n', expectValid: true, expectKinds: ['cue-text', 'cue-text-continuation'] },
      { name: 'bom-and-breaks', source: '\uFEFF1\n00:00:01,000 --> 00:00:02,000\nHi\n', expectValid: true, expectKinds: ['bom', 'record-break'] },
      { name: 'syntax-layer-sees-timestamp-fields', source: '1\n00:00:01,500 --> 00:00:02,000\nHi\n', layer: 'syntax', expectValid: true, expectKinds: ['time-hour', 'millisecond', 'timing-arrow'] },
      { name: 'start-after-end-is-flagged', source: '1\n00:00:05,000 --> 00:00:01,000\nHi\n', maxDiagnostics: 1 },
      { name: 'non-monotonic-index-is-flagged', source: '2\n00:00:01,000 --> 00:00:02,000\na\n\n1\n00:00:03,000 --> 00:00:04,000\nb\n', maxDiagnostics: 1 },
      { name: 'missing-arrow', source: '1\n00:00:01,000 00:00:02,000\nHi\n', expectValid: false },
      { name: 'dot-separator-is-subrip-wrong', source: '1\n00:00:01.000 --> 00:00:02.000\nHi\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  vtt: {
    modes: ['default'],
    cases: [
      { name: 'signature-and-cue', source: 'WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nHello there\n', expectValid: true, expectKinds: ['signature', 'cue-start', 'cue-end', 'timing-arrow', 'cue-text'] },
      { name: 'opaque-cue-id', source: 'WEBVTT\n\nintro\n00:00:01.000 --> 00:00:02.000\nHi\n', expectValid: true, expectKinds: ['cue-id'] },
      { name: 'cue-settings', source: 'WEBVTT\n\n00:00:01.000 --> 00:00:02.000 line:50% align:center\nHi\n', expectValid: true, expectKinds: ['setting-name', 'setting-separator', 'line-value', 'alignment-value'] },
      { name: 'inline-markup', source: 'WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Roger>Hi <i>there</i>\n', expectValid: true, expectKinds: ['voice-tag', 'markup-value', 'emphasis-tag', 'closing-tag', 'cue-text'] },
      { name: 'note-block', source: 'WEBVTT\n\nNOTE a remark\n', expectValid: true, expectKinds: ['block-marker', 'comment'] },
      { name: 'style-block', source: 'WEBVTT\n\nSTYLE\n::cue { color: red }\n', expectValid: true, expectKinds: ['block-marker', 'style-rule'] },
      { name: 'region-block', source: 'WEBVTT\n\nREGION\nid:bottom\nwidth:100%\n', expectValid: true, expectKinds: ['block-marker', 'region-property', 'region-separator', 'region-value'] },
      { name: 'missing-signature', source: '00:00:01.000 --> 00:00:02.000\nHi\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  ics: {
    modes: ['default'],
    cases: [
      { name: 'wrapper-and-property', source: inCalendar('BEGIN:VEVENT\r\nUID:1\r\nDTSTAMP:20260101T090000Z\r\nEND:VEVENT\r\n'), expectValid: true, expectKinds: ['structure-marker', 'component-name', 'property-name', 'value-delimiter', 'text-value'] },
      { name: 'date-times-and-uri', source: inCalendar('BEGIN:VEVENT\r\nUID:1\r\nDTSTAMP:20260101T090000Z\r\nDTSTART:20260101T090000Z\r\nURL:https://example.com\r\nEND:VEVENT\r\n'), expectValid: true, expectKinds: ['date-time-value', 'uri-value'] },
      { name: 'parameters', source: inCalendar('ATTACH;FMTTYPE=text/plain;FILENAME="a b.txt":x\r\n'), expectValid: true, expectKinds: ['parameter-delimiter', 'parameter-name', 'parameter-assignment', 'bare-param-value', 'quoted-param-value'] },
      { name: 'folded-line', source: inCalendar('DESCRIPTION:first\r\n  second\r\n'), expectValid: true, expectKinds: ['fold-marker', 'text-value'] },
      { name: 'escaped-text', source: inCalendar('SUMMARY:Launch party\\, everyone\r\n'), expectValid: true, expectKinds: ['escaped-char', 'text-value'] },
      { name: 'durations-and-recurrence', source: inCalendar('DURATION:PT15M\r\nRRULE:FREQ=DAILY\r\n'), expectValid: true, expectKinds: ['duration-value', 'recurrence-value'] },
      { name: 'unclosed-component', source: 'BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:1\r\n', expectValid: false },
      { name: 'content-line-before-begin', source: 'VERSION:2.0\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n', expectValid: false },
      { name: 'empty-source', source: '', minTokens: 0 },
    ],
  },
  sql: {
    modes: ['default'],
    cases: [
      { name: 'select', source: 'SELECT id FROM users WHERE x = 1;', expectKinds: ['keyword'] },
      { name: 'select-lower', source: 'select id from users;', expectKinds: ['keyword'] },
      { name: 'comment', source: '-- c\nSELECT 1;', expectKinds: ['comment'] },
      { name: 'string', source: "SELECT 'hi';", expectKinds: ['string'] },
      { name: 'incomplete', source: 'SELECT FROM', minTokens: 1 },
    ],
  },
  mongo: {
    modes: ['default'],
    cases: [
      { name: 'find', source: 'db.users.find({ a: 1 })', minTokens: 1 },
      { name: 'comment', source: '// c\nfind({})', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  bash: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'if [ "$x" = 1 ]; then echo hi; fi\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# note\necho ok\n', expectKinds: ['comment'] },
      { name: 'string', source: 'echo "hello"\n', expectKinds: ['string'] },
      { name: 'incomplete', source: 'if then', minTokens: 1 },
    ],
  },
  powershell: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'if ($true) { Write-Host "x" }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nGet-Item .\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  javascript: {
    modes: ['default'],
    cases: [
      { name: 'function', source: 'function add(a, b) { return a + b; }', expectKinds: ['keyword', 'function'] },
      { name: 'const', source: 'const x = 42;\n', expectKinds: ['keyword', 'number'] },
      { name: 'comment', source: '// c\nlet y = 1;', expectKinds: ['comment'] },
      { name: 'string', source: 'const s = "hi";', expectKinds: ['string'] },
      { name: 'incomplete', source: 'function (', minTokens: 1 },
      { name: 'unicode', source: 'const city = "Тбилиси";', expectKinds: ['string'] },
    ],
  },
  typescript: {
    modes: ['default'],
    cases: [
      { name: 'interface', source: 'interface U { id: number }\n', expectKinds: ['keyword'] },
      { name: 'function', source: 'function f(x: string): number { return 1; }', expectKinds: ['keyword'] },
      { name: 'type-keyword', source: 'type Id = string;\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '/* c */ const a = 1;', expectKinds: ['comment'] },
    ],
  },
  python: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def greet(name):\n    return f"hi {name}"\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hello"\n', expectKinds: ['string'] },
      { name: 'class', source: 'class A:\n    pass\n', expectKinds: ['keyword'] },
      { name: 'incomplete', source: 'def f(', minTokens: 1 },
      { name: 'unicode', source: 'msg = "Привет"\n', expectKinds: ['string'] },
    ],
  },
  java: {
    modes: ['default'],
    cases: [
      { name: 'class', source: 'class A { int x = 1; }', expectKinds: ['keyword'] },
      { name: 'method', source: 'public int f() { return 0; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nint a;', expectKinds: ['comment'] },
      { name: 'string', source: 'String s = "x";', expectKinds: ['string'] },
    ],
  },
  csharp: {
    modes: ['default'],
    cases: [
      { name: 'class', source: 'class A { public int X { get; set; } }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar x = 1;', expectKinds: ['comment'] },
      { name: 'string', source: 'var s = "hi";', expectKinds: ['string'] },
    ],
  },
  go: {
    modes: ['default'],
    cases: [
      { name: 'func', source: 'func main() { x := 1 }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar x int', expectKinds: ['comment'] },
      { name: 'string', source: 's := "hi"', expectKinds: ['string'] },
    ],
  },
  php: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'function f($x) { return $x; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\n$x = 1;', expectKinds: ['comment'] },
      { name: 'string', source: '$s = "hi";', expectKinds: ['string'] },
    ],
  },
  ruby: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def f(x)\n  x\nend\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
    ],
  },
  c: {
    modes: ['default'],
    cases: [
      { name: 'main', source: 'int main(void) { return 0; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '/* c */ int x;', expectKinds: ['comment'] },
      { name: 'string', source: 'char *s = "hi";', expectKinds: ['string'] },
      { name: 'incomplete', source: 'int main(', minTokens: 1 },
    ],
  },
  cpp: {
    modes: ['default'],
    cases: [
      { name: 'class', source: 'class A { public: int x; };', expectKinds: ['keyword'] },
      { name: 'template-ish', source: 'int f() { return 1; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nint x;', expectKinds: ['comment'] },
    ],
  },
  rust: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'fn main() { let x = 1; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nlet y = 2;', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi";', expectKinds: ['string'] },
      { name: 'incomplete', source: 'fn main(', minTokens: 1 },
    ],
  },
  kotlin: {
    modes: ['default'],
    cases: [
      { name: 'fun', source: 'fun main() { val x = 1 }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nval y = 2', expectKinds: ['comment'] },
      { name: 'string', source: 'val s = "hi"', expectKinds: ['string'] },
    ],
  },
  swift: {
    modes: ['default'],
    cases: [
      { name: 'func', source: 'func f() -> Int { return 1 }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nlet x = 1', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi"', expectKinds: ['string'] },
    ],
  },
  dart: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'void main() { var x = 1; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar y = 2;', expectKinds: ['comment'] },
      { name: 'string', source: 'var s = "hi";', expectKinds: ['string'] },
    ],
  },
  r: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'f <- function(x) { x + 1 }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx <- 1\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  visualbasic: {
    modes: ['default'],
    cases: [
      { name: 'sub', source: 'Sub Main()\nEnd Sub\n', expectKinds: ['keyword'] },
      { name: 'comment', source: "' c\nDim x As Integer\n", expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  fortran: {
    modes: ['default'],
    cases: [
      { name: 'program', source: 'program hello\nend program hello\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '! c\ninteger :: x\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  matlab: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'if x > 0\n  y = 1;\nend\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '% c\nx = 1;\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  delphi: {
    modes: ['default'],
    cases: [
      { name: 'begin', source: 'begin\n  x := 1;\nend.\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar x: Integer;\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  scala: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def f(x: Int): Int = x + 1\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nval y = 2\n', expectKinds: ['comment'] },
      { name: 'string', source: 'val s = "hi"\n', expectKinds: ['string'] },
    ],
  },
  lua: {
    modes: ['default'],
    cases: [
      { name: 'function', source: 'function f(x) return x end\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '-- c\nlocal x = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
    ],
  },
  perl: {
    modes: ['default'],
    cases: [
      { name: 'sub', source: 'sub f { my $x = shift; return $x; }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nmy $y = 1;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'my $s = "hi";\n', expectKinds: ['string'] },
    ],
  },
  objectivec: {
    modes: ['default'],
    cases: [
      { name: 'interface', source: '@interface A : NSObject\n@end\n', expectKinds: ['class', 'identifier'] },
      { name: 'comment', source: '// c\nint x;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'NSString *s = @"hi";\n', minTokens: 1 },
    ],
  },
  julia: {
    modes: ['default'],
    cases: [
      { name: 'function', source: 'function f(x)\n  x + 1\nend\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
    ],
  },
  assembly: {
    modes: ['default'],
    cases: [
      { name: 'mov', source: 'mov eax, 1\nret\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '; c\nnop\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  groovy: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def f(x) { return x + 1 }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\ndef x = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  haskell: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'f x = x + 1\n', minTokens: 1 },
      { name: 'comment', source: '-- c\nf = 1\n', expectKinds: ['comment'] },
      { name: 'where', source: 'f = g where g = 1\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  elixir: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def f(x), do: x + 1\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  erlang: {
    modes: ['default'],
    cases: [
      { name: 'fun', source: 'f(X) -> X + 1.\n', minTokens: 1 },
      { name: 'comment', source: '% c\nX = 1.\n', expectKinds: ['comment'] },
      { name: 'case', source: 'case X of 1 -> ok end.\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  clojure: {
    modes: ['default'],
    cases: [
      { name: 'defn', source: '(defn f [x] (+ x 1))\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '; c\n(def x 1)\n', expectKinds: ['comment'] },
      { name: 'string', source: '(def s "hi")\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  fsharp: {
    modes: ['default'],
    cases: [
      { name: 'let', source: 'let f x = x + 1\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nlet x = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi"\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  ocaml: {
    modes: ['default'],
    cases: [
      { name: 'let', source: 'let f x = x + 1\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '(* c *)\nlet x = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi"\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  lisp: {
    modes: ['default'],
    cases: [
      { name: 'defun', source: '(defun f (x) (+ x 1))\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '; c\n(defvar x 1)\n', expectKinds: ['comment'] },
      { name: 'string', source: '(setq s "hi")\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  scheme: {
    modes: ['default'],
    cases: [
      { name: 'define', source: '(define (f x) (+ x 1))\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '; c\n(define x 1)\n', expectKinds: ['comment'] },
      { name: 'string', source: '(define s "hi")\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  solidity: {
    modes: ['default'],
    cases: [
      { name: 'contract', source: 'contract C { function f() public {} }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nuint x;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'string s = "hi";\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  zig: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'pub fn main() void {}\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nconst x = 1;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'const s = "hi";\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  nim: {
    modes: ['default'],
    cases: [
      { name: 'proc', source: 'proc f(x: int): int = x + 1\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nlet x = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi"\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  dlang: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'int f(int x) { return x + 1; }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nint x = 1;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'string s = "hi";\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  cobol: {
    modes: ['default'],
    cases: [
      { name: 'move', source: 'MOVE 1 TO X.\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '*> c\nDISPLAY Y.\n', expectKinds: ['comment'] },
      { name: 'if', source: 'IF X = 1 THEN DISPLAY Y END-IF.\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  ada: {
    modes: ['default'],
    cases: [
      { name: 'procedure', source: 'procedure Main is begin null; end Main;\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '-- c\nX : Integer := 1;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'S : String := "hi";\n', expectKinds: ['string'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  prolog: {
    modes: ['default'],
    cases: [
      { name: 'rule', source: 'parent(X, Y) :- mother(X, Y).\n', minTokens: 1 },
      { name: 'comment', source: '% c\ntrue.\n', expectKinds: ['comment'] },
      { name: 'fail', source: 'fail.\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  abap: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'IF x = 1.\n  WRITE y.\nENDIF.\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '" c\nDATA x TYPE i.\n', expectKinds: ['comment'] },
      { name: 'data', source: 'DATA lv TYPE i.\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  vhdl: {
    modes: ['default'],
    cases: [
      { name: 'entity', source: 'entity E is end entity;\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '-- c\nsignal x : bit;\n', expectKinds: ['comment'] },
      { name: 'process', source: 'process begin wait; end process;\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  verilog: {
    modes: ['default'],
    cases: [
      { name: 'module', source: 'module m; endmodule\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nwire x;\n', expectKinds: ['comment'] },
      { name: 'always', source: 'always @(*) begin end\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  graphql: {
    modes: ['default'],
    cases: [
      { name: 'query', source: 'query Q { user { id name } }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\ntype User { id: ID }\n', expectKinds: ['comment'] },
      { name: 'type', source: 'type User { id: ID! }\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
}

/** Every language id that must be present in the WASM build. */
export const ALL_LANGUAGE_IDS = Object.keys(LANGUAGE_CASES)

export function defaultMode(language) {
  const entry = LANGUAGE_CASES[language]
  if (!entry) return 'default'
  if (language === 'json') return 'strict'
  return entry.modes[0] ?? 'default'
}
