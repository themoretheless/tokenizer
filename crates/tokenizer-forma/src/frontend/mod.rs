//! Complete Forma component/design frontend.
//!
//! Modularized by responsibility:
//! - `lexer`: UTF-16 token & trivia comment scanner
//! - `ast`: transport AST helpers and expression span tables
//! - `parser`: recursive-descent component, design, and expression parser
//! - `format`: canonical value serializer and comment-preserving source formatter
//! - `lint`: AST reference collector and document linter

mod ast;
mod format;
mod lexer;
mod lint;
mod parser;

pub(crate) use ast::{SpanTable, is_pure_function};
pub use format::{format_source, serialize_value};
pub(crate) use lexer::{BorrowedToken, Comment, tokenize_borrowed};
pub use lexer::{Error, Token, tokenize, tokenize_with_trivia};
pub use lint::{lint_document, lint_document_with_context};
pub(crate) use parser::parse_with_span_table;
pub use parser::{parse, parse_expression, parse_with_options};
