//! Production-oriented JSON lexing and parsing.
//!
//! The legacy [`crate::tokenize_json`] API remains available for semantic
//! highlighting. This module provides exact syntax tokens and a typed AST.

mod ast;
mod lexer;
mod parser;
mod semantic;

pub use ast::{
    Array, Boolean, Member, Null, Number, NumberError, Object, StringValue, Value, ValueKind,
};

pub use lexer::{
    LexDiagnostic, LexDiagnosticKind, LexToken, Lexed, LexerOptions, NumberIssue, SyntaxKind,
    TokenFlags, lex, lex_with,
};
pub use parser::{
    MAX_SUPPORTED_DEPTH, Parse, ParseDiagnostic, ParseDiagnosticKind, ParseOptions, parse,
    parse_with,
};
pub use semantic::{
    SemanticKind, SemanticToken, SemanticTokenization, semantic_tokens, tokenize, tokenize_with,
};
