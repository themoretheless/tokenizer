// Rust spans are UTF-8 byte offsets; textarea selections are UTF-16 offsets.
export function utf16OffsetFromByte(source, byteOffset) {
  let bytes = 0
  let units = 0
  const encoder = new TextEncoder()
  for (const character of source) {
    const size = encoder.encode(character).length
    if (bytes + size > byteOffset) break
    bytes += size
    units += character.length
  }
  return units
}

export function sourcePosition(source, utf16Offset) {
  const prefix = source.slice(0, utf16Offset)
  const lines = prefix.split('\n')
  return { line: lines.length, column: [...lines[lines.length - 1]].length + 1 }
}

export function bytePosition(source, byteOffset) {
  return sourcePosition(source, utf16OffsetFromByte(source, byteOffset))
}
