import test from 'node:test'
import assert from 'node:assert/strict'
import { completionRange, applyCompletion, completionCandidates } from '../src/completion.js'
const item = { name: 'normalize', maxArgs: 1 }

test('completion replaces a whole identifier even from its middle', () => {
  const source = 'normalize'
  const range = completionRange(source, 3)
  assert.equal(range.prefix, 'nor')
  assert.deepEqual(applyCompletion(source, range, item), { source: 'normalize()', cursor: 10 })
})
test('completion preserves an existing call and rejects stale edits', () => {
  const source = 'nor (vec3(1,0,0))'
  const range = completionRange(source, 3)
  assert.equal(applyCompletion(source, range, item).source, 'normalize (vec3(1,0,0))')
  assert.equal(applyCompletion(source + ' ', range, item), null)
})
test('completion handles selection, Unicode prefix and zero-argument calls', () => {
  const source = 'я + nor'
  assert.equal(applyCompletion(source, completionRange(source, 7), item).source, 'я + normalize()')
  assert.equal(applyCompletion('bad', completionRange('bad', 0, 3), item).source, 'normalize()')
  assert.deepEqual(applyCompletion('iden', completionRange('iden', 4), {name:'identity',maxArgs:0}), {source:'identity()',cursor:10})
})

test('local bindings insert identifiers without a call', () => {
  assert.deepEqual(applyCompletion('ra', completionRange('ra', 2), { name: 'radius', kind: 'binding' }), { source: 'radius', cursor: 6 })
})

test('scope candidates resolve shadowing, UTF-8 offsets, and builtin collisions', async () => {
  const { completionCandidates } = await import('../src/completion.js')
  const source = 'я; ma'
  const payload = {
    builtins: [{name:'map', maxArgs:2}, {name:'match_unused', maxArgs:0}],
    bindings: [
      {name:'map', kind:'binding', start:0,end:100,depth:0,definition:'outer'},
      {name:'map', kind:'binding', start:4,end:6,depth:1,definition:'inner'},
      {name:'maybe_later', kind:'binding', start:6,end:100,depth:0},
      {name:'main_closed', kind:'binding', start:0,end:4,depth:1},
    ],
  }
  const items = completionCandidates(source, completionRange(source, source.length), payload)
  assert.deepEqual(items.map(x => x.name), ['map', 'match_unused'])
  assert.equal(items[0].definition, 'inner')
  assert.deepEqual(completionCandidates('object.ma', completionRange('object.ma', 9), payload), [])
})

test('known vector fields complete without adding call parentheses', async () => {
  const { completionCandidates } = await import('../src/completion.js')
  const source = 'let v = vec3(1,2,3); v.x'
  const range = completionRange(source, source.length - 1)
  const payload = { bindings: [{ name: 'v', start: 19, end: 100, depth: 0, members: ['x','y','z'] }] }
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates.map(x => x.name), ['x','y','z'])
  assert.equal(applyCompletion(source, range, candidates[2]).source, 'let v = vec3(1,2,3); v.z')
  payload.bindings.push({ name: 'v', start: 20, end: 100, depth: 1, members: [] })
  assert.deepEqual(completionCandidates(source, range, payload), [])
})

test('member chains do not borrow a same-named local vector type', async () => {
  const { completionCandidates } = await import('../src/completion.js')
  for (const source of ['object.v.x', 'object.v.']) {
    const range = completionRange(source, source.endsWith('.') ? source.length : source.length - 1)
    const payload = { bindings: [{ name: 'v', start: 0, end: 100, depth: 0, members: ['x','y'] }] }
    assert.deepEqual(completionCandidates(source, range, payload), [])
  }
})

test('nested member completion does not reinterpret a suffix of a call result', () => {
  const source = 'make(). config.camera.'
  const payload = {bindings:[{name:'config',start:0,end:100,depth:0,memberPaths:{camera:['position']}}]}
  assert.deepEqual(completionCandidates(source, completionRange(source, source.length), payload), [])
})
