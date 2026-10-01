import { completionRange, applyCompletion, completionCandidates } from '../src/completion.js'
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import test from 'node:test'

const require = createRequire(import.meta.url)
const wasm = require('../.wasm-test/tokenizer_wasm.js')
const run = source => JSON.parse(wasm.run_rush(source))

test('WASM executes functional composition and exports SVG', () => {
  const result = run('range(0, 360, 5) | map(a => vec2(cos(deg(a)), sin(deg(a)))) | polygon')
  assert.equal(result.ok, true)
  assert.equal(result.kind, 'svg')
  assert.match(result.output, /<svg/)
  assert.doesNotMatch(result.output, /NaN|Infinity/)
})

test('WASM grid mesh exports indexed OBJ', () => {
  const result = run('grid_mesh(range(-3, 3.1, 0.2), range(-3, 3.1, 0.2), (x,y) => vec3(x, sin(x)*cos(y), y))')
  assert.equal(result.ok, true)
  assert.equal(result.kind, 'obj')
  const lines = result.output.split('\n')
  assert.equal(lines.filter(line => line.startsWith('v ')).length, 961)
  assert.equal(lines.filter(line => line.startsWith('f ')).length, 1800)
})

test('WASM handles errors, bounded loops and subsequent runs', () => {
  const error = run('1 / 0')
  assert.equal(error.ok, false)
  assert.equal(typeof error.error, 'string')
  assert.equal(typeof error.start, 'number')
  assert.equal(run('while true {}').ok, false)
  assert.equal(run('fn recurse(x) { return recurse(x) } recurse(1)').ok, false)
  assert.deepEqual(run('2 + 3'), { ok: true, kind: 'text', output: 'Number(5.0)' })
})

test('WASM preserves flat_map, type aliases and assertion diagnostics', () => {
  const result = run("const label: str = 'flatten'\nconst factor: float = 2\nlet values = [1,2] | flat_map(x => [x, x * factor])\nassert(values == [1,2,2,4], label)\nvalues")
  assert.equal(result.ok, true)
  const source = "assert(false, 'curve check failed')"
  const failed = run(source)
  assert.equal(failed.ok, false)
  assert.equal(failed.error, 'curve check failed')
  assert.equal(source.slice(failed.start, failed.end), source)
  assert.equal(run('assert(true)').ok, true)
})

test('playground returns execution checks without changing grammar validity', () => {
  const source = 'const value = missing\nsin(1,2)'
  const result = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.equal(result.valid, true)
  assert.ok(result.executionDiagnostics.some(d => d.code === 'unknown-name'))
  assert.ok(result.executionDiagnostics.some(d => d.code === 'argument-count'))
  const valid = JSON.parse(wasm.tokenize('rush', '[1,2] | map(x => x * 2)', 'default', 'semantic'))
  assert.deepEqual(valid.executionDiagnostics, [])
})

test('playground reference spans resolve shadowed Rush names', () => {
  const source = 'const x = 1\nconst f = x => x + 1\nf(x)'
  const result = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const inner = result.references.find(r => r.start === source.indexOf('x +'))
  const outer = result.references.find(r => r.start === source.lastIndexOf('x'))
  assert.equal(inner.definition.start, source.indexOf('x =>'))
  assert.equal(outer.definition.start, source.indexOf('x ='))
})

test('completion signatures come from the runtime builtin catalogue', () => {
  const result = JSON.parse(wasm.tokenize('rush', '', 'default', 'semantic'))
  assert.deepEqual(result.builtins.find(b => b.name === 'map'), { name: 'map', minArgs: 2, maxArgs: 2 })
  assert.deepEqual(result.builtins.find(b => b.name === 'range'), { name: 'range', minArgs: 2, maxArgs: 3 })
  assert.equal(new Set(result.builtins.map(b => b.name)).size, result.builtins.length)
})

test('WASM editor metadata drives local completion using resolver scopes', async () => {
  const { completionRange, completionCandidates, applyCompletion } = await import('../src/completion.js')
  const source = 'const radius = 4\nfn f((radius, height)) { return ra }\nrad'
  const result = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const inside = source.indexOf('ra }') + 2
  const range = completionRange(source, inside)
  const candidates = completionCandidates(source, range, result)
  const radius = candidates.find(x => x.name === 'radius')
  assert.equal(radius.definition.start, source.indexOf('radius, height'))
  assert.equal(applyCompletion(source, range, radius).source.includes('return radius }'), true)
  const outside = completionCandidates(source, completionRange(source, source.length), result)
  assert.equal(outside.find(x => x.name === 'radius').definition.start, source.indexOf('radius'))
  assert.equal(result.bindings.some(x => x.name === 'height'), true)
  assert.equal(outside.some(x => x.name === 'height'), false)
})

test('WASM closure frames preserve mutable captures and repeated-run isolation', () => {
  const source = `fn counter(seed) {
    mut value = seed
    return () => value += 1
  }
  let a = counter(10)
  let b = counter(100)
  assert(a() == 11)
  assert(b() == 101)
  assert(a() == 12)
  let saved = [1,2,3] | map(x => (() => x))
  assert(saved[0]() == 1)
  assert(saved[2]() == 3)
  a()`
  const script = source.replace(/^  /gm, '')
  for (let i = 0; i < 2; i++) {
    assert.deepEqual(run(script), { ok: true, kind: 'text', output: 'Number(13.0)' })
  }
  assert.deepEqual(run('42; if false { 7 }'), { ok: true, kind: 'text', output: 'Null' })
})

test('WASM editor checks conditional and destructured function signatures', () => {
  for (const source of [
    'let f = if true { sin } else { cos }; f(1,2)',
    'let {run:f} = {run:sin}; 1 | f(2)',
    'let (sin,f) = ((x,y) => x+y,sin); f(1,2)',
  ]) {
    const result = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.equal(result.valid, true)
    assert.ok(result.executionDiagnostics.some(d => d.code === 'argument-count'), source)
  }
})


test('WASM range arithmetic avoids intermediate overflow', () => {
  for (const source of [
    'range(-1e308, 1e308, 1e308) == [-1e308, 0]',
    '(range_iter(1e308, -1e308, -1e308) | collect(10)) == [1e308, 0]',
  ]) {
    assert.deepEqual(run(source), { ok: true, kind: 'text', output: 'Bool(true)' })
  }
})

test('WASM enforces collection and decoded string limits', () => {
  for (const source of ['range(0,10001)', 'range_iter(0,1000000) | collect(10001)']) {
    const result = run(source)
    assert.equal(result.ok, false)
    assert.equal(result.error, 'Collection item limit exceeded')
    assert.ok(result.end > result.start)
  }
  assert.equal(run('range_iter(0,1000000) | collect(2)').ok, true)
  const result = run(`"${'a'.repeat(65537)}"`)
  assert.equal(result.ok, false)
  assert.equal(result.error, 'String byte limit exceeded')
  assert.equal(run(`"${'a'.repeat(65536)}"`).ok, true)
})

test('WASM stops text, SVG and OBJ serialization at the output limit', () => {
  const cases = [
    [`let text = "${'a'.repeat(1000)}"; range(0,2000) | map(x => text)`, 'Text'],
    ['range(0,2000) | map(x => vec2(1e308,1e308)) | polygon', 'SVG'],
    ['grid_mesh(range(0,35),range(0,35),(x,y) => vec3(1e308,1e308,1e308))', 'OBJ'],
  ]
  for (const [source, format] of cases) {
    const result = run(source)
    assert.equal(result.ok, false)
    assert.equal(result.error, `${format} output byte limit exceeded`)
    assert.ok(JSON.stringify(result).length < 200)
  }
})

test('WASM export errors use execution error fields without a false source span', () => {
  const result = run('polygon([vec2(-1e308,0),vec2(1e308,0),vec2(0,1)])')
  assert.deepEqual(result, {
    ok: false,
    code: 'export-error',
    error: 'SVG bounds overflow',
  })
  assert.equal(Object.hasOwn(result, 'start'), false)
  assert.equal(Object.hasOwn(result, 'end'), false)
})

test('WASM editor metadata drives vector field completion', async () => {
  const { completionCandidates, completionRange } = await import('../src/completion.js')
  const source = 'let v = vec3(1,2,3); v.x'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.deepEqual(payload.bindings.find(x => x.name === 'v').members, ['x','y','z'])
  const range = completionRange(source, source.length - 1)
  assert.deepEqual(completionCandidates(source, range, payload).map(x => x.name), ['x','y','z'])
})

test('WASM keeps field completion for a trailing dot without accepting execution', async () => {
  const { completionCandidates, completionRange } = await import('../src/completion.js')
  const source = 'let v = vec3(1,2,3); v.'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.ok(payload.diagnostics.some(d => d.code === 'expected-name'))
  const candidates = completionCandidates(source, completionRange(source, source.length), payload)
  assert.deepEqual(candidates.map(x => x.name), ['x','y','z'])
  assert.equal(run(source).ok, false)
})

test('WASM reports known vector arithmetic and member errors before execution', () => {
  for (const [source, code] of [
    ['let v = vec2(1,2); v.z', 'vector-component'],
    ['vec2(1,2) + vec3(1,2,3)', 'vector-operands'],
    ['2 / vec2(1,2)', 'vector-operands'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
})

test('WASM exposes annotation diagnostics and executes null and unary vectors', () => {
  for (const [source, code] of [
    ['let v: vec3 = vec2(1,2)', 'annotation-type'],
    ['fn f() -> number { return true }', 'return-type'],
    ['fn f(v: unknown) { return v }', 'unsupported-annotation'],
    ['fn f(v: number) { return v }; let g = f; g(true)', 'argument-type'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
  assert.deepEqual(run('fn f() -> null { return }; f()'), {ok:true,kind:'text',output:'Null'})
  assert.deepEqual(run('-vec2(1,-2) == vec2(-1,2)'), {ok:true,kind:'text',output:'Bool(true)'})
})

test('WASM record metadata completes aliases and filters field prefixes', async () => {
  const { completionCandidates, completionRange, applyCompletion } = await import('../src/completion.js')
  const source = 'let config = {width:10,height:20}; let alias = config; alias.he'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.deepEqual(payload.bindings.find(x => x.name === 'alias').members, ['height','width'])
  const range = completionRange(source, source.length)
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates, [{name:'height',kind:'field'}])
  const completed = applyCompletion(source, range, candidates[0]).source
  assert.ok(completed.endsWith('alias.height'))
  assert.deepEqual(run(completed), {ok:true,kind:'text',output:'Number(20.0)'})
})

test('WASM reports record, pattern and boolean contract errors', () => {
  for (const [source, code] of [
    ['let config = {width:10}; config.widht', 'unknown-record-field'],
    ['let {height:h} = {width:10}', 'unknown-record-field'],
    ['let (a,b) = (1,2,3)', 'tuple-pattern'],
    ['fn f((a,b): number) { return a }', 'tuple-pattern'],
    ['if 1 { 2 }', 'condition-type'],
    ['true and 2', 'condition-type'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
})

test('WASM reports missing returns and respects nested loop exits', () => {
  for (const source of [
    'fn f() -> number { 1 }',
    'fn f() -> number { while true { break } }',
    'fn f(flag) -> number { while flag { return 1 } }',
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === 'missing-return'), source)
  }
  for (const source of [
    'fn f() -> number { while true { while true { break } } }',
    'fn f() -> number { while true { continue; break } }',
    'fn f() -> number { while true { break }; return 1 }; f()',
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(!payload.executionDiagnostics.some(d => d.code === 'missing-return'), source)
  }
  assert.deepEqual(run('fn f() -> number { while true { break }; return 1 }; f()'),
    {ok:true,kind:'text',output:'Number(1.0)'})
})

test('WASM checks collection indices and indexed vector components', () => {
  for (const [source, code] of [
    ['vec2(1,2)[2]', 'index-bounds'],
    ['[1,2][-1]', 'index-bounds'],
    ['vec3(1,2,3)[true]', 'index-type'],
    ['fn f(v: list[vec2]) { return v[0].z }', 'vector-component'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
  assert.deepEqual(run('fn f(v: list[vec3]) { return v[0].z }; f([vec3(1,2,3)])'),
    {ok:true,kind:'text',output:'Number(3.0)'})
})

test('WASM groups records with stable keys and item order', () => {
  assert.deepEqual(run("let groups = [3,2,1,4] | group_by(x => if x % 2 == 0 { 'even' } else { 'odd' }); assert(groups[0].key == 'odd'); assert(groups[0].values == [3,1]); len(groups)"),
    {ok:true,kind:'text',output:'Number(2.0)'})
  const result = run('group_by([1], x => x)')
  assert.equal(result.ok, false)
  assert.match(result.error, /Group key must be a string/)
})

test('WASM aggregates groups directly from a lazy source', () => {
  assert.deepEqual(run("let groups = range_iter(0,100) | fold_by(x => 'all', 0, (sum,x) => sum+x); groups[0].value"),
    {ok:true,kind:'text',output:'Number(4950.0)'})
  assert.equal(run("fold_by([], x => 'all', 0, 1)").ok, false)
})

test('WASM diagnoses incorrect reducer arity before execution', () => {
  const source = "[] | fold_by(x => 'all', 0, x => x)"
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.ok(payload.executionDiagnostics.some(d => d.code === 'callback-argument-count'))
  assert.match(run(source).error, /Callback argument count/)
})

test('WASM keeps callback contracts for immutable builtin aliases', () => {
  const source = 'let transform = map; transform([], (a,b) => a)'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.ok(payload.executionDiagnostics.some(d => d.code === 'callback-argument-count'))
})

test('WASM diagnoses nested record fields and preserves nested aliases', () => {
  for (const [source, code] of [
    ['let config = {camera:{position:vec3(1,2,3)}}; config.camera.postion', 'unknown-record-field'],
    ['let config = {camera:{position:vec3(1,2,3)}}; config.camera.position.w', 'vector-component'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
  const source = 'let config = {camera:{position:vec3(1,2,3)}}; let camera = config.camera; camera.'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.deepEqual(payload.bindings.find(x => x.name === 'camera').members, ['position'])
})

test('WASM completes nested records and vector fields without a local alias', () => {
  for (const [tail, names] of [
    ['config.camera.', ['position']],
    ['config.camera.position.', ['x','y','z']],
    ['config . camera . position.z', ['z']],
  ]) {
    const source = 'let config = {camera:{position:vec3(1,2,3)}}; ' + tail
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    const range = completionRange(source, source.length)
    const candidates = completionCandidates(source, range, payload)
    assert.deepEqual(candidates.map(x => x.name), names, source)
    if (tail.endsWith('.z')) {
      const completed = applyCompletion(source, range, candidates[0]).source
      assert.deepEqual(run(completed), {ok:true,kind:'text',output:'Number(3.0)'})
    }
  }
  const source = 'let config = {camera:{position:vec3(1,2,3)}}; if true { let config = {other:0}; config.camera.'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.deepEqual(completionCandidates(source, completionRange(source, source.length), payload), [])
})

test('WASM diagnoses data used as a function or callback', () => {
  for (const [source, code] of [
    ['let value = 1; value()', 'not-callable'],
    ['let config = {f:1}; config.f()', 'not-callable'],
    ['map([], 1)', 'callback-type'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
})

test('WASM preserves record completions and diagnostics after destructuring', () => {
  for (const declaration of [
    'let (config, _) = ({camera:{position:vec3(1,2,3)}}, 0); ',
    'let {settings:config} = {settings:{camera:{position:vec3(1,2,3)}}}; ',
  ]) {
    const source = declaration + 'config.camera.position.z'
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    const range = completionRange(source, source.length)
    const candidates = completionCandidates(source, range, payload)
    assert.deepEqual(candidates.map(x => x.name), ['z'])
    assert.deepEqual(run(applyCompletion(source, range, candidates[0]).source),
      {ok:true,kind:'text',output:'Number(3.0)'})
    const invalid = JSON.parse(wasm.tokenize('rush', declaration + 'config.camera.missing', 'default', 'semantic'))
    assert.ok(invalid.executionDiagnostics.some(d => d.code === 'unknown-record-field'))
    const shadowed = declaration + 'if true { let config = {other:0}; config.camera.'
    const shadowPayload = JSON.parse(wasm.tokenize('rush', shadowed, 'default', 'semantic'))
    assert.deepEqual(completionCandidates(shadowed, completionRange(shadowed, shadowed.length), shadowPayload), [])
  }
})

test('WASM diagnoses member access on known unsupported values', () => {
  const source = 'let value = null; value.missing'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.ok(payload.executionDiagnostics.some(d => d.code === 'member-object'))
  assert.match(run(source).error, /Value does not support member access/)
})

test('WASM completes mesh fields and preserves vertex types through indexing', () => {
  const source = 'fn inspect(m: mesh) -> list[vec3] { return m.ve }\nlen(inspect(grid_mesh([0,1],[0,1],(x,y) => vec3(x,y,0))))'
  const cursor = source.indexOf('m.ve') + 4
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.ok(payload.executionDiagnostics.some(d => d.code === 'mesh-field'))
  const range = completionRange(source, cursor)
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates.map(x => x.name), ['vertices'])
  const completed = applyCompletion(source, range, candidates[0]).source
  assert.deepEqual(run(completed), {ok:true,kind:'text',output:'Number(4.0)'})
  const valid = JSON.parse(wasm.tokenize('rush', completed, 'default', 'semantic'))
  assert.deepEqual(valid.executionDiagnostics, [])
  const invalid = 'fn bad(m: mesh) { return m.vertices[0].w }'
  const checked = JSON.parse(wasm.tokenize('rush', invalid, 'default', 'semantic'))
  assert.ok(checked.executionDiagnostics.some(d => d.code === 'vector-component'))
})

test('WASM checks nested patterns and tuple aliases consistently', () => {
  for (const [source, code] of [
    ['let {outer:{missing:value}} = {outer:{known:1}}', 'unknown-record-field'],
    ['let {outer:{field:value}} = {outer:1}', 'record-pattern'],
    ['let values = [1,2]; let alias = values; alias[2]', 'index-bounds'],
    ['fn read(pair: tuple[vec2,number]) { let (point, _) = pair; return point.z }', 'vector-component'],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    assert.ok(payload.executionDiagnostics.some(d => d.code === code), source)
  }
  const source = 'let pair = ({position:vec3(1,2,3)},0); let alias = pair; let (config, _) = alias; config.position.z'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const range = completionRange(source, source.length)
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates.map(x => x.name), ['z'])
  assert.deepEqual(run(applyCompletion(source, range, candidates[0]).source),
    {ok:true,kind:'text',output:'Number(3.0)'})
})

test('WASM completes inferred loop element fields within their scope', () => {
  const source = 'let point = {outer:0}; for point in [vec3(1,2,3)] { point.z }; point.outer'
  const cursor = source.indexOf('point.z') + 'point.z'.length
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const range = completionRange(source, cursor)
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates.map(x => x.name), ['z'])
  const outerRange = completionRange(source, source.length)
  assert.deepEqual(completionCandidates(source, outerRange, payload).map(x => x.name), ['outer'])
  assert.deepEqual(run(applyCompletion(source, range, candidates[0]).source),
    {ok:true,kind:'text',output:'Number(0.0)'})
  const nested = 'for item in [{position:vec3(1,2,3)}] { item.position.z }'
  const nestedCursor = nested.indexOf('item.position.z') + 'item.position.z'.length
  const nestedPayload = JSON.parse(wasm.tokenize('rush', nested, 'default', 'semantic'))
  assert.deepEqual(completionCandidates(nested, completionRange(nested, nestedCursor), nestedPayload).map(x => x.name), ['z'])
  const invalid = 'for point in [vec2(1,2)] { point.z }'
  const invalidPayload = JSON.parse(wasm.tokenize('rush', invalid, 'default', 'semantic'))
  assert.ok(invalidPayload.executionDiagnostics.some(d => d.code === 'vector-component'))
})

test('record literal index carries nested fields into completion through WASM', () => {
  for (const key of ['"camera"', "'camera'", '"\\u{63}amera"']) {
    const declaration = `let config={camera:{position:vec3(1,2,3)}}; let camera=config[${key}]; `
    const source = declaration + 'camera.position.'
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    const range = completionRange(source, source.length)
    const candidates = completionCandidates(source, range, payload)
    assert.deepEqual(candidates.map(item => item.name), ['x', 'y', 'z'])
    const completed = applyCompletion(source, range, candidates[2]).source
    assert.deepEqual(run(completed), {ok:true,kind:'text',output:'Number(3.0)'})
  }
  const source = 'let config={camera:1}; config["missing"]'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  assert.ok(payload.executionDiagnostics.some(item => item.code === 'unknown-record-field'))
})

test('AST member receivers support calls, parentheses and literal indexing', () => {
  for (const [source, expected] of [
    ['vec3(1,2,3).', ['x','y','z']],
    ['(vec2(1,2)).', ['x','y']],
    ['[vec3(1,2,3)][0].', ['x','y','z']],
    ['let config={camera:{position:vec3(1,2,3)}}; config["camera"].', ['position']],
    ['fn point() -> vec3 { return vec3(1,2,3) }; point().', ['x','y','z']],
    ['fn vec3(x:number)->number { return x }; vec3(1).', []],
    ['let имя={point:vec2(1,2)}; имя.point.', ['x','y']],
    ['unknown().', []],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    const candidates = completionCandidates(source, completionRange(source, source.length), payload)
    assert.deepEqual(candidates.map(item => item.name), expected, source)
  }
  const source = 'fn point() -> vec3 { return vec3(1,2,3) }; point().z'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const range = completionRange(source, source.length)
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates.map(item => item.name), ['z'])
  assert.deepEqual(run(applyCompletion(source, range, candidates[0]).source),
    {ok:true,kind:'text',output:'Number(3.0)'})
})

test('vector math completion follows argument dimensions and pipeline stages', () => {
  for (const [source, fields] of [
    ['normalize(vec2(1,0)).', ['x','y']],
    ['let n=normalize; n(vec3(1,0,0)).', ['x','y','z']],
    ['(vec4(1,0,0,0) | normalize | normalize).', ['x','y','z','w']],
    ['lerp(vec2(0,0),vec2(2,4),0.5).', ['x','y']],
    ['(vec3(1,0,0) | lerp(vec3(0,1,0),0.5) | normalize).', ['x','y','z']],
    ['cross(vec3(1,0,0),vec3(0,1,0)).', ['x','y','z']],
  ]) {
    const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
    const range = completionRange(source, source.length)
    const candidates = completionCandidates(source, range, payload)
    assert.deepEqual(candidates.map(item => item.name), fields, source)
    assert.equal(run(applyCompletion(source, range, candidates[0]).source).ok, true)
  }
})

test('WASM reports vector dimension mismatch before execution', () => {
  const source = 'let d=dot; vec2(1,2) | d(vec3(1,2,3))'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const diagnostic = payload.executionDiagnostics.find(item => item.code === 'vector-dimensions')
  assert.ok(diagnostic)
  assert.equal(source.slice(diagnostic.start, diagnostic.end), 'vec3(1,2,3)')
  assert.equal(run(source).ok, false)
})

test('WASM reports nonnumeric ordered comparison at the operand', () => {
  const source = 'let angle=degrees(90); 1 < angle'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const diagnostics = payload.executionDiagnostics.filter(item => item.code === 'comparison-operand')
  assert.equal(diagnostics.length, 1)
  assert.equal(diagnostics[0].start, source.lastIndexOf('angle'))
  assert.equal(source.slice(diagnostics[0].start, diagnostics[0].end), 'angle')
  assert.equal(run(source).ok, false)
  assert.equal(run('degrees(90)==degrees(90)').ok, true)
})

test('WASM checks transformation types and completes generated mesh members', () => {
  const bad = JSON.parse(wasm.tokenize('rush', 'transform_point(identity(),vec2(1,2))', 'default', 'semantic'))
  assert.ok(bad.executionDiagnostics.some(item => item.code === 'argument-type'))
  const source = 'let shape=grid_mesh([0,1],[0,1],(x,y)=>vec3(x,y,0)) | transform(identity()); shape.'
  const payload = JSON.parse(wasm.tokenize('rush', source, 'default', 'semantic'))
  const range = completionRange(source, source.length)
  const candidates = completionCandidates(source, range, payload)
  assert.deepEqual(candidates.map(item => item.name), ['triangles','vertices'])
  const completed = applyCompletion(source, range, candidates[1]).source
  assert.equal(run(completed + ' | len').output, 'Number(4.0)')
})
