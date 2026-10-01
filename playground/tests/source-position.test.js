import test from 'node:test'
import assert from 'node:assert/strict'
import { bytePosition, sourcePosition, utf16OffsetFromByte } from '../src/source-position.js'

test('Rust spans map to editor selection after Cyrillic and emoji', () => {
  const source = '// привет🙂\nconst x = "я"; missing'
  const start = new TextEncoder().encode(source.split('missing')[0]).length
  const offset = utf16OffsetFromByte(source, start)
  assert.equal(source.slice(offset), 'missing')
  assert.deepEqual(bytePosition(source, start), {line: 2, column: 16})
  assert.deepEqual(sourcePosition(source, offset), {line: 2, column: 16})
})

test('positions count Unicode scalar values and clamp incomplete byte spans', () => {
  assert.deepEqual(bytePosition('я🙂x', 6), {line: 1, column: 3})
  assert.equal(utf16OffsetFromByte('я🙂x', 3), 1)
  assert.equal(utf16OffsetFromByte('я🙂x', 999), 4)
  assert.equal(utf16OffsetFromByte('я🙂x', -1), 0)
  assert.deepEqual(bytePosition('x\n', 2), {line: 2, column: 1})
})
