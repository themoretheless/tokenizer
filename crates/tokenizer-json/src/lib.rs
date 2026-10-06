//! Production-oriented JSON and JSONC lexing and parsing.
//!
//! Exact syntax tokens, a recovering parser, borrowing AST, semantic tokens,
//! CST, navigation, and visitor APIs. The facade crate re-exports this module
//! as `themoretheless_tokenizer::json`.

mod ast;
mod host;
mod jsonl;
mod lexer;
mod navigation;
mod parser;
mod semantic;
mod syntax;
mod visitor;

pub use ast::{
    Array, Boolean, Member, Null, Number, NumberError, Object, StringValue, Value, ValueKind,
};
pub use host::{JSON5_ENGINE, JSONL_ENGINE, Json5Host, JsonlHost};

pub use jsonl::{Jsonl, Record, parse as parse_jsonl, parse_records};
pub use lexer::{
    LexDiagnostic, LexDiagnosticKind, LexToken, Lexed, LexerOptions, NumberIssue, SyntaxKind,
    TokenFlags, lex, lex_with,
};
pub use navigation::{
    AstPath, AstPathSegment, NavigationError, NodeRef, node_at_offset, path_at_offset,
};
pub use parser::{
    MAX_SUPPORTED_DEPTH, Parse, ParseDiagnostic, ParseDiagnosticKind, ParseOptions, parse,
    parse_with,
};
pub use semantic::{
    SemanticKind, SemanticToken, SemanticTokenization, semantic_tokens, tokenize, tokenize_with,
};
pub use syntax::{
    EditError, NodeId, SyntaxElement, SyntaxNode, SyntaxNodeKind, SyntaxTree, TextEdit, TokenId,
    apply_edits, syntax_tree, syntax_tree_with,
};
pub use visitor::{AstVisitor, VisitContext, VisitControl, VisitOutcome, visit_parse, visit_value};
