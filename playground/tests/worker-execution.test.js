import test from 'node:test'
import assert from 'node:assert/strict'
import { executeInWorker } from '../src/worker-execution.js'
function fake() { return {terminated:0, postMessage(value) {this.payload=value}, terminate() {this.terminated++}} }
test('worker result terminates the worker and ignores later events', async () => {
  const worker = fake()
  const pending = executeInWorker(worker, {source:'42'})
  worker.onmessage({data:{result:{ok:true}}})
  worker.onerror({message:'late error'})
  assert.deepEqual(await pending, {ok:true})
  assert.equal(worker.terminated, 1)
})
test('abort terminates a running worker and rejects the request', async () => {
  const worker = fake(), controller = new AbortController()
  const pending = executeInWorker(worker, {}, controller.signal)
  controller.abort()
  await assert.rejects(pending, /cancelled/)
  assert.equal(worker.terminated, 1)
})
test('timeout and worker failure release resources', async () => {
  const worker = fake()
  await assert.rejects(executeInWorker(worker, {}, undefined, 5), /timed out/)
  assert.equal(worker.terminated, 1)
  const broken = fake()
  const pending = executeInWorker(broken, {})
  broken.onerror({message:'load failed'})
  await assert.rejects(pending, /load failed/)
  assert.equal(broken.terminated, 1)
})
