/**
 * Fixtures for every registered language: valid, incomplete, comments, unicode.
 * Used by playground WASM bridge tests.
 */

/** @typedef {{ name: string, source: string, mode?: string, layer?: string, expectValid?: boolean, expectKinds?: string[], minTokens?: number, maxDiagnostics?: number }} Case */

/** @type {Record<string, { modes: string[], cases: Case[] }>} */
export const LANGUAGE_CASES = {
  json: {
    modes: ['strict', 'jsonc'],
    cases: [
      { name: 'empty-object', source: '{}', mode: 'strict', expectValid: true, expectKinds: ['punctuation'] },
      { name: 'unicode-property', source: '{"city":"Тбилиси"}', mode: 'strict', expectValid: true, expectKinds: ['property', 'string'] },
      { name: 'array-numbers', source: '[1,2,3]', mode: 'strict', expectValid: true, expectKinds: ['number'] },
      { name: 'leading-zero-invalid', source: '{"n":01}', mode: 'strict', expectValid: false },
      { name: 'jsonc-comment', source: '{// c\n"a":1,}', mode: 'jsonc', expectValid: true, expectKinds: ['comment'] },
      { name: 'jsonc-rejected-in-strict', source: '{// c\n"a":1}', mode: 'strict', expectValid: false },
      { name: 'incomplete-object', source: '{"a":', mode: 'strict', expectValid: false, minTokens: 1 },
      { name: 'empty-source', source: '', mode: 'strict', minTokens: 0 },
    ],
  },
  url: {
    modes: ['default'],
    cases: [
      { name: 'https', source: 'https://example.com/path?q=1#f', expectKinds: ['u-scheme', 'u-host'] },
      { name: 'path-only', source: '/api/v1/items', expectKinds: ['u-path'] },
      { name: 'incomplete-scheme', source: 'https:', minTokens: 1 },
      { name: 'whitespace-diag', source: 'https://ex.com/a b', expectValid: false },
      { name: 'port-range', source: 'https://h:99999/', expectValid: false },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  xml: {
    modes: ['default'],
    cases: [
      { name: 'root', source: '<root a="1"/>', expectKinds: ['tag'] },
      { name: 'nested', source: '<a><b>x</b></a>', expectKinds: ['tag', 'text'] },
      { name: 'comment', source: '<!-- c --><e/>', expectKinds: ['comment'] },
      { name: 'unclosed', source: '<open>', expectValid: false, minTokens: 1 },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  html: {
    modes: ['default'],
    cases: [
      { name: 'div', source: '<div class="box">hi</div>', expectKinds: ['tag', 'text'] },
      { name: 'void-ish', source: '<br/>', expectKinds: ['tag'] },
      { name: 'comment', source: '<!--x--><p></p>', expectKinds: ['comment'] },
      { name: 'incomplete', source: '<div', expectValid: false, minTokens: 1 },
      { name: 'unicode', source: '<span>Москва</span>', expectKinds: ['text'] },
    ],
  },
  css: {
    modes: ['default'],
    cases: [
      { name: 'rule', source: 'body { color: red; }', expectKinds: ['identifier', 'punctuation'] },
      { name: 'comment', source: '/* c */ .x{}', expectKinds: ['comment'] },
      { name: 'string', source: 'a { content: "hi"; }', expectKinds: ['string'] },
      { name: 'incomplete', source: 'a { color:', minTokens: 1 },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  yaml: {
    modes: ['default'],
    cases: [
      { name: 'mapping', source: 'name: Denis\nage: 1\n', minTokens: 1 },
      { name: 'comment', source: '# hi\nkey: value\n', expectKinds: ['comment'] },
      { name: 'bool', source: 'ok: true\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  toml: {
    modes: ['default'],
    cases: [
      { name: 'key-value', source: 'name = "tok"\n', expectKinds: ['string'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'bool', source: 'flag = true\n', expectKinds: ['keyword'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  markdown: {
    modes: ['default'],
    cases: [
      { name: 'heading', source: '# Title\n\npara\n', expectKinds: ['comment'] },
      { name: 'fence', source: '```js\ncode\n```\n', minTokens: 1 },
      { name: 'inline-code', source: 'use `x` here\n', minTokens: 1 },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  sql: {
    modes: ['default'],
    cases: [
      { name: 'select', source: 'SELECT id FROM users WHERE x = 1;', expectKinds: ['keyword'] },
      { name: 'select-lower', source: 'select id from users;', expectKinds: ['keyword'] },
      { name: 'comment', source: '-- c\nSELECT 1;', expectKinds: ['comment'] },
      { name: 'string', source: "SELECT 'hi';", expectKinds: ['string'] },
      { name: 'incomplete', source: 'SELECT FROM', minTokens: 1 },
    ],
  },
  mongo: {
    modes: ['default'],
    cases: [
      { name: 'find', source: 'db.users.find({ a: 1 })', minTokens: 1 },
      { name: 'comment', source: '// c\nfind({})', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  bash: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'if [ "$x" = 1 ]; then echo hi; fi\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# note\necho ok\n', expectKinds: ['comment'] },
      { name: 'string', source: 'echo "hello"\n', expectKinds: ['string'] },
      { name: 'incomplete', source: 'if then', minTokens: 1 },
    ],
  },
  powershell: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'if ($true) { Write-Host "x" }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nGet-Item .\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  javascript: {
    modes: ['default'],
    cases: [
      { name: 'function', source: 'function add(a, b) { return a + b; }', expectKinds: ['keyword', 'function'] },
      { name: 'const', source: 'const x = 42;\n', expectKinds: ['keyword', 'number'] },
      { name: 'comment', source: '// c\nlet y = 1;', expectKinds: ['comment'] },
      { name: 'string', source: 'const s = "hi";', expectKinds: ['string'] },
      { name: 'incomplete', source: 'function (', minTokens: 1 },
      { name: 'unicode', source: 'const city = "Тбилиси";', expectKinds: ['string'] },
    ],
  },
  typescript: {
    modes: ['default'],
    cases: [
      { name: 'interface', source: 'interface U { id: number }\n', expectKinds: ['keyword'] },
      { name: 'function', source: 'function f(x: string): number { return 1; }', expectKinds: ['keyword'] },
      { name: 'type-keyword', source: 'type Id = string;\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '/* c */ const a = 1;', expectKinds: ['comment'] },
    ],
  },
  python: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def greet(name):\n    return f"hi {name}"\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hello"\n', expectKinds: ['string'] },
      { name: 'class', source: 'class A:\n    pass\n', expectKinds: ['keyword'] },
      { name: 'incomplete', source: 'def f(', minTokens: 1 },
      { name: 'unicode', source: 'msg = "Привет"\n', expectKinds: ['string'] },
    ],
  },
  java: {
    modes: ['default'],
    cases: [
      { name: 'class', source: 'class A { int x = 1; }', expectKinds: ['keyword'] },
      { name: 'method', source: 'public int f() { return 0; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nint a;', expectKinds: ['comment'] },
      { name: 'string', source: 'String s = "x";', expectKinds: ['string'] },
    ],
  },
  csharp: {
    modes: ['default'],
    cases: [
      { name: 'class', source: 'class A { public int X { get; set; } }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar x = 1;', expectKinds: ['comment'] },
      { name: 'string', source: 'var s = "hi";', expectKinds: ['string'] },
    ],
  },
  go: {
    modes: ['default'],
    cases: [
      { name: 'func', source: 'func main() { x := 1 }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar x int', expectKinds: ['comment'] },
      { name: 'string', source: 's := "hi"', expectKinds: ['string'] },
    ],
  },
  php: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'function f($x) { return $x; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\n$x = 1;', expectKinds: ['comment'] },
      { name: 'string', source: '$s = "hi";', expectKinds: ['string'] },
    ],
  },
  ruby: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def f(x)\n  x\nend\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
    ],
  },
  c: {
    modes: ['default'],
    cases: [
      { name: 'main', source: 'int main(void) { return 0; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '/* c */ int x;', expectKinds: ['comment'] },
      { name: 'string', source: 'char *s = "hi";', expectKinds: ['string'] },
      { name: 'incomplete', source: 'int main(', minTokens: 1 },
    ],
  },
  cpp: {
    modes: ['default'],
    cases: [
      { name: 'class', source: 'class A { public: int x; };', expectKinds: ['keyword'] },
      { name: 'template-ish', source: 'int f() { return 1; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nint x;', expectKinds: ['comment'] },
    ],
  },
  rust: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'fn main() { let x = 1; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nlet y = 2;', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi";', expectKinds: ['string'] },
      { name: 'incomplete', source: 'fn main(', minTokens: 1 },
    ],
  },
  kotlin: {
    modes: ['default'],
    cases: [
      { name: 'fun', source: 'fun main() { val x = 1 }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nval y = 2', expectKinds: ['comment'] },
      { name: 'string', source: 'val s = "hi"', expectKinds: ['string'] },
    ],
  },
  swift: {
    modes: ['default'],
    cases: [
      { name: 'func', source: 'func f() -> Int { return 1 }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nlet x = 1', expectKinds: ['comment'] },
      { name: 'string', source: 'let s = "hi"', expectKinds: ['string'] },
    ],
  },
  dart: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'void main() { var x = 1; }', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar y = 2;', expectKinds: ['comment'] },
      { name: 'string', source: 'var s = "hi";', expectKinds: ['string'] },
    ],
  },
  r: {
    modes: ['default'],
    cases: [
      { name: 'fn', source: 'f <- function(x) { x + 1 }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx <- 1\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  visualbasic: {
    modes: ['default'],
    cases: [
      { name: 'sub', source: 'Sub Main()\nEnd Sub\n', expectKinds: ['keyword'] },
      { name: 'comment', source: "' c\nDim x As Integer\n", expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  fortran: {
    modes: ['default'],
    cases: [
      { name: 'program', source: 'program hello\nend program hello\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '! c\ninteger :: x\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  matlab: {
    modes: ['default'],
    cases: [
      { name: 'if', source: 'if x > 0\n  y = 1;\nend\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '% c\nx = 1;\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  delphi: {
    modes: ['default'],
    cases: [
      { name: 'begin', source: 'begin\n  x := 1;\nend.\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nvar x: Integer;\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
  scala: {
    modes: ['default'],
    cases: [
      { name: 'def', source: 'def f(x: Int): Int = x + 1\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '// c\nval y = 2\n', expectKinds: ['comment'] },
      { name: 'string', source: 'val s = "hi"\n', expectKinds: ['string'] },
    ],
  },
  lua: {
    modes: ['default'],
    cases: [
      { name: 'function', source: 'function f(x) return x end\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '-- c\nlocal x = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
    ],
  },
  perl: {
    modes: ['default'],
    cases: [
      { name: 'sub', source: 'sub f { my $x = shift; return $x; }\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nmy $y = 1;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'my $s = "hi";\n', expectKinds: ['string'] },
    ],
  },
  objectivec: {
    modes: ['default'],
    cases: [
      { name: 'interface', source: '@interface A : NSObject\n@end\n', expectKinds: ['class', 'identifier'] },
      { name: 'comment', source: '// c\nint x;\n', expectKinds: ['comment'] },
      { name: 'string', source: 'NSString *s = @"hi";\n', minTokens: 1 },
    ],
  },
  julia: {
    modes: ['default'],
    cases: [
      { name: 'function', source: 'function f(x)\n  x + 1\nend\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '# c\nx = 1\n', expectKinds: ['comment'] },
      { name: 'string', source: 's = "hi"\n', expectKinds: ['string'] },
    ],
  },
  assembly: {
    modes: ['default'],
    cases: [
      { name: 'mov', source: 'mov eax, 1\nret\n', expectKinds: ['keyword'] },
      { name: 'comment', source: '; c\nnop\n', expectKinds: ['comment'] },
      { name: 'empty', source: '', minTokens: 0 },
    ],
  },
}

/** Every language id that must be present in the WASM build. */
export const ALL_LANGUAGE_IDS = Object.keys(LANGUAGE_CASES)

export function defaultMode(language) {
  const entry = LANGUAGE_CASES[language]
  if (!entry) return 'default'
  if (language === 'json') return 'strict'
  return entry.modes[0] ?? 'default'
}
