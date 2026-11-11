//! AST-driven semantic token projection (`O(N + M)` monotonic cursor walk).

use std::borrow::Cow;

use crate::{Diagnostic, HostDiagnostic, HostSpan, HostToken, HostTokenization, Span};

use super::{
    ast::{Block, Item, Module, Parse, Stmt},
    lexer::SyntaxKind,
    parser::parse_full,
    profile::FullProfile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticToken {
    pub kind: &'static str,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticTokenization {
    pub tokens: Vec<SemanticToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl SemanticTokenization {
    #[must_use]
    pub fn to_host(self) -> HostTokenization {
        let valid = self.diagnostics.is_empty();
        HostTokenization {
            tokens: self
                .tokens
                .into_iter()
                .map(|t| HostToken {
                    kind: Cow::Borrowed(t.kind),
                    span: HostSpan::from(t.span),
                    error: t.kind == "error" || t.kind == "invalid",
                })
                .collect(),
            diagnostics: self
                .diagnostics
                .into_iter()
                .map(HostDiagnostic::from_diagnostic)
                .collect(),
            valid,
        }
    }
}

fn build_semantic_tokens(parse: &Parse<'_>) -> Vec<SemanticToken> {
    let mut name_spans: Vec<(Span, &'static str)> = Vec::new();
    collect_names(&parse.module, &mut name_spans);

    let mut tokens = Vec::with_capacity(parse.lexed.tokens.len());
    let mut cursor = 0usize;
    for t in &parse.lexed.tokens {
        let mut kind = t.kind.as_str();
        if matches!(t.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent) {
            while cursor < name_spans.len() && name_spans[cursor].0.start < t.span.start {
                cursor += 1;
            }
            if cursor < name_spans.len() && name_spans[cursor].0 == t.span {
                kind = name_spans[cursor].1;
            }
        }
        tokens.push(SemanticToken { kind, span: t.span });
    }
    tokens
}

/// Semantic tokens: rewrite identifier kinds using AST context (function/class/variable names).
#[must_use]
pub fn semantic_full(parse: &Parse<'_>) -> SemanticTokenization {
    SemanticTokenization {
        tokens: build_semantic_tokens(parse),
        diagnostics: parse.diagnostics.clone(),
    }
}

#[inline]
pub(crate) fn semantic_full_owned(parse: Parse<'_>) -> SemanticTokenization {
    let tokens = build_semantic_tokens(&parse);
    SemanticTokenization {
        tokens,
        diagnostics: parse.diagnostics,
    }
}

/// One-shot full pipeline for host semantic layer.
#[must_use]
pub fn analyze_full_host(source: &str, profile: &FullProfile) -> HostTokenization {
    let parsed = parse_full(source, profile);
    semantic_full_owned(parsed).to_host()
}

fn collect_names<'s>(module: &Module<'s>, out: &mut Vec<(Span, &'static str)>) {
    for item in &module.items {
        match item {
            Item::Function {
                name_span, body, ..
            } => {
                if let Some(s) = name_span {
                    out.push((*s, "function"));
                }
                collect_block(body, out);
            }
            Item::Class {
                name_span, body, ..
            } => {
                if let Some(s) = name_span {
                    out.push((*s, "class"));
                }
                collect_block(body, out);
            }
            Item::Statement(s) => collect_stmt(s, out),
        }
    }
}

fn collect_block<'s>(b: &Block<'s>, out: &mut Vec<(Span, &'static str)>) {
    for s in &b.stmts {
        collect_stmt(s, out);
    }
}

fn collect_stmt<'s>(s: &Stmt<'s>, out: &mut Vec<(Span, &'static str)>) {
    match s {
        Stmt::Declaration {
            name_span: Some(sp),
            ..
        } => {
            out.push((*sp, "variable"));
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            collect_block(then_block, out);
            if let Some(e) = else_block {
                collect_block(e, out);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => collect_block(body, out),
        _ => {}
    }
}
