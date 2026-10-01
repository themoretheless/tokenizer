// One worker per execution makes termination release the interpreter and its heap.
export function executeInWorker(worker, payload, signal, timeoutMs = 10000) {
  return new Promise((resolve, reject) => {
    let settled = false
    let timer
    function finish(error, value) {
      if (settled) return
      settled = true
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      worker.terminate()
      if (error) reject(error)
      else resolve(value)
    }
    function abort() { finish(new Error('Execution cancelled')) }
    if (signal?.aborted) { abort(); return }
    signal?.addEventListener('abort', abort, { once: true })
    worker.onmessage = event => {
      if (event.data.error) finish(new Error(event.data.error))
      else finish(null, event.data.result)
    }
    worker.onerror = event => finish(new Error(event.message || 'Execution worker failed'))
    worker.onmessageerror = () => finish(new Error('Invalid execution worker response'))
    timer = setTimeout(() => finish(new Error('Execution timed out')), timeoutMs)
    try { worker.postMessage(payload) } catch (error) { finish(error) }
  })
}
