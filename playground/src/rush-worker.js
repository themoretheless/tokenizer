self.onmessage = async event => {
  try {
    const { source, moduleUrl, wasmUrl } = event.data
    const module = await import(/* @vite-ignore */ moduleUrl)
    await module.default({ module_or_path: wasmUrl })
    self.postMessage({ result: JSON.parse(module.run_rush(source)) })
  } catch (error) {
    self.postMessage({ error: error.message || String(error) })
  }
}
