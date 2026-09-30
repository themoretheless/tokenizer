# themoretheless-tokenizer-rush

Rush editor engine with a dedicated lossless lexer, recovering parser and
borrowing syntax tree. It checks the grammar below and reports syntax errors,
including in both host token layers. It does not execute programs, resolve names,
infer types, check return types, or implement process/stream semantics.

## Basic syntax

Rush uses `.r`, case-sensitive names and keywords, `//` line comments,
non-nested `/* ... */` comments, and `return`. R also uses `.r`; hosts should
select the `rush` language id explicitly when an extension is ambiguous.

A newline or `;` ends a simple statement. Newlines inside parentheses, lists,
and maps can continue an expression. A bare `return` ends at the newline.
Strings use single or double quotes, with backslash escapes, and cannot cross
an unescaped line ending. Literal text is preserved; escapes are not evaluated.
Numbers are decimal integers or decimals with optional decimal exponents.
Names support Unicode letters/numbers and underscores, optionally prefixed by
`$`; bare `$` is invalid.

Blocks use `{ ... }` or increased space indentation on the next line (an
optional `:` precedes an indented body). Each sibling statement must have the
same indentation. Blank/comment-only lines do not end a block; tabs in
indentation are rejected. A closing brace ends a brace block independently of
indentation. Nested functions start a new loop-control context.

```text
fn add(a: int, b: int) -> int {
    return a + b
}

fn greet who: str -> str
    if who == "":
        return "hello"
    return who

let values: list[int] = [1, 2, 3]
const scale = 2
for item in values:
    print(item * scale)
```

Parenthesized parameters use commas; bare parameters use spaces or commas.
Each parameter can have a type. Function return types follow `->`.
Type syntax is `Name` or `Name[Type, ...]`; type names are not resolved.
`str` and `f64` are the canonical spellings; `string` and `float` remain accepted
legacy spellings. `Geometry`, `Row`, `T`, `print`, `where`, `select`, `count`,
`show`, `run`, `param` and `assert` are ordinary names, not special parser rules.

## Statements and expressions

- `let` / `const name [: Type] = expression`; initializers are required.
- Functions, `if` / `else if` / `else`, `while`, and `for name in expression`.
  `foreach` is retained as an alias of `for`.
- `return [expression]` inside a function; `yield expression` inside a function
  or loop; `break` / `continue` inside a loop.
- Names, decimal numbers, strings, `true`, `false`, `null`, lists, maps,
  calls, member access and indexing. A map uses `{key: value, ...}`.
- Assignment to a name, member or index with `=`, `+=`, `-=`, `*=`, `/=`, `%=`.
  Assignment is a syntax node; const mutation and type compatibility require a
  later semantic pass.
- Precedence from low to high: assignment; pipeline; `or`/`||`; `and`/`&&`;
  comparisons; `+ -`; `* / %`; unary `+ - ! not`; `**`; calls/member/index.
  Assignment and exponentiation associate right; other binary operators left.
- Parenthesized calls use comma-separated arguments. Command-style calls such
  as `echo "hello"` use whitespace-separated arguments on the same line,
  beginning with a name or scalar literal. Use parenthesized calls for list,
  map and unary arguments.
- `async`, `await` and `import` are reserved and produce `unsupported-syntax`;
  their runtime and grammar are deliberately not implied by highlighting.

## Pipelines and match

```text
let names = ls("/docs") | where size > 100 | select name
let label = match status {
    0 => "ok",
    _ => "failed"
}
```

A pipeline preserves an input expression and ordered stages. Each stage must
be a function name, member or call. This AST does not decide whether values,
records or bytes flow between stages; that belongs to the eventual runtime.

A match contains a scrutinee and `pattern => expression` arms in braces.
An arm value starts on the same line as `=>`; use parentheses for a multiline
value. Separate arms by commas or newlines. Patterns are literals, binding names or
`_`; a binding/wildcard must be last. Empty matches are errors. Destructuring,
guards, exhaustiveness analysis and block-valued arms are not implemented.

## Rust API and compatibility

```rust
use themoretheless_tokenizer_rush::{parse, StmtKind};
let parsed = parse("fn main() { return 1; } // demo\n");
assert!(parsed.is_valid());
assert!(parsed.lexed.is_lossless(parsed.source));
assert!(matches!(parsed.module.items[0].kind, StmtKind::Function { .. }));
```

`parse` now returns this crate's `Parse`, not fullkit's `Parse`. Update callers
that matched the former shared `Item` / `Stmt` / `Expr` to the exported Rush
`StmtKind` / `ExprKind` types. Functions retain parameters/result types; loops
retain bindings/iterables; pipes and matches have their own nodes. Spans use
UTF-8 byte offsets and borrowed names preserve the original spelling.

`parse_with(source, InputLimits)` enforces input, token, diagnostic and nesting
budgets. Nesting is capped at 128 even if a larger user budget is supplied.
Token exhaustion emits one terminal error span for the remaining source to
preserve coverage. Zero diagnostic storage does not turn invalid input valid;
use `Parse::is_valid()` or the host's `valid` field. `lex` alone checks lexical
structure; the host syntax and semantic layers both run syntax validation.

Semantic highlighting marks function/variable/parameter declarations, types
and member names. It does not imply name resolution. Recovery preserves later
statements, and a recovered error keeps the parse invalid.
