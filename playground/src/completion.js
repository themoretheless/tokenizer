export function completionRange(source, start, end = start) {
  if (start !== end) return { start, end, prefix: source.slice(start, end), source }
  const prefix = source.slice(0, start).match(/[\p{L}\p{N}_$]*$/u)[0]
  const suffix = source.slice(end).match(/^[\p{L}\p{N}_$]*/u)[0]
  return { start: start - prefix.length, end: end + suffix.length, prefix, source }
}

export function applyCompletion(source, range, item) {
  if (range.source !== source) return null
  const tail = source.slice(range.end)
  const hasCall = /^\s*\(/u.test(tail)
  const callable = item.kind !== 'binding' && item.kind !== 'field'
  const insert = item.name + (callable && !hasCall ? '()' : '')
  const cursor = range.start + insert.length - (callable && !hasCall && item.maxArgs ? 1 : 0)
  return { source: source.slice(0, range.start) + insert + tail, cursor }
}

// Byte intervals come from the same lexical pass as definition navigation.
export function completionCandidates(source, range, payload) {
  if (range.source !== source) return []
  const memberAccess = source.slice(0, range.start).match(/(?<![\p{L}\p{N}_$.])([\p{L}_$][\p{L}\p{N}_$]*(?:\s*\.\s*[\p{L}_$][\p{L}\p{N}_$]*)*)\s*\.\s*$/u)
  const afterDot = /\.\s*$/u.test(source.slice(0, range.start))
  const offset = new TextEncoder().encode(source.slice(0, range.start)).length
  const names = new Map()
  const locals = (payload?.bindings ?? [])
    .filter(item => item.start <= offset && offset < item.end)
    .sort((a, b) => b.depth - a.depth || b.start - a.start)
  if (afterDot) {
    if (Array.isArray(payload?.memberCompletions)) {
      const receiver = payload.memberCompletions.find(item => item.start === offset)
      return (receiver?.members ?? []).filter(name => name.startsWith(range.prefix))
        .map(name => ({name, kind: 'field'}))
    }
    if (!memberAccess || /\.\s*$/u.test(source.slice(0, memberAccess.index))) return []
    const [root, ...path] = memberAccess[1].split(/\s*\.\s*/u)
    const binding = locals.find(item => item.name === root)
    const members = path.length ? binding?.memberPaths?.[path.join('.')] : binding?.members
    return (members ?? []).filter(name => name.startsWith(range.prefix))
      .map(name => ({ name, kind: 'field' }))
  }
  for (const item of [...locals, ...(payload?.builtins ?? [])]) {
    if (!names.has(item.name)) names.set(item.name, item)
  }
  return [...names.values()].filter(item => item.name.startsWith(range.prefix))
    .sort((a, b) => a.name.localeCompare(b.name))
}
