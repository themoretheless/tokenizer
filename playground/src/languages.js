/**
 * Playground language catalog. Keep ids in sync with `all-languages` registry
 * and `tests/language-cases.js`.
 */

/** @type {{ id: string, label: string, group: string, modes: string[], modeLabels?: Record<string, string>, sample: string }[]} */
export const LANGUAGES = [
  {
    id: 'json',
    label: 'JSON',
    group: 'data',
    modes: ['strict', 'jsonc'],
    modeLabels: { strict: 'Strict JSON', jsonc: 'JSONC' },
    sample: `{\n  "project": "tokenizer",\n  "unicode": "Тбилиси 😀",\n  "stable": true,\n  "versions": [1, 2, 3]\n}`,
  },
  {
    id: 'url',
    label: 'URL',
    group: 'data',
    modes: ['default'],
    sample: 'https://user:pass@example.com:8443/path?q=1&lang=ru#frag',
  },
  {
    id: 'yaml',
    label: 'YAML',
    group: 'data',
    modes: ['default'],
    sample: '# config\nname: Denis\nok: true\n',
  },
  {
    id: 'toml',
    label: 'TOML',
    group: 'data',
    modes: ['default'],
    sample: '# config\nname = "tok"\nflag = true\n',
  },
  {
    id: 'xml',
    label: 'XML',
    group: 'markup',
    modes: ['default'],
    sample: '<root a="1"><item>text</item></root>\n',
  },
  {
    id: 'html',
    label: 'HTML',
    group: 'markup',
    modes: ['default'],
    sample: '<div class="box">Hello <b>world</b></div>\n',
  },
  {
    id: 'css',
    label: 'CSS',
    group: 'markup',
    modes: ['default'],
    sample: 'body {\n  color: red;\n  /* comment */\n  content: "hi";\n}\n',
  },
  {
    id: 'markdown',
    label: 'Markdown',
    group: 'markup',
    modes: ['default'],
    sample: '# Title\n\nUse `code` and a fence:\n\n```js\nconst x = 1\n```\n',
  },
  {
    id: 'sql',
    label: 'SQL',
    group: 'query',
    modes: ['default'],
    sample: "-- users\nSELECT id, name\nFROM users\nWHERE active = 1;\n",
  },
  {
    id: 'mongo',
    label: 'MongoDB',
    group: 'query',
    modes: ['default'],
    sample: '// query\ndb.users.find({ active: true })\n',
  },
  {
    id: 'graphql',
    label: 'GraphQL',
    group: 'query',
    modes: ['default'],
    sample: '# demo\nquery Q {\n  user {\n    id\n    name\n  }\n}\n',
  },
  {
    id: 'javascript',
    label: 'JavaScript',
    group: 'scripting',
    modes: ['default'],
    sample: '// demo\nfunction add(a, b) {\n  return a + b;\n}\nconst s = "hi";\n',
  },
  {
    id: 'typescript',
    label: 'TypeScript',
    group: 'scripting',
    modes: ['default'],
    sample: '// demo\nfunction add(a: number, b: number): number {\n  return a + b;\n}\n',
  },
  {
    id: 'python',
    label: 'Python',
    group: 'scripting',
    modes: ['default'],
    sample: '# demo\ndef add(a, b):\n    return a + b\n\nprint("hi")\n',
  },
  {
    id: 'ruby',
    label: 'Ruby',
    group: 'scripting',
    modes: ['default'],
    sample: '# demo\ndef add(a, b)\n  a + b\nend\nputs "hi"\n',
  },
  {
    id: 'php',
    label: 'PHP',
    group: 'scripting',
    modes: ['default'],
    sample: '<?php\nfunction add($a, $b) {\n  return $a + $b;\n}\n',
  },
  {
    id: 'perl',
    label: 'Perl',
    group: 'scripting',
    modes: ['default'],
    sample: "# demo\nsub f {\n  my ($x) = @_;\n  return $x + 1;\n}\n",
  },
  {
    id: 'lua',
    label: 'Lua',
    group: 'scripting',
    modes: ['default'],
    sample: '-- demo\nfunction f(x)\n  return x + 1\nend\n',
  },
  {
    id: 'r',
    label: 'R',
    group: 'scripting',
    modes: ['default'],
    sample: '# demo\nf <- function(x) {\n  x + 1\n}\n',
  },
  {
    id: 'matlab',
    label: 'MATLAB',
    group: 'scripting',
    modes: ['default'],
    sample: '% demo\nif true\n  x = 1;\nend\n',
  },
  {
    id: 'julia',
    label: 'Julia',
    group: 'scripting',
    modes: ['default'],
    sample: '# demo\nfunction f(x)\n  x + 1\nend\n',
  },
  {
    id: 'dart',
    label: 'Dart',
    group: 'scripting',
    modes: ['default'],
    sample: 'void main() {\n  var s = "hi";\n  print(s);\n}\n',
  },
  {
    id: 'groovy',
    label: 'Groovy',
    group: 'scripting',
    modes: ['default'],
    sample: '// demo\ndef f(x) {\n  return x + 1\n}\n',
  },
  {
    id: 'bash',
    label: 'Bash',
    group: 'scripting',
    modes: ['default'],
    sample: '#!/bin/bash\nif [ "$1" = "ok" ]; then\n  echo "hi"\nfi\n',
  },
  {
    id: 'powershell',
    label: 'PowerShell',
    group: 'scripting',
    modes: ['default'],
    sample: '# script\nif ($true) {\n  Write-Host "hi"\n}\n',
  },
  {
    id: 'rust',
    label: 'Rust',
    group: 'systems',
    modes: ['default'],
    sample: 'fn main() {\n  let s = "hi";\n  println!("{s}");\n}\n',
  },
  {
    id: 'c',
    label: 'C',
    group: 'systems',
    modes: ['default'],
    sample: '#include <stdio.h>\nint main(void) {\n  int x = 1;\n  return x;\n}\n',
  },
  {
    id: 'cpp',
    label: 'C++',
    group: 'systems',
    modes: ['default'],
    sample: '#include <string>\nint main() {\n  auto s = std::string("hi");\n  return 0;\n}\n',
  },
  {
    id: 'go',
    label: 'Go',
    group: 'systems',
    modes: ['default'],
    sample: 'package main\n\nfunc main() {\n  s := "hi"\n  _ = s\n}\n',
  },
  {
    id: 'zig',
    label: 'Zig',
    group: 'systems',
    modes: ['default'],
    sample: '// demo\npub fn main() void {\n  const s = "hi";\n  _ = s;\n}\n',
  },
  {
    id: 'nim',
    label: 'Nim',
    group: 'systems',
    modes: ['default'],
    sample: '# demo\nproc f(x: int): int =\n  x + 1\n',
  },
  {
    id: 'dlang',
    label: 'D',
    group: 'systems',
    modes: ['default'],
    sample: '// demo\nint f(int x) {\n  return x + 1;\n}\n',
  },
  {
    id: 'swift',
    label: 'Swift',
    group: 'systems',
    modes: ['default'],
    sample: 'func main() {\n  let s = "hi"\n  print(s)\n}\n',
  },
  {
    id: 'objectivec',
    label: 'Objective-C',
    group: 'systems',
    modes: ['default'],
    sample: '@interface Foo : NSObject\n@end\n// comment\nNSString *s = @"hi";\n',
  },
  {
    id: 'assembly',
    label: 'Assembly',
    group: 'systems',
    modes: ['default'],
    sample: '; demo\nmov eax, 1\nret\n',
  },
  {
    id: 'fortran',
    label: 'Fortran',
    group: 'systems',
    modes: ['default'],
    sample: 'program main\n  ! comment\n  print *, "hi"\nend program main\n',
  },
  {
    id: 'java',
    label: 'Java',
    group: 'managed',
    modes: ['default'],
    sample: 'class Main {\n  public static void main(String[] args) {\n    System.out.println("hi");\n  }\n}\n',
  },
  {
    id: 'csharp',
    label: 'C#',
    group: 'managed',
    modes: ['default'],
    sample: 'class Program {\n  static void Main() {\n    var s = "hi";\n  }\n}\n',
  },
  {
    id: 'kotlin',
    label: 'Kotlin',
    group: 'managed',
    modes: ['default'],
    sample: 'fun main() {\n  val s = "hi"\n  println(s)\n}\n',
  },
  {
    id: 'scala',
    label: 'Scala',
    group: 'managed',
    modes: ['default'],
    sample: 'object Main {\n  def f(x: Int) = x + 1\n  val s = "hi"\n}\n',
  },
  {
    id: 'visualbasic',
    label: 'Visual Basic',
    group: 'managed',
    modes: ['default'],
    sample: "Module M\n  Sub Main()\n    Dim s As String = \"hi\"\n  End Sub\nEnd Module\n",
  },
  {
    id: 'delphi',
    label: 'Delphi',
    group: 'managed',
    modes: ['default'],
    sample: "program P;\nbegin\n  // comment\n  WriteLn('hi');\nend.\n",
  },
  {
    id: 'solidity',
    label: 'Solidity',
    group: 'managed',
    modes: ['default'],
    sample: '// demo\ncontract C {\n  function f() public {}\n}\n',
  },
  {
    id: 'haskell',
    label: 'Haskell',
    group: 'functional',
    modes: ['default'],
    sample: '-- demo\nf x = x + 1\n  where g = 1\n',
  },
  {
    id: 'elixir',
    label: 'Elixir',
    group: 'functional',
    modes: ['default'],
    sample: '# demo\ndefmodule M do\n  def f(x), do: x + 1\nend\n',
  },
  {
    id: 'erlang',
    label: 'Erlang',
    group: 'functional',
    modes: ['default'],
    sample: '% demo\nf(X) ->\n  case X of\n    1 -> ok\n  end.\n',
  },
  {
    id: 'clojure',
    label: 'Clojure',
    group: 'functional',
    modes: ['default'],
    sample: '; demo\n(defn f [x]\n  (+ x 1))\n',
  },
  {
    id: 'fsharp',
    label: 'F#',
    group: 'functional',
    modes: ['default'],
    sample: '// demo\nlet f x = x + 1\nlet s = "hi"\n',
  },
  {
    id: 'ocaml',
    label: 'OCaml',
    group: 'functional',
    modes: ['default'],
    sample: '(* demo *)\nlet f x = x + 1\nlet s = "hi"\n',
  },
  {
    id: 'lisp',
    label: 'Common Lisp',
    group: 'functional',
    modes: ['default'],
    sample: '; demo\n(defun f (x)\n  (+ x 1))\n',
  },
  {
    id: 'scheme',
    label: 'Scheme',
    group: 'functional',
    modes: ['default'],
    sample: '; demo\n(define (f x)\n  (+ x 1))\n',
  },
  {
    id: 'prolog',
    label: 'Prolog',
    group: 'functional',
    modes: ['default'],
    sample: '% demo\nparent(X, Y) :-\n  mother(X, Y).\n',
  },
  {
    id: 'cobol',
    label: 'COBOL',
    group: 'legacy',
    modes: ['default'],
    sample: '*> demo\nMOVE 1 TO X.\nDISPLAY Y.\n',
  },
  {
    id: 'ada',
    label: 'Ada',
    group: 'legacy',
    modes: ['default'],
    sample: '-- demo\nprocedure Main is\nbegin\n  null;\nend Main;\n',
  },
  {
    id: 'abap',
    label: 'ABAP',
    group: 'legacy',
    modes: ['default'],
    sample: '" demo\nDATA lv TYPE i.\nIF lv = 1.\n  WRITE lv.\nENDIF.\n',
  },
  {
    id: 'vhdl',
    label: 'VHDL',
    group: 'legacy',
    modes: ['default'],
    sample: '-- demo\nentity E is\nend entity;\n',
  },
  {
    id: 'verilog',
    label: 'Verilog',
    group: 'legacy',
    modes: ['default'],
    sample: '// demo\nmodule m;\n  wire x;\nendmodule\n',
  },
]

/** @type {{ id: string, label: string }[]} */
export const LANGUAGE_GROUPS = [
  { id: 'data', label: 'Data & Config' },
  { id: 'markup', label: 'Markup & Web' },
  { id: 'query', label: 'Query & API' },
  { id: 'scripting', label: 'Scripting & Shell' },
  { id: 'systems', label: 'Systems' },
  { id: 'managed', label: 'Managed & JVM/.NET' },
  { id: 'functional', label: 'Functional & Logic' },
  { id: 'legacy', label: 'Legacy & Hardware' },
]

const byId = Object.fromEntries(LANGUAGES.map((lang) => [lang.id, lang]))

export function languageMeta(id) {
  return byId[id] ?? null
}

export function defaultModeFor(id) {
  const meta = languageMeta(id)
  if (!meta) return 'default'
  return meta.modes[0] ?? 'default'
}

export function modeLabelFor(id, mode) {
  return languageMeta(id)?.modeLabels?.[mode] ?? (mode === 'default' ? 'Default' : mode)
}

export function groupLabelFor(id) {
  return LANGUAGE_GROUPS.find((group) => group.id === id)?.label ?? id
}

export function groupedLanguages(filter = '') {
  const needle = filter.trim().toLowerCase()
  const matches = needle
    ? LANGUAGES.filter((lang) => lang.label.toLowerCase().includes(needle) || lang.id.includes(needle))
    : LANGUAGES
  return LANGUAGE_GROUPS
    .map((group) => ({ ...group, languages: matches.filter((lang) => lang.group === group.id) }))
    .filter((group) => group.languages.length > 0)
}

export function sampleFor(id) {
  return languageMeta(id)?.sample ?? ''
}
