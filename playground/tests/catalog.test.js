import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import test from 'node:test'
import { LANGUAGES } from '../src/languages.js'
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
  assert.equal(counts.formats, 8, 'formats20 target is tracked in docs/languages.md')
})

test('family axis overrides the naive grouping rule', () => {
  // Query languages sit in the playground's "query" group but are not formats.
  assert.equal(byId.sql.family, 'language')
  assert.equal(byId.graphql.family, 'language')
  assert.equal(byId.mongo.family, 'language')
  for (const id of ['json', 'yaml', 'toml', 'url', 'xml', 'html', 'css', 'markdown']) {
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
})

test('generic baseline is the measured fullkit vocabulary', () => {
  assert.deepEqual([...GENERIC_KINDS].sort(), [
    'class', 'comment', 'function', 'identifier', 'keyword', 'number',
    'punctuation', 'string', 'type', 'variable', 'whitespace',
  ])
})
