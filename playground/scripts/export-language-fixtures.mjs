// One-shot generator: write each language engine's picker sample out as a native
// fixture so `tests/wiring_completeness.rs` can assert "a valid document is
// quiet" in CI, where the Node playground suite does not run.
import { mkdirSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { LANGUAGES } from '../src/languages.js'

const require = createRequire(import.meta.url)
const wasm = require('../.wasm-test/tokenizer_wasm.js')
const catalog = JSON.parse(wasm.wasm_catalog())
const family = Object.fromEntries(catalog.engines.map((e) => [e.id, e.family]))

const dir = new URL('../../tests/fixtures/languages/', import.meta.url).pathname
mkdirSync(dir, { recursive: true })
let written = 0
for (const entry of LANGUAGES) {
  if (family[entry.id] !== 'language') continue
  writeFileSync(dir + `${entry.id}.txt`, entry.sample)
  written += 1
}
console.log(`${written} language fixtures written to tests/fixtures/languages/`)
