import { runCatalog } from './tokenizer.js'

/// Curated engine sets, mirroring `Preset` in the Rust core.
export const PRESETS = [
  { id: 'top20', label: 'Top 20 languages' },
  { id: 'next20', label: 'Next 20 languages' },
  { id: 'formats', label: 'Top 20 formats' },
]

/// Kinds the generated fullkit engines share, measured as the union over the
/// semantic layer of python, rust, go, java, c, lua, cobol, zig, haskell,
/// verilog, sql and bash. A format engine that only ever emits these has no
/// format-aware vocabulary yet.
export const GENERIC_KINDS = new Set([
  'class', 'comment', 'function', 'identifier', 'keyword', 'number',
  'punctuation', 'string', 'type', 'variable', 'whitespace',
])

let catalogPromise

/// Registry descriptors keyed by engine id, loaded once per page.
export function loadCatalog() {
  if (!catalogPromise) {
    catalogPromise = runCatalog()
      .then((payload) => Object.fromEntries(payload.engines.map((engine) => [engine.id, engine])))
      .catch((error) => {
        catalogPromise = undefined
        throw error
      })
  }
  return catalogPromise
}

/// Vocabulary depth of one tokenization: how many kinds the engine actually
/// emitted, and how many of them fall outside the generic set.
export function depthOf(tokens) {
  const kinds = new Set(tokens.map((token) => token.kind))
  const specific = [...kinds].filter((kind) => !GENERIC_KINDS.has(kind))
  return { kinds: kinds.size, specific: specific.length, specificKinds: specific.sort() }
}

/// Capability set as a short human label, most complete first.
export function depthLabel(engine) {
  if (!engine) return 'unknown engine'
  const caps = new Set(engine.capabilities)
  if (caps.has('navigate') || caps.has('cst') || caps.has('visitor')) return 'document engine'
  if (caps.has('parse')) return 'recovering parser'
  if (caps.has('validate')) return 'structure validator'
  return 'lexer only'
}

export function presetOf(engine) {
  return PRESETS.find((item) => engine?.presets?.includes(item.id)) ?? null
}
