//! Forma language frontend, shared by native applications and Studio/WASM.
pub mod diagnostic;
pub mod evaluate;
pub mod frontend;
pub mod host;
pub mod scene;
pub mod semantics;
pub mod typecheck;

pub use diagnostic::Diagnostic;
pub use frontend::{
    Error, Token, format_source, lint_document, lint_document_with_context, parse,
    parse_expression, parse_with_options, tokenize, tokenize_with_trivia,
};
pub use host::{DESCRIPTOR, ENGINE, FormaHost, Host, LANGUAGE_ID};
pub use typecheck::TypeContext;
