# themoretheless-tokenizer-rush

Rush editor engine with a dedicated lossless lexer, recovering parser and
borrowing syntax tree. It checks the grammar below and reports syntax errors,
including in both host token layers. An optional binding analysis pass checks
duplicate declarations and immutable bindings. The experimental evaluator below
executes a functional subset with runtime contracts and portable graphics values.
A compact [language contract](../../docs/rush-language-spec.md) defines executable semantics.
General type inference and a whole-runtime memory limit remain planned.

## Basic syntax

Rush uses `.r`, case-sensitive names and keywords, `//` line comments,
non-nested `/* ... */` comments, and `return`. R also uses `.r`; hosts should
select the `rush` language id explicitly when an extension is ambiguous.

A newline or `;` ends a simple statement. Newlines inside parentheses, lists,
and maps can continue an expression. A bare `return` ends at the newline.
Strings use single or double quotes, with backslash escapes, and cannot cross
an unescaped line ending. The lexer preserves literal spelling; runtime decodes supported escapes.
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
Type syntax is `Name` or `Name[Type, ...]`. Parsing accepts arbitrary names;
the evaluator resolves supported annotations and rejects unknown types.
`str` and `f64` are the canonical spellings; `string` and `float` remain accepted
legacy spellings. `Geometry`, `Row`, `T`, `print`, `where`, `select`, `count`,
`show`, `run`, `param` and `assert` are ordinary names, not special parser rules.

## Statements and expressions

- `let` / `const name [: Type] = expression` creates an immutable binding;
  `mut name` or `let mut name` explicitly permits reassignment in analysis.
  Initializers are required.
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
- `async` and `await` are reserved and produce `unsupported-syntax`;
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
be a function name, member or call. Runtime passes the input value once as the
first argument of each successive stage; lists remain single values.

A match contains a scrutinee and `pattern => expression` arms in braces.
An arm value starts on the same line as `=>`; use parentheses for a multiline
value. Separate arms by commas or newlines. Patterns are literals, binding names or
`_`, variants, tuples and records. Guards use `if (condition)`. An unguarded
binding/wildcard must be last. Empty matches are errors. Exhaustiveness analysis
and block-valued arms are not implemented.

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

## Optional binding analysis

`analyze(source)` and `analyze_with(source, limits)` parse and then check lexical
bindings. `parse`, `validate`, tokenization and the host entrypoints continue to
check syntax only; opt into `analyze` when binding diagnostics are wanted.

```rust
use themoretheless_tokenizer_rush::{analyze, parse};
let source = "const size = 10; size += 1";
assert!(parse(source).is_valid());
assert!(!analyze(source).is_valid());
```

- Duplicate declarations in one scope report `duplicate-binding`. Parameters
  share the function body's scope; a loop binding shares its body's scope.
- `let`, `const`, lambda parameters and function names cannot be reassigned
  (`immutable-binding`). Only explicit `mut` bindings permit reassignment. Named function parameters,
  loop bindings and match bindings are immutable.
- Nested blocks may shadow outer names. A match binding belongs only to its arm;
  `_` does not declare a binding. Names retain exact spelling, including `$`.
- This analysis pass accepts member/index assignment syntax; runtime currently
  rejects such writes even for `mut`. Only assignment to a mutable name executes.
- Declarations become visible in source order, including while inspecting nested
  function bodies. References to later declarations are not resolved. Unknown
  names remain accepted for external commands and host-provided pipeline fields.
- Analysis skips recovered invalid syntax trees to avoid cascading errors.
  Diagnostic limits apply; a zero limit still preserves invalid status.

This is a partial binding check, not a type checker or complete name resolver.

### Experimental functional evaluation

`evaluate(source, expression_budget)` executes numbers, booleans, lists, `const`
bindings, arithmetic, short-circuit boolean operators and expression lambdas.
The module result is the result of its final statement (see the language contract).
Lambda parameters are immutable;
closures capture immutable bindings by value and mutable bindings through
shared cells within the same execution.

```rust
use themoretheless_tokenizer_rush::{evaluate, Value};
let source = "const scale = factor => x => factor * x\nconst double = scale(2)\ndouble(21)";
assert_eq!(evaluate(source, 100).unwrap(), Value::Number(42.0));
```

The evaluator rejects unsupported statements and expressions, unknown names,
incorrect argument counts, non-finite arithmetic and exhausted execution budgets.
Expression nesting during evaluation is limited to 64. These limits are not a
memory sandbox: captured environments and returned collections allocate memory.
Variable mutation, host callbacks and 2D polygon execution are supported. Parsing a construct does not yet imply that it can be evaluated.

Collection functions are first-class values: `map(list, callback)`,
`filter(list, predicate)` and `fold(list, initial, reducer)`. A filter predicate
must return a boolean. Empty folds return their initial value.
`input | function(args)` passes input as the first argument; `input | function`
passes only input. Each callback invocation consumes the shared execution budget.

```rush
[1, 2, 3, 4]
```

A pipeline can use one line or continue on indented lines beginning with `|`:

```rush
[1, 2, 3, 4] | filter(x => x % 2 == 0) | map(x => x * x) | fold(0, (sum, x) => sum + x)
// Result: 20
```

Mathematical values now include `vec2`, `vec3` and `vec4`. Vectors support
addition/subtraction at matching dimensions and multiplication/division by a
scalar; scalar multiplication also works with the scalar first. `dot`, `length`
and `normalize` operate on vectors. Zero vectors cannot be normalized.
`sin` and `cos` accept radians; `deg(number)` converts degrees to radians as a
number. `degrees` and `radians` construct distinct `angle` values. `sqrt` rejects negative inputs.
Non-finite results and dimension mismatches produce runtime errors.

Named functions now execute local constants, nested functions, boolean `if`
statements and explicit `return` (including returns inside nested branches).
Falling through a named function returns `null`. Self-recursion uses the same
execution and depth limits. Functions capture lexical values at declaration time;
mutual recursion and forward references are not supported yet.
`match` evaluates only the selected arm, with arm-local bindings; a missing match
is a runtime error. Function type annotations are checked at runtime.

`range_iter(start, end[, step])` creates a repeatable lazy numeric range for
`for` loops. It stores three numbers and computes each element when requested;
`break` and `return` stop consumption immediately. The endpoint is excluded,
the default step is 1, and a zero step is rejected. Each iteration consumes the
execution budget and checks cancellation. If floating-point arithmetic cannot
advance, consumption reports an error instead of looping forever.

```rush
mut sum = 0
for x in range_iter(0, 1000000000000) {
    if x == 4 { break }
    sum += x
}
sum // 6
```

Each loop starts the range again. `iter(list)` wraps an existing list as a
repeatable sequence. `map` and `filter` return deferred sequences when given a
range iterator or sequence; their behavior on lists remains eager.

```rush
range_iter(0, 1000000000000)
    | map(x => x * 2)
    | filter(x => x >= 4)
    | collect(3) // [4, 6, 8]
```

`collect(sequence, maximum)` returns at most `maximum` results, without requesting
an extra item. The maximum is a required nonnegative integer; zero invokes no
callbacks. `for`, `fold`, `any`, and `all` consume sequences directly. `any` and
`all` stop as soon as their answer is known. `flat_map` accepts sequences but
still collects its output eagerly.

Transformations run in order for each input. Reusing a sequence starts it again
and reruns callbacks, including side effects on captured mutable variables.
Callbacks are validated as callable at construction; their argument/result
contracts are checked when consumed. Invalid filter results are runtime errors.
Sequences with callbacks cannot be compared for equality.

Numeric sources and transformations retain only the range description, callbacks,
and current item. `iter(list)` retains its already allocated input. `collect`
bounds the number of output items, not their byte size; closures, nested values,
and source lists may retain additional memory. Budget and cancellation checks
also apply to rejected filter candidates. Host sources can provide resource-owning iterators as described below. A
whole-runtime memory limit remains planned.

`range(start, end, step)` generates a list, excluding `end`; `step` defaults to 1
and can be negative. Zero steps and floating-point steps that cannot advance are
errors. Each generated element consumes execution budget.
`cross(a, b)` requires two 3D vectors. `lerp(a, b, t)` supports numbers or matching
vectors and permits extrapolation. `clamp(x, low, high)` requires ordered bounds;
`smoothstep(low, high, x)` requires strictly increasing bounds.

```rush
range(0, 360, 90) | map(a => vec2(cos(deg(a)), sin(deg(a))) * 2)
// Four points on a circle of radius 2.
```

### 2D graphics prototype

`polygon(points)` constructs a polygon from at least three `vec2` values.
`shape | rotate(radians) | translate(vec2(x, y))` returns transformed copies.
Rust callers can inspect `Polygon::points()` and export `Polygon::to_svg()`.
The SVG uses an upward mathematical Y axis and even-odd filling for intersecting
contours; this prototype does not perform CAD validity checks or boolean unions.
Run `cargo run -p themoretheless-tokenizer-rush --example polygon` to print an SVG
flower computed by a Rush script. Rendering, 3D solids and engine integration
remain separate work.

### Repeated execution and cancellation

`Program::compile(source)` retains the analyzed AST. Call
`program.run(budget, &cancellation, &[("time", seconds), ("radius", radius)])`
to recompute with new finite numeric parameters. Each run starts with a fresh
environment and budget. Input names must be unique and cannot replace builtins;
script declarations can shadow inputs. Input name lifetimes follow the program
source lifetime. `run_with_host` accepts typed callbacks (described below);
opaque application-owned objects remain unimplemented.

`CancellationToken` is cloneable and can be cancelled from another thread.
Cancellation is checked at statement/expression boundaries, function calls and
each generated range element. A cancelled token stays cancelled; create a new
one for another run. Cancellation does not interrupt a single polygon operation
or allocation in progress, and there is no hard memory limit yet.

### Run a script file

```sh
cargo run -p themoretheless-tokenizer-rush --bin rush -- crates/tokenizer-rush/examples/scripts/flower.r radius=30 petals=6 time=0 > flower.svg
```

The `rush` binary reads a file, accepts numeric `name=value` parameters and prints
SVG for polygon results. Other values currently use Rust debug formatting.
Runtime failures include the source file and line and return a failing exit code.
The CLI uses a fixed budget of one million steps. `examples/scripts/spiral.r`
provides a second procedural example that returns point data.

### Host function contracts

`Program::run_with_host` accepts registered `Rc<HostFunction>` values. Each entry
provides a name, parameter types, result type and Rust callback. `ValueType`
covers finite numbers, booleans, fixed-dimension vectors, polygons, typed lists
and null. Arguments are checked before calling Rust; results are checked afterward.
Registered functions can be passed to `map` and called through pipelines.
Callbacks receive the cancellation token and return `Result<Value, String>`.
They must cooperate with cancellation; Rush cannot preempt native code, catch
all native failures or account for native allocations. Stateful closures and
opaque application objects are not supported by this initial callback interface.
Signatures are public metadata. `analyze_host_calls(source, &functions)` uses
the same registrations for static argument-count checks and provably incompatible
argument types, including pipelines and immutable aliases. Local declarations
shadow host names. It recognizes literal types, nested list/tuple literals,
obvious arithmetic/boolean results, equal types in both conditional branches,
and declared results of registered host calls. No callback executes during analysis.

For a host registration `double(number) -> number`, both `double(true)` and
`true | double` produce `argument-type` at the supplied value. If `text(number)`
returns a string, `1 | text | double` reports the incompatible stage result.
Empty lists and `None` can satisfy different element contracts, so overlapping
contracts are not rejected merely because their type descriptions differ.

Unknown variables, mutable aliases, user-function results, inferred types through
arbitrary higher-order calls, and general dataflow remain runtime checks. This
is a conservative check of known expression shapes, not whole-program type
inference. Argument and result contracts are still enforced at every runtime
host call even when static analysis found no error.

Conditional expressions use `if condition { value } else { value }`; both branches
are required, and only the selected one is evaluated. They work in constants,
returns and lambda bodies. Standalone `if` statements retain their block syntax.
Vectors expose `.x`, `.y`, `.z`, `.w` within their dimensions. Lists and vectors
support zero-based integer indexing; negative, fractional and out-of-bounds
indices are runtime errors.

Runtime annotations support `number` (`f64`), `bool`, `vec2`, `vec3`, `vec4`,
`polygon` and recursive `list[T]` types. Named function parameter/result contracts
and constant annotations use the same `ValueType` checks as host functions.
Unknown annotations produce errors when their declarations execute. This is
runtime validation; static inference and user-defined types remain planned.

### 3D transformation matrices

`identity()`, `translation(vec3(...))`, `scaling(vec3(...))` and
`rotation_z(radians)` create 4x4 matrices. `A * B` applies B first, then A.
`transform_point(matrix, vec3(...))` includes translation;
`transform_direction(matrix, vec3(...))` does not. Matrices use row-major storage
and column-vector multiplication. Rust callers can inspect `Matrix4::rows()`.
These are mathematical transforms; they do not yet construct or render 3D solids.

Tuples use `(a, b)`, `(a,)` or `()` and support integer indexing; `(a)` remains
ordinary grouping. `zip(left, right)` pairs two lists into a list of two-element
tuples, stopping at the shorter list. For example,
`zip([1, 2], [10, 20]) | map(pair => vec2(pair[0], pair[1]))` constructs points.
Tuple annotations use `tuple[T, U, ...]`.

Immutable tuple bindings support `const (x, (y, _)) = (1, (2, 3))`.
Patterns must match tuple shape exactly, names cannot repeat in a scope, and `_`
discards a value. Binding validation completes before inserting any names.
Named functions and lambdas also accept nested tuple and record parameter
patterns. Each top-level pattern counts as one argument; duplicate bound names
across parameters are rejected. `_` can appear repeatedly and discards values.
Record patterns require the named fields and allow extra fields. Parameter
patterns accept names, tuples and records; literals and variant patterns belong
to `match`.

```rush
fn distance_squared((x, y): tuple[number, number]) { return x*x + y*y }
let point_sum = ({point: (x, y)}) => x + y
zip([1, 2], [10, 20]) | map(((x, y)) => vec2(x, y))
```

The outer parentheses delimit lambda arguments: `((x, y)) => ...` takes one
tuple, while `(x, y) => ...` takes two arguments. Named-function type annotations
apply to the whole argument before destructuring. A shape mismatch stops the
call before the body executes and points to the parameter pattern in the
function's source module. The Rust AST exposes `Parameter.pattern`; lambda
parameters are pattern expressions as well.

`match` additionally supports variant patterns and guards.

`rotation_x`, `rotation_y` and `rotation_z` use right-handed rotations in radians.
Matrices participate in function/constant contracts through `mat4` and host
contracts through `ValueType::Matrix4`. Composition preserves direction lengths
for rotations; non-uniform scaling intentionally changes them.

Records use `{radius: 20, center: vec2(0, 0)}` with immutable fields read through
`config.radius`. Keys are literal names or strings; duplicate keys are rejected.
Strings support single/double quotes, Unicode text, `\n`, `\r`, `\t`, escaped
quotes and backslashes. Unknown escapes fail during evaluation. Strings concatenate
with `+` and support `string` contracts. Equality compares data values structurally;
comparing a function encountered during structural traversal is an error. Record
patterns are supported; named record type declarations remain planned.

Structural comparison consumes execution budget for traversed values, strings,
record keys and polygon points. Nested collections use an explicit work list.
Comparison short-circuits on a difference, so later values may not be visited.

Breaking semantic change: plain `let` is now immutable, matching the functional
language direction. Existing mutable declarations must use `mut` or `let mut`.
Mutable declarations execute with run-local cells. Immutable `let` executes
exactly like `const`.

`mut` variables support `=`, `+=`, `-=`, `*=`, `/=` and `%=`. Closures share
captured mutable cells, including when returned from a function. Each program
run owns fresh cells; typed variables validate every assigned value. Cells use
arena indices so closure/cell cycles do not form reference-counting leaks.
Returned closures are inspection values: this API does not invoke them after
their run ends. Field/index mutation remains unsupported by the evaluator.

`for name in list` and boolean `while` loops now execute with scoped bodies,
`break`, `continue` and propagation of function `return`. For bindings are fresh
and immutable per iteration; outer mutable cells remain shared. Conditions and
iterations consume the execution budget, including empty loop bodies.

Unit quaternions use `axis_angle(vec3(...), radians)`. The axis is normalized;
zero axes fail. `q1 * q2` composes rotations with q2 applied first, and
`rotation_matrix(q)` converts to `mat4`. `quat` annotations and
`ValueType::Quaternion` support runtime contracts. Components use `(x,y,z,w)`
order in the Rust API. `slerp(a, b, t)` interpolates the shortest rotation arc for finite `t` in [0, 1];
almost identical orientations use normalized linear interpolation.

### Formula surfaces

`grid_mesh(xs, ys, (x, y) => vec3(...))` samples a point function on a rectangular
grid and creates two triangles per cell. Both axes need at least two entries;
the callback must return a finite vec3. Vertices are stored with X varying fastest,
and triangle winding follows increasing grid indices. Sampling and triangulation
consume the execution budget. This constructs a mesh, not a watertight CAD solid;
degenerate coordinates and self-intersections are not repaired automatically.
`mesh` annotations and `ValueType::Mesh` support runtime contracts.

```sh
cargo run -p themoretheless-tokenizer-rush --bin rush -- crates/tokenizer-rush/examples/scripts/surface.r amplitude=2 > surface.obj
```

Mesh results print OBJ through the CLI; Rust callers can inspect vertices,
triangles or call `Mesh::to_obj()`.

`mesh | transform(matrix)` creates a transformed mesh, preserving vertex order
and triangle indices. Each vertex and copied triangle consumes budget. Original
meshes remain unchanged; non-finite transformed positions fail. Reflections retain
triangle index order (therefore reverse geometric orientation), and singular
scales may produce degenerate triangles. Automatic normal repair is not provided.

Procedural randomness is stateless: `random(seed, index)` returns a reproducible
number in [0, 1). Seeds and indices are nonnegative integers up to 2^53-1.
`noise(x, seed)` is 1D value noise with quintic interpolation between hashed
lattice samples; negative coordinates are supported and |x| must not exceed
2^53-2. It produces smooth, deterministic variation in [0, 1]. This is not
cryptographic randomness or multidimensional gradient noise.

Built-in variants `Some(value)`, `None()`, `Ok(value)` and `Err(error)` support
explicit optional/results data. Match arms can use nested constructor patterns,
for example `Some(Ok(x)) => x`. Bindings belong to the selected arm and are
immutable. Missing matches fail at runtime. These are initial built-in variants;
user-defined algebraic types, exhaustiveness
checking and automatic error propagation are still planned.

`Option[T]`, `Result[T, E]` and `tuple[T, U, ...]` annotations validate nested
payload types at runtime and are also available through `ValueType` for host
contracts. `None()` matches any Option element type; `Some`, `Ok` and `Err`
validate exactly one payload. These contracts do not provide static inference.

### Lexical navigation API

`analyze_references(source)` returns the analyzed parse and `NameReference`
entries with usage spans and optional declaration spans. Resolution respects
source order, nested scopes, lambda captures and match bindings. Record field
names are literal keys, not variable references. Missing definitions indicate
external or unresolved names, not automatic errors. Invalid syntax produces no
links; type names and member fields are not resolved by this API yet.

`analyze_names(source, external_names)` opts into strict lexical name checking,
including unexecuted branches. Supply registered host function and input names
explicitly. The public `builtin_catalog()` is the exact table used to populate
the runtime environment, so editor validation does not duplicate builtin names.
Forward references remain invalid under the language's declaration-order capture
semantics. This is name resolution, not static type or call-arity checking.

`analyze_calls(source)` checks direct builtin argument counts before execution,
including the first argument supplied by a pipeline. It respects local shadowing
and inspects unexecuted branches. `Builtin::arity()` is shared with runtime call
validation. It follows named functions, direct lambdas, immutable aliases and
conditional function expressions. Destructuring literal tuples and records with
name keys preserves function signatures, including registered host signatures;
values returned by arbitrary expressions remain dynamically checked. For a conditional, it reports an argument-count
error when every possible signature rejects that count. If either branch has an
unknown signature, the call remains dynamically checked. Mutable function bindings
and function-valued parameters also remain dynamically checked. Host registrations
can additionally be checked with `analyze_host_calls`, described below.

`rush --check script.r [name=number ...]` validates syntax, bindings, registered
names and known call arity without executing the script. Diagnostics include
file, line, Unicode character column and a source-line pointer. Input values must
be finite and names unique. Check mode is not a full type checker and does not
prove execution will succeed; for example, it accepts an infinite loop.

### Explicit modules

`import math` binds an immutable namespace supplied through
`Program::run_with_modules(..., &[("math", &math_program)])`. A module evaluates
to its final record, explicitly selecting exported values, for example
`{square: square}`. It initializes once per run, shares its captured state across
imports and starts fresh on the next run. Modules see registered builtins, host
functions and numeric inputs, not the importing function's local variables.
Missing registrations, cyclic imports and non-record exports fail. The core does
not read files or fetch modules; The CLI loads explicitly supplied module files. Runtime errors carry the originating module name and source span.

CLI modules use `--module name=path.r`, repeated for each module. For example:

```sh
cargo run -p themoretheless-tokenizer-rush --bin rush -- crates/tokenizer-rush/examples/scripts/modular-flower.r --module curves=crates/tokenizer-rush/examples/scripts/curves.r > flower.svg
```

`--check` also analyzes every supplied module without initializing it. Module
syntax/check diagnostics use the module filename. Missing imports and cycles in the reachable module graph are checked before execution; runtime errors retain source-module attribution across exported closures.

`Program::imports()` lists syntactic dependencies, including unexecuted branches
and function bodies. `Program::validate_modules(registry)` checks missing modules,
duplicate registrations and cycles with an explicit traversal stack. The CLI
uses this in `--check`; evaluation still validates imports as they execute.

Explicit angle values use `degrees(number)` or `radians(number)` and the `angle`
annotation (`ValueType::Angle` for hosts). Angles add/subtract with angles and
scale by numbers; mixing an angle and a number with `+` is an error.
Trigonometry and rotations accept angle values directly. For existing scripts,
plain numeric radians and the numeric conversion `deg` remain accepted by these
builtins; an `angle`-annotated function rejects a plain number.

Record binding patterns use `const {radius: r, center: (x, y)} = value` and can
nest tuple/record patterns. Named keys select fields and the right side names
local bindings; unmentioned fields are allowed. Missing fields, repeated pattern
keys and duplicate binding names fail. Destructured bindings are immutable.

`match` supports tuple patterns `(x, y)` and record patterns `{kind: 'circle',
radius: r}`, nested inside variants or each other. Tuple shapes must match
exactly; records can have additional fields. A failed arm does not expose its
bindings to later arms. Parenthesized match patterns denote tuples (use `(x,)`
for consistency with tuple expressions); guards use `if (condition)` before `=>`.

### Playground execution

The playground's **Run Rush** button evaluates the current source with a fixed
100,000-step budget. Polygon results can be inspected as SVG; mesh results are
textual OBJ with download. 3D rendering belongs to embedding applications.
Development uses the local Rust bridge; static hosting requires rebuilding the
WASM package (`npm run build:pages` in `playground`).

`flat_map(list, callback)` maps each item to a list and concatenates those lists
in input order, flattening exactly one level. It also accepts pipeline syntax:
`[1, 2] | flat_map(x => [x, x * 10])` produces `[1, 10, 2, 20]`.
Returning a non-list is an error. Each emitted item consumes execution budget,
including when callbacks return an existing list.

### Bézier curves in Rush

`examples/scripts/curves.r` exports `quadratic(a,b,c,t)`, `cubic(a,b,c,d,t)`
and their analytic tangents (`quadratic_tangent`, `cubic_tangent`). They work
with scalar values or same-sized vectors. Parameters from 0 to 1 trace the
curve; other finite parameters extrapolate. Tangents are not normalized.
Evaluation uses nested linear interpolation (de Casteljau).

Generate a triangle ribbon along a cubic curve with:

```sh
cargo run -p themoretheless-tokenizer-rush --bin rush -- \
  crates/tokenizer-rush/examples/scripts/bezier-ribbon.r \
  --module curves=crates/tokenizer-rush/examples/scripts/curves.r > ribbon.obj
```

### Core execution contract

- `x` and `$x` are distinct names; `$` alone is invalid.
- Declarations are visible in source order. Named functions can call themselves;
  mutual recursion and references to later bindings are not supported.
- `let` and `const` are immutable. Explicit `mut` enables reassignment; captured
  mutable bindings are shared within a run. Field/index assignment is not implemented.
- Numbers are finite IEEE 754 f64 values. There is no separate integer type;
  integer precision above 2^53 is not guaranteed. Non-finite results are errors.
- Missing names, fields and indices produce errors. A function without an explicit
  return produces `null`; a module produces its final statement value.
- A pipeline evaluates its input once and inserts it as the next call's first argument.
- Type annotations are runtime contracts. Static analysis does not infer types.
- Imports resolve through an explicit host registry; CLI module files must be supplied
  with `--module`. No implicit filesystem or network access is granted.

### Runtime benchmarks

```sh
cargo bench -p themoretheless-tokenizer-rush --bench runtime
# Shorter measurement:
RUSH_BENCH_ITERS=30 cargo bench -p themoretheless-tokenizer-rush --bench runtime
```

The benchmark validates outputs before timing and separately measures compilation,
execution of a prepared program, and compilation plus execution. It covers a
captured closure, eager and lazy map/filter/fold over 1,000 numbers, and a 961-vertex grid mesh.
Each operation has 10 warmups and 7 timed samples; output reports minimum, median
and maximum microseconds per operation. Result allocation and destruction are
included. SVG/OBJ export and browser rendering are excluded. Prepared execution
still initializes a fresh runtime and its builtins on every run.

An earlier implementation baseline on macOS/aarch64 with 30 iterations per sample measured these medians:

| Workload | Compile (µs) | Prepared run (µs) | Compile + run (µs) |
| --- | ---: | ---: | ---: |
| Closure | 1.792 | 3.526 | 5.497 |
| Collections | 3.172 | 737.082 | 751.944 |
| Grid mesh | 3.163 | 518.069 | 522.529 |

These are an initial local baseline, not cross-language results or a performance
promise. The operations are timed independently, so their medians need not add
exactly. Profile execution before choosing a bytecode backend or optimizing
parser throughput for these workloads. A later [eager/lazy memory and timing study](../../docs/rush-performance.md)
measures requested heap bytes, process RSS, and native cancellation latency separately.
Competitor comparisons remain unmeasured.

`RUSH_BENCH_CASE` selects `closure`, `collections`, `collections_lazy`, or `surface`.
`RUSH_BENCH_ITEMS` sets collection input size (1–1000000); both collection variants
are validated against the same Rust reference sum. `RUSH_BENCH_OPERATION` selects
`compile`, `prepared_run`, `compile_and_run`, or `verify`. The last mode runs once
with result validation and no warmup, for external process-memory measurements.

`cargo bench -p themoretheless-tokenizer-rush --bench memory` uses a counting
allocator in a separate executable. It reports peak additional requested heap
bytes, allocation calls, and live bytes after dropping the result, for three runs
at each input size. It asserts that the final live bytes return to baseline.
Its allocation counter is deliberately excluded from timing measurements.

### Assertions in scripts

`assert(condition[, message])` requires a boolean condition and an optional
string message. Success returns `null`; failure stops execution with the call's
source span and message (default: `Assertion failed`). Arguments are evaluated
eagerly, including the message. Assertions remain enabled in optimized builds.

Run the curve library checks with the regular CLI:

```sh
cargo run -p themoretheless-tokenizer-rush --bin rush -- \
  crates/tokenizer-rush/examples/scripts/curves-test.r \
  --module curves=crates/tokenizer-rush/examples/scripts/curves.r
```

A failed assertion exits through the CLI's normal runtime-error path. Directory-based discovery and isolated execution are available through
`--test`, described below.

`mesh(vertices, triangles)` constructs an indexed triangle mesh. Vertices are a
list of `vec3`; triangles are lists of three zero-based integer indices, for
example `mesh([vec3(0,0,0), vec3(1,0,0), vec3(0,1,0)], [[0,1,2]])`.
Out-of-bounds, fractional and repeated indices fail. Each vertex and triangle
consumes execution budget. As with `grid_mesh`, this validates representation,
not manifoldness or geometric self-intersections; collinear faces are allowed.
See `examples/scripts/tetrahedron.r` for a closed four-face example.

Meshes expose read-only `vertices` (list of vec3) and `triangles` (list of
three-index lists). Each access creates a data copy and charges one execution
step per vertex or triangle. Functional reconstruction leaves the input intact:

```text
const lifted = mesh(
    original.vertices | map(v => v + vec3(0, 0, 2)),
    original.triangles
)
```

Filtering vertices requires updating indices yourself; invalid references are
rejected by the constructor. Use `transform` for standard matrix transforms to
avoid converting mesh data into intermediate Rush lists.

`len(value)` returns the number of list/tuple elements, record fields or Unicode
scalar values in a string. Combining marks count separately (`len('é') == 2`);
this is not a grapheme or byte count. String scanning charges execution budget
by UTF-8 byte length. Use `length(vector)` for Euclidean vector magnitude.

The `examples/scripts/meshes.r` module implements `join(a, b)` entirely in Rush:
it concatenates vertex data and offsets the second mesh's face indices using
`len`, `map` and `flat_map`. It preserves disconnected components; it does not
perform solid boolean union, intersection removal or vertex welding.

`get(collection, key)` provides optional lookup: records use string keys and
lists/tuples use non-negative integer indices. A present entry returns `Some(value)`;
a missing key or out-of-range index returns `None`. A present `null` is therefore
`Some(null)`. Wrong key types, negative indices and fractional indices are errors.
Direct member/index access retains its strict error-on-missing behavior.

```text
match get(settings, 'radius') {
    Some(radius) => radius,
    None => 10
}
```

`any(list, predicate)` and `all(list, predicate)` inspect items in order and stop
at the first `true` or `false`, respectively. Predicates must return booleans;
empty lists produce `false` for `any` and `true` for `all`. The callback must be
callable even for an empty list. Calls share the normal execution budget and
cancellation checks. For example: `vertices | all(v => v.z >= 0)`.
These functions avoid an intermediate result list; the input list is still eager.

Run a directory of script tests with `rush --test directory [--module name=path.r ...]`.
The runner discovers regular files ending in `-test.r` or `_test.r` directly in
that directory, sorts them by path and executes each in a separate process with
the ordinary one-million-step budget. Numeric inputs and module options are
forwarded to every file. It continues after failures, prints PASS/FAIL per file
and a total, and exits unsuccessfully if any test fails or no tests are found.
Discovery is not recursive and does not follow symlinks. Each file is one test
case; assertions within a file stop at its first failure. This provides process
isolation, not a wall-clock timeout or OS memory sandbox.

Match guards evaluate only after a pattern matches, with its bindings in scope.
A false guard continues to the next arm; a non-boolean result is an error.
A guarded wildcard does not make later arms unreachable. Parentheses around the
condition are required to distinguish it from lambda syntax. For example:

```text
match get(settings, 'radius') {
    Some(r) if (r > 0) => r,
    _ => 10
}
```

Records support computed string keys: `row[key]` is strict lookup, equivalent to
`row.radius` when `key == 'radius'`. Missing fields are errors and numeric keys
are rejected. Use `get(row, key)` when absence is expected. For example,
`['height', 'radius'] | map(key => row[key])` projects selected values in order.

String literals accept Unicode scalar escapes with 1–6 hexadecimal digits in
braces: `"\u{41}\u{44f}\u{1F642}"` evaluates to `"Aя🙂"`. Empty escapes,
surrogates, values above U+10FFFF and malformed sequences produce runtime errors.
This applies to both quote styles and quoted record keys. No Unicode normalization
is performed; composed and decomposed text remain distinct values.

String decoding charges one step per source UTF-8 byte inside the quotes, and
concatenation charges the combined UTF-8 byte length before allocating output.
Repeated doubling therefore exhausts a bounded budget as output grows. This is
work accounting, not a global memory limit: source parsing, value clones,
host callbacks and other allocations still need separate memory controls.

The static playground executes each Rush run in a fresh Web Worker. Cancel,
source changes, completion and errors terminate that worker. A 10-second timeout
also terminates it, including WASM loading time. This keeps execution off the UI
thread but does not bound process memory. Development mode uses the Rust HTTP bridge. A disconnected execution request
terminates its dedicated process group on macOS/Linux, including cargo children.
On Windows only the direct child is terminated; descendant cleanup is not yet
guaranteed. Tokenization and
semantic analysis are currently separate from the execution worker.


### Local name completion

`analyze_editor(source)` returns the parsed document, resolved references, and
lexical bindings with definition spans, visibility intervals, and scope depth.
Offsets are UTF-8 bytes. Visibility intervals are half-open; the module interval
ends at `source.len() + 1` to include the end-of-file cursor. Initializers see the
previous environment; newly declared names become visible after initialization.
Nested scopes include destructured parameters, loop bindings and match guards.
Syntax recovery suppresses editor metadata rather than guessing definitions.

The playground's Ctrl+Space combines visible bindings with the builtin catalog.
The innermost binding wins on a name collision. Local names are inserted without
call parentheses, including local function names; builtins retain call insertion.
Completion uses the beginning of the identifier under the cursor and requires
analysis of the current source. Member completion follows known immutable record
fields, vector components and mesh fields (`vertices`, `triangles`) through
dotted paths. Mesh vertices have type `list[vec3]`; triangle indices have type
`list[list[number]]`, including during static checks after indexing. Destructuring tuple and record
literals preserves nested record fields, including vector components. Record
patterns also preserve known fields when their source is an immutable record
binding or alias; mutable source bindings do not retain their initial shape. Shadowed
bindings use their own metadata. Static diagnostics reject member access on
known unsupported values (such as numbers, booleans, strings, lists and null);
unknown parameter types remain checked at runtime. A trailing dot supports limited completion
recovery; arbitrary incomplete syntax, dynamic fields and call results are not
inferred by this completion path.


### Cancellation latency benchmark

`cargo bench -p themoretheless-tokenizer-rush --bench cancellation` measures
request-to-return latency from another thread for an infinite loop, eager range
construction, a lazy filter rejecting every item, and a blocking host callback.
It synchronizes on entry, requests cancellation after 5 ms, and records 25 samples
after two warmups. Every run must end with the cancellation error. Runtime cleanup
and OS scheduling are included; parsing and requester startup are excluded.
The sleeping host callback demonstrates cooperative cancellation: the runtime
cannot interrupt host code that ignores its token. These native measurements do
not describe browser worker termination. See the performance report for results.


### Resource-owning host sequences

An embedding app can return `Value::host_sequence(factory, item_type)` from a
registered callback whose result contract is `ValueType::Sequence`. The Rush
annotation is `sequence`; `range_iter` and ordinary deferred sequences also
satisfy it. This contract does not statically infer the element type through
transformations.

- `HostSequence::open(&CancellationToken)` creates a boxed `HostSequenceIterator`.
  Factories should retain configuration and open their resource in this method.
- `HostSequenceIterator::next(&CancellationToken)` returns
  `Result<Option<Value<'static>>, String>`. Returned values own their data.
  Every item is checked against the type passed to `Value::host_sequence`, so an
  invalid numeric value or wrong item type becomes a runtime error.
- Constructing or transforming a sequence does not open it. The first requested
  item opens an iterator; `collect(0)` opens nothing. Every new consumer opens its
  own iterator. External contents may change between openings; results are not
  cached or promised to be identical.
- The runtime owns the opened iterator exclusively and drops it at EOF or when
  its consumer exits through `break`, `return`, a collection limit, short-circuit
  `any`/`all`, a runtime error, budget exhaustion, or cancellation. The host must
  keep its resource handle in that iterator and release it through Rust `Drop`.
  Partial resources created by a failing `open` remain the factory's responsibility.
- Cancellation is checked before opening/reading and after each host operation.
  Blocking open/read/Drop implementations cannot be forcibly interrupted by the
  interpreter; the host must provide cooperative cancellation or its own timeout.
  There is no background prefetch or additional source buffer in Rush.

The [host_lines example](examples/host_lines.rs) opens a text file selected by the
embedding process and supplies `lines()` to a script:

```sh
cargo run -p themoretheless-tokenizer-rush --example host_lines -- path/to/file.txt
```

```rush
lines() | filter(line => len(line) > 0) | collect(3)
```

It closes the reader after three accepted lines even if the file is larger.
`lines` is an example host registration, not a builtin file-access capability.
The example uses blocking file reads and can allocate one arbitrarily long line;
item-count limits and runtime steps do not bound host allocation sizes or I/O time.


### Formatting

```sh
rush --fmt script.r          # Print formatted source to stdout.
rush --fmt-check script.r    # Exit unsuccessfully if formatting differs.
```

Both commands read one file, never execute its code, and leave the file unchanged.
Syntax errors include source location and produce no formatted output. Rust hosts
can call `format_source(&str) -> Result<String, FormatError>`.

Canonical style uses four-space indentation, braced blocks, parenthesized calls
and function parameter lists, semicolons after simple statements, and one final
newline for nonempty files. Immutable declarations use `let`, including inputs
written with `const`; mutable declarations use `mut`, including `let mut`.
Parentheses preserve operator precedence and associativity. Lambdas always use
parenthesized parameter lists. No line-width wrapping or style options are
provided yet; long expressions remain on one line unless comments require breaks.

```rush
fn add(a: number, b: number) -> number {
    return a + b;
}
let result = add(1, 2);
```

The formatter prints the AST, preserving literal spellings and the text/order of
line and block comments. Comment placement and whitespace can be normalized;
expressions containing line comments may retain extra grouping to preserve
continuation rules. It then reparses the output, compares canonical AST
representations and comment sequences, and verifies that formatting it again
would produce identical text. If any check fails, it returns an error rather than
unverified output. It uses the parser's existing input/depth limits; formatting
valid source near those limits can therefore return a verification error.

Tests cover operator precedence/associativity, execution equivalence, lambdas,
parameter patterns, guards, comments, supported grammar examples and every `.r`
file under `examples/scripts`. Unknown host names and unsupported runtime types
do not prevent formatting syntactically valid source.


### Shared host registration in editors

`analyze_editor_with_host(source, input_names, &functions)` accepts the same
`Rc<HostFunction>` objects as `Program::run_with_host`. Its `HostEditorAnalysis`
contains the checked parse, source references, lexical bindings and registered
functions. An embedding editor can read each function's `name`, `parameters` and
`result` for completion and signature help; these are the actual registration
objects, not a second signature catalogue. Analysis never calls their callbacks.

This entry point checks unknown names, known call arities, provably incompatible
host arguments and registration collisions with builtins, other functions or
input names. Registration errors use the empty source span `0..0` because the
conflict belongs to host configuration. Local script bindings can still shadow
host names. Dynamic values remain checked during execution. Syntax errors suppress
reference and lexical-binding metadata, while the host function catalogue remains
available for completion.


### Application-owned objects

`HostObject::new("Point", &owner)` creates an opaque, non-owning reference to an
`Rc<T>` owned by the application (`T: Any`). Return `Value::HostObject(reference)`
from a registered function with result type `ValueType::HostObject("Point")`.
Use the same type in function parameters, including inside list/tuple/Option
contracts. Host names are application-defined tags; callbacks must also use the
expected Rust type when calling `reference.upgrade::<T>()`.

Script aliases compare by object identity and type tag. They do not keep the
object alive or expose its fields. When the last strong Rust owner is dropped,
`is_alive()` is false and `upgrade()` returns None. Runtime argument/result checks
reject expired references, so an expired argument never reaches the callback.
A successful upgrade returns an `Rc<T>` that pins the object for that host operation;
dropping the application's owner does not invalidate an outstanding strong pin.
Expired references retain their identity and can still be compared or stored.

The editor uses the same `ValueType::HostObject` registrations for signatures and
known argument-type checks. Liveness is checked at execution time. This does not
introduce script-defined classes or type annotations for arbitrary host names.
The host controls mutations, capabilities and object memory; the runtime does not
account for that memory or forcibly revoke other strong owners.


`cargo run -p themoretheless-tokenizer-rush --example host_scene > triangle.obj`
runs a complete scene embedding example. The application owns points; Rush uses
`scene_point`, `position`, `translate_point` and `remove_point` registrations,
transforms a list with `map`, and creates a mesh for OBJ export. The same
registrations feed `analyze_editor_with_host`. The example validates the exported
vertices/triangle and an expired reference captured by a closure. Its tests also
check that a failed translation does not partially mutate the application object.
No renderer or implicit scene access is added to Rush.


### Explicit execution limits

`Program::run_with_limits(ExecutionLimits { max_depth: 32, ..ExecutionLimits::new(100_000) },
&cancellation, &inputs, &functions, &modules)` lets an embedding application lower
the evaluation-depth ceiling independently of the step budget. Existing `run`,
`run_with_host` and `run_with_modules` preserve their depth ceiling of 64.
`ExecutionLimits::new(steps)` uses that same ceiling; depths above 64 are rejected
before execution to preserve the runtime's native stack bound.

Depth counts nested expression evaluation, function bodies and imports, so it is
not simply the number of function calls. Limits are shared within one execution
and reset on the next execution. Cancellation tokens remain sticky. A zero step
budget or zero depth prevents evaluating an expression, including calling a host
function; an empty module can still return null without evaluating an expression.
These limits do not bound heap memory, host allocations or blocking host I/O.
A host callback that independently starts another Program run supplies that run's
own limits.


The CLI accepts execution limits after the script path:

```sh
rush script.r --steps 100000 --depth 32
rush --test tests --steps 100000 --depth 32
```

Defaults are 1,000,000 steps and depth 64. Each test file receives its own limits
in its child process. Values must be nonnegative integers; depth above 64, missing
values and repeated options are errors. `--check` rejects execution-limit options
because it does not execute code. Exhaustion reports the script/module source
location and exits unsuccessfully without printing a result.


`ExecutionLimits::max_collection_items` currently bounds list/tuple/record literals, eager `range`,
eager `map`/`filter`/`flat_map`, `zip`, geometry construction and the maximum requested
by `collect`. Its default is `usize::MAX` for compatibility.
A range errors before appending an element beyond the bound. A collect request
above the bound errors before opening/reading its sequence, even if that sequence
might contain fewer items. The source expression itself is evaluated normally.
Zero permits empty ranges and `collect(..., 0)`. Literal length is checked before
any element is evaluated. Eager transforms check before appending/extending output;
the callback producing an overflowing result has already run, so its side effects
are not rolled back. Flat-map errors close an open source without reading another
item. Each nested collection has its own length bound.

This is the first part of data-size enforcement: other constructors, element sizes,
aggregate live memory and host allocations are not bounded by this field yet.
It must not be treated as a heap sandbox or a process RSS limit.


For `grid_mesh`, both the vertex count (`xs * ys`) and triangle count
(`2 * (xs - 1) * (ys - 1)`) are checked with integer-overflow protection before
invoking the point callback. `mesh`, `transform`, polygon construction/translation/
rotation and mesh field conversion check collection sizes before constructing
outputs. `zip` checks both output length and its two-element tuples. Fixed-size
vector/matrix components are not treated as variable-length collections.

`ExecutionLimits.max_string_bytes` limits the UTF-8 byte length of decoded string
literals, string concatenation results and record keys. Escapes count by their decoded size:
`"\u{1f600}"` occupies four bytes. Checks happen before appending a character or
allocating a concatenation result. Zero permits empty strings; the default is
`usize::MAX`. Record names are checked before allocating a key; computed keys are checked
before evaluating the associated value. Strings returned by host callbacks and
host sequence readers are checked after allocation, as described below. This
does not bound allocator capacity or total live memory.

The CLI exposes these data limits as `--items N` and `--string-bytes N`:

```sh
rush scene.r --steps 100000 --depth 32 --items 10000 --string-bytes 65536
rush --test tests/rush --items 10000 --string-bytes 65536
```

Both default to unlimited and accept nonnegative integers, including zero.
`--test` forwards them to each script; `--check` rejects execution-only options.
These flags have the same per-value coverage as `ExecutionLimits` above.

With a finite data limit enabled, host callback results and host sequence items
are checked before entering the script. Nested lists, tuples, variants and records
are visited; strings, record keys and polygon/mesh sizes obey the same limits.
Traversal consumes execution steps and rejects nesting deeper than 64. These
checks run after the host has allocated its result. They do not bound allocations
inside callbacks, opaque objects, or values retained inside closures/sequences.

`Option`/`Result` payload slots have fixed arity and do not count as collection
items. Their payload data is still checked recursively at host boundaries, so
`Some("abc")` follows the same limits whether created by Rush or by a callback.

The playground web bridge uses 100,000 execution steps, depth 64, at most
10,000 items per collection and 65,536 UTF-8 bytes per string. Large lazy ranges
remain usable when consumers request a bounded prefix. These limits do not bound
total memory. Text, SVG and OBJ formatting stop before
their UTF-8 output exceeds 1 MiB and returns an output-limit error. JSON escaping
can make the final response larger than that text limit.

`Mesh::write_obj` streams OBJ into a caller-owned `fmt::Write` and propagates
writer errors immediately. `to_obj` remains the convenience API returning a String.

`Polygon::write_svg` streams SVG into `fmt::Write`, validates bounds before
writing, and reports writer failures. The playground caps this output at 1 MiB.

Editor binding metadata includes `members` for immutable bindings with a known
vector type, including aliases. Constructor calls, host return types, pipelines
and valid vector component accesses contribute known types. The playground uses
these fields for completion after a simple local name, respecting lexical
shadowing and inserting fields without call parentheses. It does not infer types
for mutable bindings or arbitrary member chains. A missing final member name (`v.` at end of input) preserves metadata for
completion while retaining its syntax error. Other incomplete syntax still
suppresses metadata.

Unary `-` negates each vector component; unary `+` returns the same vector value.
Both preserve dimension and leave other bindings unchanged. Static analysis keeps
the vector type through these operations for host argument checks and completion.

Immutable record literals with identifier keys expose their field names through
editor metadata as well. Immutable aliases preserve these names; completion
filters them by the typed prefix and inserts a plain field access. Computed or
string keys and mutable records are not inferred yet. Analysis follows known
nested record fields and preserves their value types. Completion follows known
dotted paths such as `config.camera.position.` and offers vector components.
Calls and indexing within the completion path are not supported yet.

### Record processing with JSON owned by the host

```sh
cargo run -p themoretheless-tokenizer-rush --example record_summary -- \
  crates/tokenizer-rush/examples/records.json
```

The host validates JSON rows and exposes `records()` as a typed list of
`(string, number, bool)` tuples. [The Rush script](examples/scripts/record-summary.r)
filters active rows, discovers groups in input order and computes each total.
The host serializes the returned records as JSON. The fixture produces totals
10.5 for `design` and 30 for `engineering`; `archived` is excluded.

This example keeps file access and JSON conversion outside the language.
`serde_json` is a development dependency used by the example. Input is limited
to 1000 rows after parsing; file reading and JSON parsing are host allocations
and do not have a byte limit here. Rush has explicit step, collection and string
limits. Aggregation uses `fold_by` and retains only the total per group. The host input
list and the eager filter result are still buffered in this example.
The thread-local host storage is cleared after execution, including runtime errors.

### Grouping collections

`group_by(source, key)` consumes a list or sequence and returns a list of records
`{key: string, values: list}`. The callback runs once per input element and must
return a string. Groups follow first appearance; items retain input order.
The implementation uses a tree lookup (O(log G) comparisons per item for G groups,
plus string comparison cost). It buffers all elements; this is an eager consumer,
including for a lazy source. It does not establish an aggregate heap limit.

```text
[3,2,1,4] | group_by(x => if x % 2 == 0 { 'even' } else { 'odd' })
// [{key:'odd', values:[3,1]}, {key:'even', values:[2,4]}]
```

Collection limits apply to the number of groups, each group's values, and the two
fields of each result record. String limits apply to keys and generated field names
(`values` needs six bytes). Step budget and cancellation remain active while reading,
calling the key function and constructing results. Host iterators close on completion,
error or cancellation. Callback side effects already performed are not rolled back.

### Aggregating groups without retaining their rows

`fold_by(source, key, initial, reducer)` consumes a list or sequence and returns
records `{key: string, value: accumulator}` in first-key order. For each element,
`key(element)` is called once, followed by `reducer(accumulator, element)` for
that group. Each new group starts with a clone of `initial` using ordinary Rush
value semantics; captured mutable cells or host object references keep their
normal sharing behavior. Empty input returns `[]`; both callbacks must be callable.

```text
range_iter(0,10000) | fold_by(x => 'all', 0, (sum,x) => sum+x)
// [{key:'all', value:49995000}]
```

Unlike `group_by`, the operation does not retain input elements automatically.
Memory depends on the number of distinct keys and the accumulator values chosen
by the reducer. A reducer that builds a list still buffers rows. Existing limits
apply to group count, record fields, key strings, and values constructed by the
callbacks; this does not provide an aggregate heap bound. The generated field
`value` requires five bytes under the string limit. Errors and cancellation close
host sources and do not roll back callback side effects.

### Streaming the same report from JSON Lines

```sh
cargo run -p themoretheless-tokenizer-rush --example record_summary -- \
  --jsonl crates/tokenizer-rush/examples/records.jsonl
```

This mode exposes a `HostSequence` instead of a list. The same Rush script lazily
filters rows and uses `fold_by` to retain only a total for each group. The fixture
produces the same report as the JSON-array mode. Each invocation opens its own
reader for the host-selected file; the reader owns the file until dropped.

Each physical line must contain one valid record. Blank lines are errors. UTF-8,
CRLF, and a final line without a newline are supported. The line limit is 65536
bytes including any line terminator; reading stops at limit+1 bytes to detect an
oversized line before parsing. JSON and record-schema errors include the 1-based
line number. The ordinary JSON-array mode still reads the entire input file.

Cancellation is checked before and after reading. A blocking filesystem read
cannot be interrupted by this token; the host must supply a cancellable I/O
implementation when that is required. The script keeps the existing 10-million
step limit, at most 1000 groups, and string/collection limits. The host reader
and parser have their own bounded per-line allocations, not a shared process
heap budget. A test processes 10000 records into two groups with exact totals.

### Persistent script instances

`Program::instantiate` executes top-level initialization once and returns a
`ScriptInstance`. Keep it between frames; `mut` variables and captured cells,
import caches, and host registrations survive calls. Named lifecycle functions
are ordinary Rush functions:

```rust
use themoretheless_tokenizer_rush::{CancellationToken, ExecutionLimits, Program, Value};
let program = Program::compile("mut elapsed = 0; fn update(delta) { elapsed += delta; return elapsed }")?;
let cancellation = CancellationToken::default();
let limits = ExecutionLimits::new(10_000);
let mut script = program.instantiate(limits, &cancellation, &[], &[], &[], &[])?;
assert_eq!(script.call("update", &[Value::Number(0.5)], limits)?, Value::Number(0.5));
assert_eq!(script.call("update", &[Value::Number(0.5)], limits)?, Value::Number(1.0));
# Ok::<(), themoretheless_tokenizer_rush::RuntimeError>(())
```

The arguments to `instantiate` are limits, cancellation, value inputs, ordinary
host functions, contextual host registrations, and modules. `initial_value()`
returns the initialization result; `get(name)` reads an instance binding.
`call` accepts any `Value`, including `HostObject`, records, lists, and events,
and resets the execution limits for each invocation. `start`, `update`,
`fixed_update`, and event handler names have no special automatic behavior;
the embedding engine chooses when to invoke them. Errors preserve prior
mutations; they do not roll back a frame. Instances are independent and borrow
the script source and cancellation token, rather than the `Program` itself.

For scene-specific callbacks, construct `HostRegistration::new(name,
parameter_types, result_type, move |arguments, cancellation| { ... })` and pass
it in the contextual registrations slice. Capture `Rc<RefCell<Scene>>` or a
command queue to mutate host state without globals. Signatures, cancellation,
and output size checks use the same path as ordinary host functions; pass
`registration.function` to editor analysis to expose its signature.

Run `cargo run -p themoretheless-tokenizer-rush --example scene_motion` for a
complete embedding example. Its `scripts/scene-motion.r` keeps position and
velocity between calls, while a captured Rust scene receives each position.
The example checks 120 frames, a velocity-change event, and the final position
after frame 121 against an independent expected result. It uses the same host
signature for editor analysis and execution.

`Program::run_with_values` supplies arbitrary value inputs for one-shot
execution, with limits, cancellation, host functions, and modules. Existing
numeric `run` methods and `HostFunction` struct literals remain compatible.
