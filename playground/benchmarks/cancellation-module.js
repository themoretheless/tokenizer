const moduleUrl = new URL('/wasm/tokenizer_wasm.js', self.location.href).href
const { default: init, run_rush } = await import(/* @vite-ignore */ moduleUrl)
export default init
export function run_rush_benchmark(source) {
  self.postMessage({ benchmarkStarted: true })
  return run_rush(source)
}
export { run_rush_benchmark as run_rush }
