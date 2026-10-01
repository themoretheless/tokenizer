import { executeInWorker } from './worker-execution.js'
let wasmModulePromise

async function tokenizeWithWasm({ source, language = 'json', mode, layer }) {
  if (!wasmModulePromise) {
    const base = import.meta.env.BASE_URL
    wasmModulePromise = import(/* @vite-ignore */ `${base}wasm/tokenizer_wasm.js`)
      .then(async (module) => {
        await module.default({ module_or_path: `${base}wasm/tokenizer_wasm_bg.wasm` })
        return module
      })
      .catch((error) => {
        wasmModulePromise = undefined
        throw error
      })
  }
  const module = await wasmModulePromise
  if (typeof module.tokenize === 'function') {
    return JSON.parse(module.tokenize(language, source, mode, layer))
  }
  return JSON.parse(module.tokenize_json(source, mode, layer))
}

async function tokenizeWithDevServer(payload) {
  const response = await fetch('/api/tokenize', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  })
  const result = await response.json()
  if (!response.ok) throw new Error(result.error || 'Tokenization failed')
  return result
}

export function runTokenizer(payload) {
  return import.meta.env.DEV ? tokenizeWithDevServer(payload) : tokenizeWithWasm(payload)
}

async function catalogWithWasm() {
  if (!wasmModulePromise) {
    const base = import.meta.env.BASE_URL
    wasmModulePromise = import(/* @vite-ignore */ `${base}wasm/tokenizer_wasm.js`)
      .then(async (module) => {
        await module.default({ module_or_path: `${base}wasm/tokenizer_wasm_bg.wasm` })
        return module
      })
      .catch((error) => {
        wasmModulePromise = undefined
        throw error
      })
  }
  const module = await wasmModulePromise
  if (typeof module.wasm_catalog !== 'function') throw new Error('WASM build predates the catalog export')
  return JSON.parse(module.wasm_catalog())
}

/// Registry truth: id, family, presets, capabilities and dialects per engine.
export async function runCatalog() {
  if (!import.meta.env.DEV) return catalogWithWasm()
  const response = await fetch('/api/catalog')
  const result = await response.json()
  if (!response.ok) throw new Error(result.error || 'Catalog failed')
  return result
}

export async function runRush(source, signal) {
  if (import.meta.env.DEV) {
    const response = await fetch('/api/run-rush', { signal, method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ source }) })
    if (!response.ok) throw new Error(`Execution request failed: ${response.status}`)
    return response.json()
  }
  const base = new URL(import.meta.env.BASE_URL, window.location.href)
  return executeInWorker(new Worker(new URL('./rush-worker.js', import.meta.url), { type: 'module' }), {
    source,
    moduleUrl: new URL('wasm/tokenizer_wasm.js', base).href,
    wasmUrl: new URL('wasm/tokenizer_wasm_bg.wasm', base).href,
  }, signal)
}
