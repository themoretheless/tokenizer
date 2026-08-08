# themoretheless-tokenizer-core

Shared primitives for [themoretheless-tokenizer](https://github.com/themoretheless/tokenizer) language plugins:

- UTF-8 byte [`Span`](https://docs.rs/themoretheless-tokenizer-core) and source positions
- Diagnostics, language / dialect ids, capability flags
- Host-facing token DTOs and a static language registry

Language engines (`json`, `url`, …) depend on this crate. Hosts (playground, WASM, editors) use the registry and `HostLanguage` facade.

See `docs/plugin-api-design.md` in the repository for the full plugin contract.
