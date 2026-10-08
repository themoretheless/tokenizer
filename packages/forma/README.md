# Forma language — Rust frontend

The complete `.ui` component/design language is implemented in
`crates/tokenizer-forma`: tokenization, parsing, expression evaluation, component
scopes, typed contracts, match selection, design state patches and keyed
conditional/list expansion. `scene` retains the native renderer's old scene
subset under its existing API.

`src/language.js`, `src/expressions.js` and `src/component-semantics.js` are
WASM/host adapters. They contain no markup parser or expression interpreter.
Studio still owns rendering, component linking, source annotations and mutable
host model references. Lezer's generated editor tree remains an editor adapter;
it is not used to compile or execute `.ui` documents.

```rust
let document = themoretheless_tokenizer_forma::parse(
    "component Hello { Text { text: 'Hello'; } }"
)?;
```

```js
import {parse} from '@themoretheless/tokenizer-forma';
const document = parse("component Hello { Text { text: 'Hello'; } }");
```

The JSON-compatible AST intentionally preserves Studio's existing fields.
Editor offsets use UTF-16 units; native scene parsing keeps its separate API.
Source input is bounded to 4 MiB / one million tokens, recursive parsing to 32
frames, expression execution to 128 levels, and expansion to 64 levels / 16,384
visited nodes. Input passed through the WASM JSON bridge is also bounded by
serde's JSON nesting limit. Host references are explicit; there is no JavaScript
`eval`, arbitrary native execution, or implicit conversion to Bool.

Run `npm ci && npm test` here. `prepare` builds the Rust WASM module; it needs
Rust's `wasm32-unknown-unknown` target and wasm-bindgen CLI 0.2.125. Rebuild with
`npm run build:wasm` after Rust changes. Run `npm run generate` for Lezer changes.
Rust tests include an AST compatibility corpus captured from all 87 original
Forma `.ui` files plus invalid inputs, UTF-16 offsets, runtime errors, cyclic
properties and keyed collections.

For the current local integration, check out `forma` and `tokenizer` beside one
another. Registry publication/version pinning is still needed for independent
repository builds. See [the language contract](LANGUAGE.md).
