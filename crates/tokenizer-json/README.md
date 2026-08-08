# themoretheless-tokenizer-json

JSON and JSONC syntax engine for the themoretheless-tokenizer workspace:

- lossless lexer (`lex` / `lex_with`)
- recovering parser and borrowing AST (`parse` / `parse_with`)
- semantic tokens, CST, navigation, visitor

Depends only on `themoretheless-tokenizer-core` (no other runtime deps).

The facade crate `themoretheless-tokenizer` re-exports this API as
`themoretheless_tokenizer::json`.
