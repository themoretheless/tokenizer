/** Drive the Rust scheduler with monotonic browser time and named DOM events. */
export function driveRushScheduler(scheduler, { events = new EventTarget(), signal, onStep = () => {}, onError = () => {}, clock = () => performance.now(), setTimer = setTimeout, clearTimer = clearTimeout } = {}) {
  const start = clock()
  const subscriptions = new Map()
  let timer
  let disposed = false
  function dispose() {
    if (disposed) return
    disposed = true
    if (timer !== undefined) clearTimer(timer)
    for (const [name, listener] of subscriptions) events.removeEventListener(name, listener)
    subscriptions.clear()
    signal?.removeEventListener('abort', dispose)
    scheduler.cancel_all()
    scheduler.free?.()
  }
  function pump() {
    if (disposed) return
    if (timer !== undefined) clearTimer(timer)
    timer = undefined
    try {
      const result = JSON.parse(scheduler.poll(clock() - start))
      for (const step of result.steps) {
        if (step.event && !subscriptions.has(step.event)) {
          const name = step.event
          const listener = () => {
            if (disposed) return
            try { scheduler.emit(name); pump() } catch (error) { dispose(); onError(error) }
          }
          subscriptions.set(name, listener)
          events.addEventListener(name, listener)
        }
        onStep(step)
        if (disposed) return
      }
      if (result.empty) { dispose(); return }
      const deadline = scheduler.next_deadline_ms()
      if (deadline != null) timer = setTimer(pump, Math.max(0, deadline - (clock() - start)))
    } catch (error) { dispose(); onError(error) }
  }
  signal?.addEventListener('abort', dispose, { once: true })
  if (signal?.aborted) dispose()
  else pump()
  return { dispose, emit(name) { if (!disposed) { scheduler.emit(name); pump() } } }
}
