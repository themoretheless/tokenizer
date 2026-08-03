//! Production-oriented JSON lexing and parsing.
//!
//! The legacy [`crate::tokenize_json`] API remains available for semantic
//! highlighting. This module provides exact syntax tokens and a typed AST.

mod ast;
mod lexer;
mod navigation;
mod parser;
mod semantic;
mod syntax;
mod visitor;

pub use ast::{
    Array, Boolean, Member, Null, Number, NumberError, Object, StringValue, Value, ValueKind,
};

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
