/**
 * Playground language catalog. Keep ids in sync with `all-languages` registry
 * and `tests/language-cases.js`.
 */

/** @type {{ id: string, label: string, modes: string[], sample: string }[]} */
export const LANGUAGES = [
  {
    id: 'json',
    label: 'JSON',
    modes: ['strict', 'jsonc'],
    sample: `{\n  "project": "tokenizer",\n  "unicode": "Тбилиси 😀",\n  "stable": true,\n  "versions": [1, 2, 3]\n}`,
  },
  {
    id: 'url',
    label: 'URL',
    modes: ['default'],
    sample: 'https://user:pass@example.com:8443/path?q=1&lang=ru#frag',
  },
  {
    id: 'xml',
    label: 'XML',
    modes: ['default'],
    sample: '<root a="1"><item>text</item></root>\n',
  },
  {
    id: 'html',
    label: 'HTML',
    modes: ['default'],
    sample: '<div class="box">Hello <b>world</b></div>\n',
  },
  {
    id: 'css',
    label: 'CSS',
    modes: ['default'],
    sample: 'body {\n  color: red;\n  /* comment */\n  content: "hi";\n}\n',
  },
  {
    id: 'yaml',
    label: 'YAML',
    modes: ['default'],
    sample: '# config\nname: Denis\nok: true\n',
  },
  {
    id: 'toml',
    label: 'TOML',
    modes: ['default'],
    sample: '# config\nname = "tok"\nflag = true\n',
  },
  {
    id: 'markdown',
    label: 'Markdown',
    modes: ['default'],
    sample: '# Title\n\nUse `code` and a fence:\n\n```js\nconst x = 1\n```\n',
  },
  {
    id: 'sql',
    label: 'SQL',
    modes: ['default'],
    sample: "-- users\nSELECT id, name\nFROM users\nWHERE active = 1;\n",
  },
  {
    id: 'mongo',
    label: 'MongoDB',
    modes: ['default'],
    sample: '// query\ndb.users.find({ active: true })\n',
  },
  {
    id: 'bash',
    label: 'Bash',
    modes: ['default'],
    sample: '#!/bin/bash\nif [ "$1" = "ok" ]; then\n  echo "hi"\nfi\n',
  },
  {
    id: 'powershell',
    label: 'PowerShell',
    modes: ['default'],
    sample: '# script\nif ($true) {\n  Write-Host "hi"\n}\n',
  },
  {
    id: 'javascript',
    label: 'JavaScript',
    modes: ['default'],
    sample: '// demo\nfunction add(a, b) {\n  return a + b;\n}\nconst s = "hi";\n',
  },
  {
    id: 'typescript',
    label: 'TypeScript',
    modes: ['default'],
    sample: '// demo\nfunction add(a: number, b: number): number {\n  return a + b;\n}\n',
  },
  {
    id: 'python',
    label: 'Python',
    modes: ['default'],
    sample: '# demo\ndef add(a, b):\n    return a + b\n\nprint("hi")\n',
  },
  {
    id: 'java',
    label: 'Java',
    modes: ['default'],
    sample: 'class Main {\n  public static void main(String[] args) {\n    System.out.println("hi");\n  }\n}\n',
  },
  {
    id: 'csharp',
    label: 'C#',
    modes: ['default'],
    sample: 'class Program {\n  static void Main() {\n    var s = "hi";\n  }\n}\n',
  },
  {
    id: 'go',
    label: 'Go',
    modes: ['default'],
    sample: 'package main\n\nfunc main() {\n  s := "hi"\n  _ = s\n}\n',
  },
  {
    id: 'php',
    label: 'PHP',
    modes: ['default'],
    sample: '<?php\nfunction add($a, $b) {\n  return $a + $b;\n}\n',
  },
  {
    id: 'ruby',
    label: 'Ruby',
    modes: ['default'],
    sample: '# demo\ndef add(a, b)\n  a + b\nend\nputs "hi"\n',
  },
  {
    id: 'c',
    label: 'C',
    modes: ['default'],
    sample: '#include <stdio.h>\nint main(void) {\n  int x = 1;\n  return x;\n}\n',
  },
  {
    id: 'cpp',
    label: 'C++',
    modes: ['default'],
    sample: '#include <string>\nint main() {\n  auto s = std::string("hi");\n  return 0;\n}\n',
  },
  {
    id: 'rust',
    label: 'Rust',
    modes: ['default'],
    sample: 'fn main() {\n  let s = "hi";\n  println!("{s}");\n}\n',
  },
  {
    id: 'kotlin',
    label: 'Kotlin',
    modes: ['default'],
    sample: 'fun main() {\n  val s = "hi"\n  println(s)\n}\n',
  },
  {
    id: 'swift',
    label: 'Swift',
    modes: ['default'],
    sample: 'func main() {\n  let s = "hi"\n  print(s)\n}\n',
  },
  {
    id: 'dart',
    label: 'Dart',
    modes: ['default'],
    sample: 'void main() {\n  var s = "hi";\n  print(s);\n}\n',
  },
  {
    id: 'r',
    label: 'R',
    modes: ['default'],
    sample: '# demo\nf <- function(x) {\n  x + 1\n}\n',
  },
  {
    id: 'visualbasic',
    label: 'Visual Basic',
    modes: ['default'],
    sample: "Module M\n  Sub Main()\n    Dim s As String = \"hi\"\n  End Sub\nEnd Module\n",
  },
  {
    id: 'fortran',
    label: 'Fortran',
    modes: ['default'],
    sample: 'program main\n  ! comment\n  print *, "hi"\nend program main\n',
  },
  {
    id: 'matlab',
    label: 'MATLAB',
    modes: ['default'],
    sample: '% demo\nif true\n  x = 1;\nend\n',
  },
  {
    id: 'delphi',
    label: 'Delphi',
    modes: ['default'],
    sample: "program P;\nbegin\n  // comment\n  WriteLn('hi');\nend.\n",
  },
  {
    id: 'scala',
    label: 'Scala',
    modes: ['default'],
    sample: 'object Main {\n  def f(x: Int) = x + 1\n  val s = "hi"\n}\n',
  },
  {
    id: 'lua',
    label: 'Lua',
    modes: ['default'],
    sample: '-- demo\nfunction f(x)\n  return x + 1\nend\n',
  },
  {
    id: 'perl',
    label: 'Perl',
    modes: ['default'],
    sample: "# demo\nsub f {\n  my ($x) = @_;\n  return $x + 1;\n}\n",
  },
  {
    id: 'objectivec',
    label: 'Objective-C',
    modes: ['default'],
    sample: '@interface Foo : NSObject\n@end\n// comment\nNSString *s = @"hi";\n',
  },
  {
    id: 'julia',
    label: 'Julia',
    modes: ['default'],
    sample: '# demo\nfunction f(x)\n  x + 1\nend\n',
  },
  {
    id: 'assembly',
    label: 'Assembly',
    modes: ['default'],
    sample: '; demo\nmov eax, 1\nret\n',
  },
  {
    id: 'groovy',
    label: 'Groovy',
    modes: ['default'],
    sample: '// demo\ndef f(x) {\n  return x + 1\n}\n',
  },
  {
    id: 'haskell',
    label: 'Haskell',
    modes: ['default'],
    sample: '-- demo\nf x = x + 1\n  where g = 1\n',
  },
  {
    id: 'elixir',
    label: 'Elixir',
    modes: ['default'],
    sample: '# demo\ndefmodule M do\n  def f(x), do: x + 1\nend\n',
  },
  {
    id: 'erlang',
    label: 'Erlang',
    modes: ['default'],
    sample: '% demo\nf(X) ->\n  case X of\n    1 -> ok\n  end.\n',
  },
  {
    id: 'clojure',
    label: 'Clojure',
    modes: ['default'],
    sample: '; demo\n(defn f [x]\n  (+ x 1))\n',
  },
  {
    id: 'fsharp',
    label: 'F#',
    modes: ['default'],
    sample: '// demo\nlet f x = x + 1\nlet s = "hi"\n',
  },
  {
    id: 'ocaml',
    label: 'OCaml',
    modes: ['default'],
    sample: '(* demo *)\nlet f x = x + 1\nlet s = "hi"\n',
  },
  {
    id: 'lisp',
    label: 'Common Lisp',
    modes: ['default'],
    sample: '; demo\n(defun f (x)\n  (+ x 1))\n',
  },
  {
    id: 'scheme',
    label: 'Scheme',
    modes: ['default'],
    sample: '; demo\n(define (f x)\n  (+ x 1))\n',
  },
  {
    id: 'solidity',
    label: 'Solidity',
    modes: ['default'],
    sample: '// demo\ncontract C {\n  function f() public {}\n}\n',
  },
  {
    id: 'zig',
    label: 'Zig',
    modes: ['default'],
    sample: '// demo\npub fn main() void {\n  const s = "hi";\n  _ = s;\n}\n',
  },
  {
    id: 'nim',
    label: 'Nim',
    modes: ['default'],
    sample: '# demo\nproc f(x: int): int =\n  x + 1\n',
  },
  {
    id: 'dlang',
    label: 'D',
    modes: ['default'],
    sample: '// demo\nint f(int x) {\n  return x + 1;\n}\n',
  },
  {
    id: 'cobol',
    label: 'COBOL',
    modes: ['default'],
    sample: '*> demo\nMOVE 1 TO X.\nDISPLAY Y.\n',
  },
  {
    id: 'ada',
    label: 'Ada',
    modes: ['default'],
    sample: '-- demo\nprocedure Main is\nbegin\n  null;\nend Main;\n',
  },
  {
    id: 'prolog',
    label: 'Prolog',
    modes: ['default'],
    sample: '% demo\nparent(X, Y) :-\n  mother(X, Y).\n',
  },
  {
    id: 'abap',
    label: 'ABAP',
    modes: ['default'],
    sample: '" demo\nDATA lv TYPE i.\nIF lv = 1.\n  WRITE lv.\nENDIF.\n',
  },
  {
    id: 'vhdl',
    label: 'VHDL',
    modes: ['default'],
    sample: '-- demo\nentity E is\nend entity;\n',
  },
  {
    id: 'verilog',
    label: 'Verilog',
    modes: ['default'],
    sample: '// demo\nmodule m;\n  wire x;\nendmodule\n',
  },
  {
    id: 'graphql',
    label: 'GraphQL',
    modes: ['default'],
    sample: '# demo\nquery Q {\n  user {\n    id\n    name\n  }\n}\n',
  },
]

const byId = Object.fromEntries(LANGUAGES.map((lang) => [lang.id, lang]))

export function languageMeta(id) {
  return byId[id] ?? null
}

export function defaultModeFor(id) {
  const meta = languageMeta(id)
  if (!meta) return 'default'
  if (id === 'json') return 'strict'
  return meta.modes[0] ?? 'default'
}

export function sampleFor(id) {
  return languageMeta(id)?.sample ?? ''
}
