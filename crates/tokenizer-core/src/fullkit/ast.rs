//! Borrowing AST nodes for the shared recovering parser.

use crate::{Diagnostic, Span};

use super::lexer::Lexed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module<'s> {
    pub span: Span,
    pub items: Vec<Item<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item<'s> {
    Function {
        span: Span,
        name: Option<&'s str>,
        name_span: Option<Span>,
        body: Block<'s>,
    },
    Class {
        span: Span,
        name: Option<&'s str>,
        name_span: Option<Span>,
        body: Block<'s>,
    },
    Statement(Stmt<'s>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'s> {
    pub span: Span,
    pub stmts: Vec<Stmt<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt<'s> {
    Expr {
        span: Span,
        expr: Expr<'s>,
    },
    Return {
        span: Span,
        value: Option<Expr<'s>>,
    },
    If {
        span: Span,
        cond: Option<Expr<'s>>,
        then_block: Block<'s>,
        else_block: Option<Block<'s>>,
    },
    While {
        span: Span,
        cond: Option<Expr<'s>>,
        body: Block<'s>,
    },
    For {
        span: Span,
        body: Block<'s>,
    },
    Declaration {
        span: Span,
        name: Option<&'s str>,
        name_span: Option<Span>,
        value: Option<Expr<'s>>,
    },
    Error {
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr<'s> {
    Ident {
        span: Span,
        name: &'s str,
    },
    Literal {
        span: Span,
        kind: LitKind,
        text: &'s str,
    },
    Call {
        span: Span,
        callee: Box<Expr<'s>>,
        args: Vec<Expr<'s>>,
    },
    Binary {
        span: Span,
        left: Box<Expr<'s>>,
        op_span: Span,
        right: Box<Expr<'s>>,
    },
    Unary {
        span: Span,
        op_span: Span,
        expr: Box<Expr<'s>>,
    },
    Member {
        span: Span,
        object: Box<Expr<'s>>,
        field: Option<&'s str>,
        field_span: Option<Span>,
    },
    Index {
        span: Span,
        object: Box<Expr<'s>>,
        index: Box<Expr<'s>>,
    },
    Paren {
        span: Span,
        expr: Box<Expr<'s>>,
    },
    List {
        span: Span,
        elements: Vec<Expr<'s>>,
    },
    Map {
        span: Span,
        entries: Vec<(Expr<'s>, Expr<'s>)>,
    },
    Error {
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    String,
    Number,
    Bool,
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'s> {
    pub source: &'s str,
    pub lexed: Lexed,
    pub module: Module<'s>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parse<'_> {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && self.lexed.diagnostics.is_empty()
    }
}

#[inline]
pub(crate) fn expr_span<'s>(e: &Expr<'s>) -> Span {
    match e {
        Expr::Ident { span, .. }
        | Expr::Literal { span, .. }
        | Expr::Call { span, .. }
        | Expr::Binary { span, .. }
        | Expr::Unary { span, .. }
        | Expr::Member { span, .. }
        | Expr::Index { span, .. }
        | Expr::Paren { span, .. }
        | Expr::List { span, .. }
        | Expr::Map { span, .. }
        | Expr::Error { span } => *span,
    }
}

#[inline]
pub(crate) fn stmt_span<'s>(s: &Stmt<'s>) -> Span {
    match s {
        Stmt::Expr { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Declaration { span, .. }
        | Stmt::Error { span } => *span,
    }
}

#[inline]
pub(crate) fn item_span<'s>(i: &Item<'s>) -> Span {
    match i {
        Item::Function { span, .. } | Item::Class { span, .. } => *span,
        Item::Statement(s) => stmt_span(s),
    }
}
