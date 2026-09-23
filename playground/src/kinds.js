/**
 * Token kind presentation map for every kind emitted by the engines
 * (verified against the wasm output of all languages + cases).
 * Unknown kinds fall back to a stable hashed color so no engine renders dark.
 */

const COMMENT = '#66757d'
const STRING = '#f5cf8d'
const NUMBER = '#bda6ff'
const KEYWORD = '#ff9d66'
const IDENT = '#dfe7ec'
const MEMBER = '#75e9d0'
const TYPE = '#7ec3e8'
const PUNCT = '#93a0a7'
const OPERATOR = '#bda6ff'
const TEXT = '#cfd8dd'
const ERROR = '#ff657a'
const URL_PART = '#7ee8a2'

export const KIND_COLORS = {
  comment: COMMENT,
  'line-comment': COMMENT,
  'block-comment': COMMENT,
  string: STRING,
  number: NUMBER,
  keyword: KEYWORD,
  boolean: KEYWORD,
  true: KEYWORD,
  false: KEYWORD,
  null: KEYWORD,
  identifier: IDENT,
  variable: IDENT,
  property: MEMBER,
  attribute: MEMBER,
  tag: MEMBER,
  function: MEMBER,
  type: TYPE,
  class: TYPE,
  punctuation: PUNCT,
  comma: PUNCT,
  colon: PUNCT,
  'left-brace': PUNCT,
  'right-brace': PUNCT,
  'left-bracket': PUNCT,
  'right-bracket': PUNCT,
  delimiter: PUNCT,
  operator: OPERATOR,
  text: TEXT,
  invalid: ERROR,
  error: ERROR,
  whitespace: 'inherit',
  bom: 'inherit',
}

export const KIND_FAMILIES = [...new Set(Object.keys(KIND_COLORS))]

export const PUNCTUATION_KINDS = new Set([
  'punctuation', 'comma', 'colon', 'left-brace', 'right-brace',
  'left-bracket', 'right-bracket', 'delimiter', 'operator', 'u-sep',
])

export const INVISIBLE_KINDS = new Set(['whitespace', 'bom'])

const FALLBACK_PALETTE = ['#7ec3e8', '#d8b4fe', '#86d7a6', '#e8c07e', '#e88ea0', '#8fd8d2', '#c7d87e']

export function kindColor(kind) {
  const exact = KIND_COLORS[kind]
  if (exact) return exact
  if (kind.startsWith('u-')) return kind === 'u-sep' ? PUNCT : URL_PART
  if (kind.includes('comment')) return COMMENT
  if (kind.includes('string')) return STRING
  if (kind.includes('number')) return NUMBER
  if (kind.includes('keyword')) return KEYWORD
  if (kind.includes('error') || kind.includes('invalid')) return ERROR
  let hash = 0
  for (let i = 0; i < kind.length; i += 1) hash = (hash * 31 + kind.charCodeAt(i)) >>> 0
  return FALLBACK_PALETTE[hash % FALLBACK_PALETTE.length]
}

export function kindCounts(tokens) {
  const counts = new Map()
  for (const token of tokens) counts.set(token.kind, (counts.get(token.kind) ?? 0) + 1)
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([kind, count]) => ({ kind, count }))
}
