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
  newline: 'inherit',
  'line-break': 'inherit',
  // TOML lexical categories
  'bare-key': MEMBER,
  'basic-string': STRING,
  'literal-string': STRING,
  'multi-line-basic-string': STRING,
  'multi-line-literal-string': STRING,
  integer: NUMBER,
  float: NUMBER,
  'hex-integer': NUMBER,
  'octal-integer': NUMBER,
  'binary-integer': NUMBER,
  inf: NUMBER,
  nan: NUMBER,
  'offset-date-time': TYPE,
  'local-date-time': TYPE,
  'local-date': TYPE,
  'local-time': TYPE,
  equals: PUNCT,
  dot: PUNCT,
  // YAML node vocabulary
  'document-start': KEYWORD,
  'document-end': KEYWORD,
  directive: KEYWORD,
  'block-entry': PUNCT,
  'key-indicator': MEMBER,
  'value-indicator': PUNCT,
  'flow-sequence-start': PUNCT,
  'flow-sequence-end': PUNCT,
  'flow-mapping-start': PUNCT,
  'flow-mapping-end': PUNCT,
  'flow-entry': PUNCT,
  anchor: MEMBER,
  alias: MEMBER,
  'block-scalar-header': OPERATOR,
  'single-quoted-scalar': STRING,
  'double-quoted-scalar': STRING,
  'plain-scalar': TEXT,
  // Markdown block + inline vocabulary
  'heading-marker': KEYWORD,
  'heading-text': TYPE,
  'setext-underline': KEYWORD,
  'thematic-break': PUNCT,
  'blockquote-marker': OPERATOR,
  'list-marker': KEYWORD,
  'code-fence-marker': PUNCT,
  'fence-info': TYPE,
  'code-block-line': STRING,
  'table-delimiter': PUNCT,
  'table-pipe': PUNCT,
  'table-cell': TEXT,
  'link-label': MEMBER,
  'link-destination': URL_PART,
  'front-matter-delimiter': PUNCT,
  'front-matter': COMMENT,
  'html-block': MEMBER,
  'html-inline': MEMBER,
  'hard-break': 'inherit',
  strong: KEYWORD,
  emphasis: KEYWORD,
  strikethrough: KEYWORD,
  'code-span': STRING,
  'link-text': MEMBER,
  'image-marker': KEYWORD,
  autolink: URL_PART,
  'email-autolink': URL_PART,
  'footnote-ref': MEMBER,
  'footnote-definition-label': MEMBER,
  // CSS selector / declaration vocabulary
  semicolon: PUNCT,
  'left-paren': PUNCT,
  'right-paren': PUNCT,
  'type-selector': TYPE,
  'class-selector': MEMBER,
  'id-selector': TYPE,
  'universal-selector': KEYWORD,
  'attribute-selector': MEMBER,
  'pseudo-class': OPERATOR,
  'pseudo-element': OPERATOR,
  'nesting-selector': KEYWORD,
  important: KEYWORD,
  'at-rule': KEYWORD,
  'at-rule-prelude-text': TEXT,
  color: STRING,
  percentage: NUMBER,
  unit: OPERATOR,
  value: TEXT,
}

export const KIND_FAMILIES = [...new Set(Object.keys(KIND_COLORS))]

export const PUNCTUATION_KINDS = new Set([
  'punctuation', 'comma', 'colon', 'left-brace', 'right-brace',
  'left-bracket', 'right-bracket', 'delimiter', 'operator', 'u-sep',
  'equals', 'dot', 'value-indicator', 'key-indicator', 'block-entry',
  'flow-sequence-start', 'flow-sequence-end', 'flow-mapping-start',
  'flow-mapping-end', 'flow-entry', 'block-scalar-header',
  'semicolon', 'left-paren', 'right-paren', 'table-pipe', 'table-delimiter',
  'code-fence-marker', 'front-matter-delimiter', 'thematic-break',
])

export const INVISIBLE_KINDS = new Set(['whitespace', 'bom', 'newline', 'line-break', 'hard-break'])

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
