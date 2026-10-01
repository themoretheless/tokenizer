//! Canonical AST formatting with comment preservation and a parse-back guard.
use crate::{Block, Expr, ExprKind, Name, Stmt, StmtKind, Type, parse};
use themoretheless_tokenizer_core::{Span, SyntaxKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError {
    pub span: Span,
    pub message: String,
}

/// Format syntactically valid Rush without executing it. Never returns unverified output.
pub fn format_source(source: &str) -> Result<String, FormatError> {
    let parsed = parse(source);
    if !parsed.is_valid() {
        let diagnostic = parsed.diagnostics.first();
        return Err(FormatError {
            span: diagnostic.map_or(Span::new(0, 0), |d| d.span),
            message: diagnostic
                .map_or_else(|| "Invalid Rush syntax".into(), |d| d.message.to_string()),
        });
    }
    let output = Writer::new(comments(&parsed)).module(&parsed.module.items);
    let reparsed = parse(&output);
    if !reparsed.is_valid()
        || Writer::new(vec![]).module(&parsed.module.items)
            != Writer::new(vec![]).module(&reparsed.module.items)
        || !comments(&parsed)
            .iter()
            .map(|comment| comment.text)
            .eq(comments(&reparsed).iter().map(|comment| comment.text))
    {
        return Err(FormatError {
            span: parsed.module.span,
            message: "Formatter could not preserve the program".into(),
        });
    }
    if Writer::new(comments(&reparsed)).module(&reparsed.module.items) != output {
        return Err(FormatError {
            span: parsed.module.span,
            message: "Formatter could not produce a stable layout".into(),
        });
    }
    Ok(output)
}

fn comments<'s>(parsed: &crate::Parse<'s>) -> Vec<Comment<'s>> {
    parsed
        .lexed
        .tokens
        .iter()
        .filter(|token| {
            matches!(
                token.kind,
                SyntaxKind::LineComment | SyntaxKind::BlockComment
            )
        })
        .map(|token| Comment {
            span: token.span,
            text: &parsed.source[token.span.start..token.span.end],
            line: token.kind == SyntaxKind::LineComment,
        })
        .collect()
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for FormatError {}

struct Comment<'s> {
    span: Span,
    text: &'s str,
    line: bool,
}
struct Writer<'s> {
    output: String,
    indent: usize,
    comments: Vec<Comment<'s>>,
    next: usize,
}
impl<'s> Writer<'s> {
    fn new(comments: Vec<Comment<'s>>) -> Self {
        Self {
            output: String::new(),
            indent: 0,
            comments,
            next: 0,
        }
    }
    fn text(&mut self, text: &str) {
        if self.output.ends_with('\n') && !text.is_empty() {
            self.output.push_str(&"    ".repeat(self.indent));
        }
        self.output.push_str(text);
    }
    fn newline(&mut self) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }
    fn before(&mut self, offset: usize) {
        while self
            .comments
            .get(self.next)
            .is_some_and(|c| c.span.start < offset)
        {
            let comment = &self.comments[self.next];
            let (text, line) = (comment.text, comment.line);
            if !self.output.is_empty() && !self.output.ends_with([' ', '\n']) {
                self.text(" ");
            }
            self.text(text);
            if line {
                self.newline();
            } else {
                self.text(" ");
            }
            self.next += 1;
        }
    }
    fn module(mut self, statements: &[Stmt<'s>]) -> String {
        for statement in statements {
            self.statement(statement);
        }
        self.before(usize::MAX);
        if !self.output.is_empty() {
            self.newline();
        }
        self.output
    }
    fn name(&mut self, name: &Name<'s>) {
        self.before(name.span.start);
        self.text(name.text);
    }
    fn ty(&mut self, ty: &Type<'s>) {
        self.name(&ty.name);
        if !ty.arguments.is_empty() {
            self.text("[");
            for (i, ty) in ty.arguments.iter().enumerate() {
                if i > 0 {
                    self.text(", ");
                }
                self.ty(ty);
            }
            self.text("]");
        }
    }
    fn block(&mut self, block: &Block<'s>) {
        self.text("{");
        self.newline();
        self.indent += 1;
        for statement in &block.stmts {
            self.statement(statement);
        }
        self.before(block.span.end);
        self.newline();
        self.indent -= 1;
        self.text("}");
    }
    fn statement(&mut self, statement: &Stmt<'s>) {
        self.before(statement.span.start);
        match &statement.kind {
            StmtKind::Import(name) => {
                self.text("import ");
                self.name(name);
            }
            StmtKind::Destructure { pattern, value } => {
                self.text("let ");
                self.pattern(pattern);
                self.text(" = ");
                self.expr(value, 0);
            }
            StmtKind::Declaration {
                name,
                constant,
                ty,
                value,
            } => {
                self.text(if *constant { "let " } else { "mut " });
                self.name(name);
                if let Some(ty) = ty {
                    self.text(": ");
                    self.ty(ty);
                }
                self.text(" = ");
                self.expr(value, 0);
            }
            StmtKind::Function {
                name,
                parameters,
                result,
                body,
            } => {
                self.text("fn ");
                self.name(name);
                self.text("(");
                for (i, parameter) in parameters.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.pattern(&parameter.pattern);
                    if let Some(ty) = &parameter.ty {
                        self.text(": ");
                        self.ty(ty);
                    }
                }
                self.text(")");
                if let Some(ty) = result {
                    self.text(" -> ");
                    self.ty(ty);
                }
                self.text(" ");
                self.block(body);
            }
            StmtKind::Return(value) => {
                self.text("return");
                if let Some(value) = value {
                    self.text(" ");
                    self.expr(value, 0);
                }
            }
            StmtKind::Yield(value) => {
                self.text("yield ");
                self.expr(value, 0);
            }
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.text("if ");
                self.expr(condition, 0);
                self.text(" ");
                self.block(then_block);
                if let Some(block) = else_block {
                    self.text(" else ");
                    self.block(block);
                }
            }
            StmtKind::While { condition, body } => {
                self.text("while ");
                self.expr(condition, 0);
                self.text(" ");
                self.block(body);
            }
            StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                self.text("for ");
                self.name(binding);
                self.text(" in ");
                self.expr(iterable, 0);
                self.text(" ");
                self.block(body);
            }
            StmtKind::Break => self.text("break"),
            StmtKind::Continue => self.text("continue"),
            StmtKind::Expr(value) => self.expr(value, 1),
            StmtKind::Error => unreachable!("invalid input is rejected"),
        }
        if !matches!(
            statement.kind,
            StmtKind::Function { .. }
                | StmtKind::If { .. }
                | StmtKind::While { .. }
                | StmtKind::For { .. }
        ) {
            self.text(";");
        }
        self.before(statement.span.end);
        self.newline();
    }
    fn pattern(&mut self, pattern: &Expr<'s>) {
        self.before(pattern.span.start);
        match &pattern.kind {
            ExprKind::Name(name) => self.name(name),
            ExprKind::Tuple(items) => {
                self.text("(");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.pattern(item);
                }
                if items.len() == 1 {
                    self.text(",");
                }
                self.text(")");
            }
            ExprKind::Map(items) => {
                self.text("{");
                for (i, (key, value)) in items.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.pattern(key);
                    self.text(": ");
                    self.pattern(value);
                }
                self.text("}");
            }
            ExprKind::Call { callee, arguments } => {
                self.pattern(callee);
                self.text("(");
                for (i, item) in arguments.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.pattern(item);
                }
                self.text(")");
            }
            _ => self.expr(pattern, 0),
        }
        self.before(pattern.span.end);
    }
    fn expr(&mut self, expression: &Expr<'s>, minimum: u8) {
        self.before(expression.span.start);
        let precedence = match &expression.kind {
            ExprKind::Assign { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::If { .. }
            | ExprKind::Match { .. } => 0,
            ExprKind::Pipeline { .. } => 1,
            ExprKind::Binary { operator, .. } => crate::parser::binding(operator).unwrap().0,
            ExprKind::Unary { .. } => 13,
            ExprKind::Call { .. } | ExprKind::Member { .. } | ExprKind::Index { .. } => 20,
            _ => 21,
        };
        // Keeping comments with newlines inside grouping preserves expression continuation.
        let multiline_comment = self.comments[self.next..]
            .iter()
            .take_while(|c| c.span.start < expression.span.end)
            .any(|c| {
                c.span.start >= expression.span.start
                    && c.span.end <= expression.span.end
                    && (c.line || c.text.contains('\n'))
            });
        let parens = precedence < minimum || multiline_comment;
        if parens {
            self.text("(");
        }
        match &expression.kind {
            ExprKind::Name(name) => self.name(name),
            ExprKind::Number(text) | ExprKind::String(text) => self.text(text),
            ExprKind::Bool(value) => self.text(if *value { "true" } else { "false" }),
            ExprKind::Null => self.text("null"),
            ExprKind::Unary { operator, value } => {
                self.text(operator);
                self.text(" ");
                self.expr(value, 13);
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let (left_bp, right_bp) = crate::parser::binding(operator).unwrap();
                self.expr(left, left_bp + u8::from(left_bp == right_bp));
                self.text(" ");
                self.text(operator);
                self.text(" ");
                self.expr(right, right_bp);
            }
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                self.expr(target, 1);
                self.text(" ");
                self.text(operator);
                self.text(" ");
                self.expr(value, 0);
            }
            ExprKind::Call { callee, arguments } => {
                self.expr(callee, 20);
                self.text("(");
                self.items(arguments);
                self.text(")");
            }
            ExprKind::Member { object, field } => {
                self.expr(object, 20);
                self.text(".");
                self.name(field);
            }
            ExprKind::Index { object, index } => {
                self.expr(object, 20);
                self.text("[");
                self.expr(index, 0);
                self.text("]");
            }
            ExprKind::List(items) => {
                self.text("[");
                self.items(items);
                self.text("]");
            }
            ExprKind::Tuple(items) => {
                self.text("(");
                self.items(items);
                if items.len() == 1 {
                    self.text(",");
                }
                self.text(")");
            }
            ExprKind::Map(items) => {
                self.text("{");
                for (i, (key, value)) in items.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.expr(key, 0);
                    self.text(": ");
                    self.expr(value, 0);
                }
                self.text("}");
            }
            ExprKind::Lambda { parameters, body } => {
                self.text("(");
                for (i, pattern) in parameters.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.pattern(pattern);
                }
                self.text(") => ");
                self.expr(body, 0);
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                self.text("if ");
                self.expr(condition, 0);
                self.text(" { ");
                self.expr(then_value, 0);
                self.text(" } else { ");
                self.expr(else_value, 0);
                self.text(" }");
            }
            ExprKind::Pipeline { input, stages } => {
                self.expr(input, 2);
                for stage in stages {
                    self.text(" | ");
                    self.expr(stage, 2);
                }
            }
            ExprKind::Match { value, arms } => {
                self.text("match ");
                self.expr(value, 0);
                self.text(" {");
                self.newline();
                self.indent += 1;
                for arm in arms {
                    self.before(arm.span.start);
                    self.pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.text(" if (");
                        self.expr(guard, 0);
                        self.text(")");
                    }
                    self.text(" => ");
                    self.expr(&arm.value, 0);
                    self.text(",");
                    self.before(arm.span.end);
                    self.newline();
                }
                self.before(expression.span.end);
                self.newline();
                self.indent -= 1;
                self.text("}");
            }
            ExprKind::Error => unreachable!("invalid input is rejected"),
        }
        self.before(expression.span.end);
        if parens {
            self.text(")");
        }
    }
    fn items(&mut self, items: &[Expr<'s>]) {
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.text(", ");
            }
            self.expr(item, 0);
        }
    }
}
