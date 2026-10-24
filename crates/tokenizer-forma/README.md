# Forma language

Full Rust frontend for Forma `.ui`: components/designs, typed declarations,
bindings/events, expressions, match, if/for, contracts and bounded evaluation.
`parse` / `parse_expression` return the Studio-compatible JSON AST. Public
`evaluate` and `semantics` modules execute it with explicit host data/resolvers.
Editor offsets are UTF-16 units; lexical tokens retain their original text.

`scene` contains the previous native renderer subset. Use `scene::parse` for
that API. It is deliberately separate from the complete language frontend.

## Design boundaries

Input is deliberately bounded: sources up to 4 MiB and one million tokens,
recursive parsing to 32 frames, expression execution to 128 levels, and
expansion to 64 levels / 16,384 visited nodes. `scene` is an intentional
subset of the language (the previous native renderer surface), not an
incomplete parser; keep it separate from the full frontend.

`cargo test -p themoretheless-tokenizer-forma` verifies the 87-file compatibility
corpus and regressions. `packages/forma/wasm` exposes this crate to Studio; the
JavaScript modules are adapters, not a second implementation of the language.
