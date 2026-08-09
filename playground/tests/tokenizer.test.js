import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import test from 'node:test'
import { ALL_LANGUAGE_IDS, LANGUAGE_CASES, defaultMode } from './language-cases.js'

const require = createRequire(import.meta.url)
const wasm = require('../.wasm-test/tokenizer_wasm.js')

function run(language, source, mode, layer = 'semantic') {
  const raw =
    typeof wasm.tokenize === 'function'
      ? wasm.tokenize(language, source, mode, layer)
      : wasm.tokenize_json(source, mode, layer)
  return JSON.parse(raw)
}

function tokenKinds(result) {
  return new Set((result.tokens ?? []).map((t) => t.kind))
}

function assertLossless(result, source) {
  const rebuilt = (result.tokens ?? []).map((t) => t.text ?? '').join('')
  assert.equal(
    rebuilt,
    source,
    `tokens must cover source losslessly for language=${result.language}`,
  )
}

function assertCase(language, c) {
  const mode = c.mode ?? defaultMode(language)
  const layer = c.layer ?? 'semantic'
  const result = run(language, c.source, mode, layer)

  assert.equal(result.language, language, `${language}/${c.name}: language field`)
  assert.equal(typeof result.sourceBytes, 'number')
  assert.equal(result.sourceBytes, Buffer.byteLength(c.source, 'utf8'))
  assert.ok(Array.isArray(result.tokens), `${language}/${c.name}: tokens array`)
  assert.ok(Array.isArray(result.diagnostics), `${language}/${c.name}: diagnostics array`)

  if (c.minTokens != null) {
    assert.ok(
      result.tokens.length >= c.minTokens,
      `${language}/${c.name}: expected >= ${c.minTokens} tokens, got ${result.tokens.length}`,
    )
  }

  if (c.expectValid === true) {
    assert.equal(result.valid, true, `${language}/${c.name}: expected valid`)
  }
  if (c.expectValid === false) {
    assert.equal(result.valid, false, `${language}/${c.name}: expected invalid`)
  }

  if (c.expectKinds?.length) {
    const kinds = tokenKinds(result)
    for (const kind of c.expectKinds) {
      assert.ok(
        kinds.has(kind),
        `${language}/${c.name}: missing kind "${kind}", have [${[...kinds].join(', ')}]`,
      )
    }
  }

  if (c.maxDiagnostics != null) {
    assert.ok(result.diagnostics.length <= c.maxDiagnostics)
  }

  // Full cover when tokenizer emits text spans (bridge always includes text).
  if (result.tokens?.length) {
    assertLossless(result, c.source)
  } else {
    assert.equal(c.source, '', `${language}/${c.name}: empty tokens only for empty source`)
  }

  // Span sanity
  for (const token of result.tokens ?? []) {
    assert.ok(Number.isInteger(token.start))
    assert.ok(Number.isInteger(token.end))
    assert.ok(token.start >= 0 && token.end >= token.start)
    assert.ok(token.end <= result.sourceBytes)
    assert.equal(typeof token.kind, 'string')
    assert.ok(token.kind.length > 0)
  }
}

// ─── Per-language case matrix ───────────────────────────────────────────────

for (const language of ALL_LANGUAGE_IDS) {
  const entry = LANGUAGE_CASES[language]
  assert.ok(entry, `missing cases for ${language}`)
  assert.ok(entry.cases.length > 0, `${language} has no cases`)

  for (const c of entry.cases) {
    test(`${language} / ${c.name}`, () => {
      assertCase(language, c)
    })
  }

  test(`${language} / syntax-layer-smoke`, () => {
    const mode = defaultMode(language)
    const sample = entry.cases.find((x) => x.source.length > 0)?.source ?? 'x'
    const result = run(language, sample, mode, 'syntax')
    assert.equal(result.language, language)
    assert.equal(result.layer, 'syntax')
    assert.ok(Array.isArray(result.tokens))
    if (result.tokens.length) assertLossless(result, sample)
  })

  test(`${language} / semantic-layer-smoke`, () => {
    const mode = defaultMode(language)
    const sample = entry.cases.find((x) => x.source.length > 0)?.source ?? 'x'
    const result = run(language, sample, mode, 'semantic')
    assert.equal(result.language, language)
    assert.equal(result.layer, 'semantic')
    assert.ok(Array.isArray(result.tokens))
    if (result.tokens.length) assertLossless(result, sample)
  })
}

// ─── Cross-cutting ──────────────────────────────────────────────────────────

test('catalog covers every expected language id', () => {
  // Keep in sync with facade all-languages registry (json+url+35 plugins).
  const expected = [
    'json',
    'url',
    'xml',
    'html',
    'css',
    'yaml',
    'toml',
    'markdown',
    'sql',
    'mongo',
    'bash',
    'powershell',
    'javascript',
    'typescript',
    'python',
    'java',
    'csharp',
    'go',
    'php',
    'ruby',
    'c',
    'cpp',
    'rust',
    'kotlin',
    'swift',
    'dart',
    'r',
    'visualbasic',
    'fortran',
    'matlab',
    'delphi',
    'scala',
    'lua',
    'perl',
    'objectivec',
    'julia',
    'assembly',
  ]
  assert.deepEqual([...ALL_LANGUAGE_IDS].sort(), [...expected].sort())
})

test('unknown language returns protocol error payload', () => {
  const result = run('no-such-lang', 'x', 'default', 'semantic')
  assert.equal(result.error, true)
  assert.equal(result.code, 'unknown-language')
})

test('invalid layer returns protocol error payload', () => {
  const raw = wasm.tokenize('json', '{}', 'strict', 'nope')
  const result = JSON.parse(raw)
  assert.equal(result.error, true)
  assert.equal(result.code, 'invalid-layer')
})

test('json tokenize_json compatibility still works', () => {
  const result = JSON.parse(wasm.tokenize_json('{"a":1}', 'strict', 'semantic'))
  assert.equal(result.valid, true)
  assert.ok(result.tokens.some((t) => t.kind === 'property'))
})

test('two languages produce independent token streams', () => {
  const py = run('python', 'def f():\n    return 1\n', 'default', 'semantic')
  const html = run('html', '<div>hi</div>', 'default', 'semantic')
  assert.equal(py.language, 'python')
  assert.equal(html.language, 'html')
  assert.ok(py.tokens.some((t) => t.kind === 'keyword'))
  assert.ok(html.tokens.some((t) => t.kind === 'tag' || t.kind === 'text'))
  assertLossless(py, 'def f():\n    return 1\n')
  assertLossless(html, '<div>hi</div>')
})
