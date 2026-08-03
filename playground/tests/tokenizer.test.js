import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import test from 'node:test'

const require = createRequire(import.meta.url)
const { tokenize_json: tokenizeJson } = require('../.wasm-test/tokenizer_wasm.js')

function tokenize(source, mode = 'strict', layer = 'semantic') {
  return JSON.parse(tokenizeJson(source, mode, layer))
}

test('tokenizes valid JSON through the WebAssembly bridge', () => {
  const result = tokenize('{"city":"Тбилиси"}')

  assert.equal(result.valid, true)
  assert.equal(result.sourceBytes, 25)
  assert.ok(result.tokens.some((token) => token.kind === 'property' && token.text === '"city"'))
})

test('returns recovery diagnostics for invalid JSON', () => {
  const result = tokenize('{"leadingZero":01}')

  assert.equal(result.valid, false)
  assert.ok(result.diagnostics.length > 0)
})

test('keeps JSONC behavior separate from strict JSON', () => {
  const source = '{// comment\n"items":[1,],}'

  assert.equal(tokenize(source, 'strict').valid, false)
  assert.equal(tokenize(source, 'jsonc').valid, true)
})
