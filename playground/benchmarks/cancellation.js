import { executeInWorker } from '../src/worker-execution.js'

const source = 'while true { let xs = range(0,1000) | map(x => x*x) }'
const payload = {
  source,
  moduleUrl: new URL('./cancellation-module.js', import.meta.url).href,
  wasmUrl: new URL('/wasm/tokenizer_wasm_bg.wasm', location.href).href,
}
async function trial(cancel) {
  const worker = new Worker(new URL('../src/rush-worker.js', import.meta.url), { type: 'module' })
  const controller = new AbortController()
  let started, aborted, terminateStarted, terminateReturned, timer
  // Filter instrumentation messages before the production protocol sees them.
  const adapter = {
    set onmessage(handler) {
      worker.onmessage = event => {
        if (event.data.benchmarkStarted) {
          started = performance.now()
          if (cancel) timer = setTimeout(() => {
            aborted = performance.now()
            controller.abort()
          }, 1)
        } else handler(event)
      }
    },
    set onerror(handler) { worker.onerror = handler },
    set onmessageerror(handler) { worker.onmessageerror = handler },
    postMessage(value) { worker.postMessage(value) },
    terminate() {
      terminateStarted = performance.now()
      worker.terminate()
      terminateReturned = performance.now()
    },
  }
  let result, error
  try { result = await executeInWorker(adapter, payload, controller.signal) }
  catch (failure) { error = failure.message }
  const settled = performance.now()
  clearTimeout(timer)
  if (started === undefined) throw new Error(error || 'Worker did not enter Rush')
  if (cancel && (error !== 'Execution cancelled' || aborted === undefined)) {
    throw new Error(`Cancellation lost race with execution: ${JSON.stringify({result,error})}`)
  }
  if (!cancel && (error || result?.ok !== false || result.error !== 'Execution limit exceeded')) {
    throw new Error(`Unexpected baseline: ${JSON.stringify({result,error})}`)
  }
  return {
    startToSettlementMs: settled - started,
    abortToSettlementMs: cancel ? settled - aborted : null,
    abortToTerminateMs: cancel ? terminateStarted - aborted : null,
    terminateCallMs: terminateReturned - terminateStarted,
    error: error || result.error,
  }
}
const button = document.querySelector('#run')
const output = document.querySelector('#result')
button.onclick = async () => {
  button.disabled = true
  const report = { date: new Date().toISOString(), userAgent: navigator.userAgent, source: payload.source, baseline: [], cancelled: [] }
  try {
    for (let i=0; i<3; i++) report.baseline.push(await trial(false))
    for (let i=0; i<20; i++) {
      report.cancelled.push(await trial(true))
      output.textContent = `Cancelled ${i+1}/20`
    }
    const worker = new Worker(new URL('../src/rush-worker.js', import.meta.url), {type:'module'})
    report.recovery = await executeInWorker(worker, {...payload, source:'1+2', moduleUrl:new URL('/wasm/tokenizer_wasm.js', location.href).href})
    if (report.recovery.output !== 'Number(3.0)') throw new Error('Recovery failed')
    report.ok = true
  } catch (error) { report.error = error.message; report.ok = false }
  output.textContent = JSON.stringify(report, null, 2)
  button.disabled = false
}
