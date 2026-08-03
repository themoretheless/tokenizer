let wasmModulePromise

async function tokenizeWithWasm({ source, mode, layer }) {
  if (!wasmModulePromise) {
    const base = import.meta.env.BASE_URL
    wasmModulePromise = import(/* @vite-ignore */ `${base}wasm/tokenizer_wasm.js`).then(async (module) => {
      await module.default(`${base}wasm/tokenizer_wasm_bg.wasm`)
      return module
    })
  }
  const module = await wasmModulePromise
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
