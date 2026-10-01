import test from 'node:test'
import assert from 'node:assert/strict'
import { runProcess } from '../server/process.js'

test('process runner returns stdout and reports nonzero exits', async () => {
  assert.equal(await runProcess(process.execPath, ['-e', 'process.stdout.write("ok")']), 'ok')
  await assert.rejects(runProcess(process.execPath, ['-e', 'process.stderr.write("failure"); process.exit(2)']), /failure/)
})
test('abort stops a running process', async () => {
  const controller = new AbortController()
  const pending = runProcess(process.execPath, ['-e', 'setInterval(()=>{},1000)'], {signal:controller.signal})
  const timer = setTimeout(() => controller.abort(), 100)
  try { await assert.rejects(pending, /cancelled/) } finally { clearTimeout(timer) }
})
test('pre-aborted request does not start a command', async () => {
  const controller = new AbortController(); controller.abort()
  await assert.rejects(runProcess('nonexistent-command', [], {signal:controller.signal}), /cancelled/)
})

test('POSIX cancellation stops both launcher and descendant', {skip:process.platform === 'win32', timeout:5000}, async () => {
  const { mkdtemp, readFile, rm } = await import('node:fs/promises')
  const { tmpdir } = await import('node:os')
  const { join } = await import('node:path')
  const { setTimeout: delay } = await import('node:timers/promises')
  const directory = await mkdtemp(join(tmpdir(), 'rush-process-'))
  const marker = join(directory, 'pids.json')
  const controller = new AbortController()
  const script = `const {spawn}=require('node:child_process'); const fs=require('node:fs'); const child=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'}); fs.writeFileSync(process.argv[1],JSON.stringify([process.pid,child.pid])); setInterval(()=>{},1000)`
  const pending = runProcess(process.execPath, ['-e', script, marker], {signal:controller.signal})
  // Observe rejection immediately so cleanup cannot create an unhandled promise.
  const stopped = assert.rejects(pending, /cancelled/)
  try {
    let pids
    for (let attempt=0; attempt<100; attempt++) {
      try { pids = JSON.parse(await readFile(marker,'utf8')); break } catch { await delay(10) }
    }
    assert.ok(pids, 'launcher did not report descendant PID')
    for (const pid of pids) process.kill(pid, 0)
    controller.abort()
    await stopped
    for (const pid of pids) {
      let alive = true
      for (let attempt=0; attempt<100; attempt++) {
        try { process.kill(pid,0); await delay(10) } catch(error) { if(error.code !== 'ESRCH') throw error; alive=false; break }
      }
      assert.equal(alive,false, `process ${pid} survived cancellation`)
    }
  } finally {
    controller.abort()
    await stopped
    await rm(directory,{recursive:true,force:true})
  }
})
