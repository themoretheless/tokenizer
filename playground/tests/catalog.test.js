import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { LANGUAGES } from '../src/languages.js'
import { LANGUAGE_CASES, defaultMode } from './language-cases.js'
import { KIND_COLORS } from '../src/kinds.js'
import { GENERIC_KINDS, PRESETS, depthOf, depthLabel } from '../src/catalog.js'

const require = createRequire(import.meta.url)
const wasm = require('../.wasm-test/tokenizer_wasm.js')

const catalog = JSON.parse(wasm.wasm_catalog())
const byId = Object.fromEntries(catalog.engines.map((engine) => [engine.id, engine]))

test('js catalog and rust registry cover the same engine ids', () => {
  const js = LANGUAGES.map((lang) => lang.id).sort()
  const rust = catalog.engines.map((engine) => engine.id).sort()
  assert.deepEqual(js, rust, 'playground catalog and registry must list the same ids')
  assert.equal(catalog.count, rust.length)
})

test('preset sizes match the curated rust tables', () => {
  const counts = Object.fromEntries(PRESETS.map((item) => [
    item.id,
    catalog.engines.filter((engine) => engine.presets.includes(item.id)).length,
  ]))
  assert.equal(counts.top20, 20)
  assert.equal(counts.next20, 20)
  assert.equal(counts.formats, 20)
})

test('family axis overrides the naive grouping rule', () => {
  // Query languages sit in the playground's "query" group but are not formats.
  assert.equal(byId.sql.family, 'language')
  assert.equal(byId.graphql.family, 'language')
  assert.equal(byId.mongo.family, 'language')
  for (const id of ['json', 'json5', 'jsonl', 'yaml', 'toml', 'url', 'xml', 'html', 'css', 'markdown', 'csv', 'tsv', 'logfmt', 'ini', 'properties', 'hcl', 'edn', 'srt', 'vtt', 'ics']) {
    assert.equal(byId[id].family, 'format', id)
  }
})

test('playground modes are rust dialect ids', () => {
  for (const lang of LANGUAGES) {
    const dialects = byId[lang.id].dialects.map((dialect) => dialect.id)
    assert.deepEqual(
      [...lang.modes].sort(),
      [...dialects].sort(),
      `${lang.id}: modes ${lang.modes} vs dialects ${dialects}`,
    )
  }
})

test('capability labels separate the three engine tiers', () => {
  assert.equal(depthLabel(byId.json), 'document engine')
  assert.equal(depthLabel(byId.python), 'recovering parser')
  assert.equal(depthLabel(byId.url), 'structure validator')
})

test('only the format family advertises validation', () => {
  // `validate` claims a grammar that rejects input its own spec forbids. The
  // shared fullkit pipeline behind the wave languages can only prove bracket
  // balance and closed literals, so it stays tolerant of constructs it does not
  // model. Pinning the partition stops a new engine inheriting the badge from
  // the descriptor.
  const claimers = catalog.engines
    .filter((engine) => engine.capabilities.includes('validate'))
    .map((engine) => engine.id)
    .sort()
  const formats = catalog.engines
    .filter((engine) => engine.family === 'format')
    .map((engine) => engine.id)
    .sort()
  assert.deepEqual(claimers, formats, 'validate must be claimed by exactly the formats')
  assert.equal(claimers.length, 20)
  for (const engine of catalog.engines.filter((item) => item.family === 'language')) {
    assert.equal(engine.capabilities.includes('validate'), false, engine.id)
    assert.deepEqual(engine.capabilities, ['lex', 'parse', 'semantic'], engine.id)
  }
})

test('an engine that claims validation is quiet on valid input', () => {
  // The badge is only worth having if the matrix agrees: every case marked
  // expectValid for a validating engine must come back with no diagnostics.
  const checked = []
  for (const [id, entry] of Object.entries(LANGUAGE_CASES)) {
    const engine = byId[id]
    if (!engine?.capabilities.includes('validate')) continue
    for (const item of entry.cases) {
      if (item.expectValid !== true) continue
      const dialect = item.mode ?? defaultMode(id)
      const payload = JSON.parse(wasm.tokenize(id, item.source, dialect, item.layer ?? 'semantic'))
      assert.equal(
        payload.valid,
        true,
        `${id}/${item.name}: engine claims validate but flagged ${payload.diagnostics.map((d) => d.code).join(', ')}`,
      )
      checked.push(`${id}/${item.name}`)
    }
  }
  // Measured: the 20 validating engines carry 72 expectValid cases between them.
  assert.ok(checked.length >= 70, `only ${checked.length} valid-input cases covered the badge`)
})

test('a picker sample is a valid document, and the language fixtures track it', () => {
  // Nothing in the matrix says "this document is well-formed and the engine
  // agrees" for the wave languages, so fullkit flagged valid go, java, sql and
  // 22 more on their own samples. This is the same claim
  // `every_engine_is_quiet_on_its_valid_fixture` makes in Rust, where the Node
  // suite does not run: that gate reads `tests/fixtures/languages/<id>.txt`,
  // which is exported from these samples by
  // `playground/scripts/export-language-fixtures.mjs`. Checking the export
  // against the sample here is what keeps the two corpora from drifting apart.
  const quiet = []
  for (const lang of LANGUAGES) {
    const engine = byId[lang.id]
    for (const layer of ['syntax', 'semantic']) {
      const payload = JSON.parse(wasm.tokenize(lang.id, lang.sample, engine.defaultDialect, layer))
      assert.equal(
        payload.valid,
        true,
        `${lang.id}/${layer}: valid sample flagged by ${payload.diagnostics.map((d) => d.code).join(', ')}`,
      )
    }
    quiet.push(lang.id)
    if (engine.family !== 'language') continue
    const fixture = readFileSync(
      new URL(`../../tests/fixtures/languages/${lang.id}.txt`, import.meta.url),
      'utf8',
    )
    assert.equal(
      fixture,
      lang.sample,
      `${lang.id}: fixture drifted from the picker sample — run npm run export:language-fixtures`,
    )
  }
  assert.equal(quiet.length, 69)
})

test('format vocabulary depth is measurable per engine', () => {
  const tokensOf = (id) => {
    const engine = byId[id]
    const payload = JSON.parse(
      wasm.tokenize(id, LANGUAGES.find((lang) => lang.id === id).sample, engine.defaultDialect, 'semantic'),
    )
    return payload.tokens
  }
  // url maps every structural part to its own kind.
  assert.equal(depthOf(tokensOf('url')).specific, 9)
  // toml and yaml map their own lexical categories through the host adapter.
  // Floors are the measured counts over the playground samples, which are one
  // or two lines long; a collapse back through core_kind drops these to 0.
  assert.deepEqual(depthOf(tokensOf('toml')).specificKinds, ['bare-key', 'basic-string', 'equals', 'newline', 'true'])
  assert.deepEqual(depthOf(tokensOf('yaml')).specificKinds, ['line-break', 'plain-scalar', 'value-indicator'])
  // markdown and css were generic fullkit wrappers that emitted zero
  // format-specific kinds (and spurious diagnostics on valid documents); they
  // are now hand-written format engines, so the floor is their own vocabulary.
  assert.deepEqual(depthOf(tokensOf('markdown')).specificKinds, ['code-block-line', 'code-fence-marker', 'code-span', 'fence-info', 'heading-marker', 'heading-text', 'line-break', 'text'])
  assert.deepEqual(depthOf(tokensOf('css')).specificKinds, ['colon', 'left-brace', 'property', 'right-brace', 'semicolon', 'type-selector', 'value'])
  // The four JSON/CSV-family formats added for the top-20 formats goal
  // ship their own vocabulary from the first commit; these floors are the
  // measured semantic-layer counts over the playground samples.
  assert.deepEqual(depthOf(tokensOf('json5')).specificKinds, ['colon', 'comma', 'hex-number', 'infinity', 'left-brace', 'left-bracket', 'line-comment', 'property', 'right-brace', 'right-bracket', 'single-quoted-string', 'trailing-comma'])
  assert.deepEqual(depthOf(tokensOf('jsonl')).specificKinds, ['colon', 'comma', 'left-brace', 'left-bracket', 'property', 'record-break', 'right-brace', 'right-bracket', 'true'])
  assert.deepEqual(depthOf(tokensOf('csv')).specificKinds, ['boolean-field', 'decimal-field', 'delimiter', 'field', 'header-field', 'integer-field', 'quote', 'quoted-field', 'record-break'])
  assert.deepEqual(depthOf(tokensOf('tsv')).specificKinds, ['boolean-field', 'decimal-field', 'delimiter', 'field', 'header-field', 'integer-field', 'record-break'])
  // logfmt types its values on the semantic layer only; the sample exercises
  // bare, integer, boolean and null values plus a quoted one.
  assert.deepEqual(depthOf(tokensOf('logfmt')).specificKinds, ['bare-value', 'boolean-value', 'integer-value', 'key', 'null-value', 'quoted-value', 'record-break', 'separator'])
  // ini and .properties are one dialect-parameterised crate: sections, quoted
  // values and continuations on the INI side, escapes and joined lines on the
  // properties side.
  assert.deepEqual(depthOf(tokensOf('ini')).specificKinds, ['boolean-value', 'integer-value', 'key', 'padding', 'quote', 'quoted-value', 'record-break', 'section-marker', 'section-name', 'separator', 'value'])
  assert.deepEqual(depthOf(tokensOf('properties')).specificKinds, ['boolean-value', 'escape-sequence', 'integer-value', 'key', 'line-continuation', 'padding', 'record-break', 'separator', 'value'])
  // Batch 3 closes the top-20 formats goal: blocks with labels and heredocs,
  // Clojure data literals, two subtitle dialects from one crate, and iCalendar
  // content lines. srt is the deliberate floor of the group — a subtitle cue
  // has a small vocabulary, and it is still eight kinds the generic set lacks.
  assert.deepEqual(depthOf(tokensOf('hcl')).specificKinds, ['attribute-name', 'block-label', 'block-type', 'boolean', 'heredoc-body', 'heredoc-close', 'heredoc-open', 'interpolation', 'line-comment', 'newline', 'null', 'operator'])
  assert.deepEqual(depthOf(tokensOf('edn')).specificKinds, ['bigint', 'character', 'discard', 'discarded-form', 'instant-tag', 'instant-value', 'list-close', 'list-open', 'map-close', 'map-open', 'namespaced-keyword', 'radix-integer', 'ratio', 'set-open', 'symbol', 'tag', 'tagged-value', 'uuid-tag', 'uuid-value', 'vector-close', 'vector-open'])
  assert.deepEqual(depthOf(tokensOf('srt')).specificKinds, ['cue-end', 'cue-identifier', 'cue-index', 'cue-start', 'cue-text', 'cue-text-continuation', 'record-break', 'timing-arrow'])
  // One crate, two dialect tables: WebVTT reads 24 kinds where SubRip reads 8.
  assert.deepEqual(depthOf(tokensOf('vtt')).specificKinds, ['alignment-value', 'block-marker', 'class-tag', 'closing-tag', 'cue-end', 'cue-id', 'cue-start', 'cue-text', 'cue-text-continuation', 'emphasis-tag', 'line-value', 'markup-punctuation', 'markup-value', 'position-value', 'record-break', 'setting-name', 'setting-separator', 'signature', 'signature-note', 'size-value', 'style-rule', 'timing-arrow', 'timing-tag', 'voice-tag'])
  assert.deepEqual(depthOf(tokensOf('ics')).specificKinds, ['bare-param-value', 'component-name', 'date-time-value', 'date-value', 'duration-value', 'escaped-char', 'fold-marker', 'line-break', 'parameter-assignment', 'parameter-delimiter', 'parameter-name', 'property-name', 'quoted-param-value', 'structure-marker', 'text-value', 'uri-value', 'value-delimiter'])
})

test('every kind a sample emits has an exact palette entry', () => {
  // kindColor() falls back to a hashed colour, so an unlisted kind still
  // renders — it just renders at a colour nobody chose. url's u-* parts are
  // the one deliberate exception; they are handled by prefix rules instead.
  const gaps = new Map()
  for (const lang of LANGUAGES) {
    const engine = byId[lang.id]
    for (const layer of ['syntax', 'semantic']) {
      const payload = JSON.parse(wasm.tokenize(lang.id, lang.sample, engine.defaultDialect, layer))
      for (const token of payload.tokens) {
        const kind = token.kind
        if (kind in KIND_COLORS || kind.startsWith('u-')) continue
        gaps.set(kind, `${lang.id}/${layer}`)
      }
    }
  }
  assert.deepEqual([...gaps.entries()], [], `${gaps.size} kind(s) render on a hashed fallback colour`)
})

test('generic baseline is the measured fullkit vocabulary', () => {
  assert.deepEqual([...GENERIC_KINDS].sort(), [
    'class', 'comment', 'function', 'identifier', 'keyword', 'number',
    'punctuation', 'string', 'type', 'variable', 'whitespace',
  ])
})
