# themoretheless-tokenizer

Независимая Rust-библиотека токенизации. Она не зависит от WebAssembly,
JavaScript, UI-фреймворков или `serde` и может использоваться в нативном Rust,
на сервере и внутри WASM-приложений.

Пока встроен JSON-токенизатор. Публичные диапазоны задаются в UTF-8 байтах,
поэтому ими можно безопасно индексировать исходную Rust-строку.

```rust
use themoretheless_tokenizer::{JsonTokenizer, TokenKind, Tokenizer};

let source = r#"{"name":"Denis","active":true}"#;
let result = JsonTokenizer.tokenize(source);

assert!(result.diagnostics.is_empty());
assert!(result.tokens.iter().any(|token| {
    token.kind == TokenKind::Property && token.text(source) == Some("\"name\"")
}));
```

Подключение из Git:

```toml
[dependencies]
themoretheless-tokenizer = { git = "https://github.com/themoretheless/tokenizer.git" }
```

Имя Cargo-пакета содержит дефисы, а имя Rust-модуля использует подчёркивания:
`themoretheless_tokenizer`.

