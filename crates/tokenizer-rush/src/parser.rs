use crate::ast::*;
use themoretheless_tokenizer_core::{Diagnostic, InputLimits, LexToken, Lexed, Span, SyntaxKind};

#[derive(Clone, Copy)]
struct Token {
    raw: LexToken,
    indent: usize,
    tabs: bool,
}

pub(crate) fn run(source: &str, lexed: Lexed, valid: bool, limits: InputLimits) -> Parse<'_> {
    let mut starts = vec![0];
    for (i, byte) in source.bytes().enumerate() {
        if byte == b'\n' || (byte == b'\r' && source.as_bytes().get(i + 1) != Some(&b'\n')) {
            starts.push(i + 1);
        }
    }
    let tokens = lexed
        .tokens
        .iter()
        .filter(|t| !t.kind.is_trivia())
        .map(|raw| {
            let line = starts.partition_point(|&start| start <= raw.span.start) - 1;
            let prefix = &source[starts[line]..raw.span.start];
            let indent = prefix
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            Token {
                raw: *raw,
                indent,
                tabs: prefix[..indent].contains('\t'),
            }
        })
        .collect();
    let mut parser = Parser {
        source,
        tokens,
        pos: 0,
        diagnostics: lexed.diagnostics.clone(),
        valid,
        limits,
        depth: 0,
        functions: 0,
        loops: 0,
        roles: Vec::new(),
    };
    let items = parser.sequence(None, Some(0));
    Parse {
        source,
        lexed,
        module: Module {
            span: Span::new(0, source.len()),
            items,
        },
        diagnostics: parser.diagnostics,
        valid: parser.valid,
        roles: parser.roles,
    }
}

struct Parser<'s> {
    source: &'s str,
    tokens: Vec<Token>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    valid: bool,
    limits: InputLimits,
    depth: usize,
    functions: usize,
    loops: usize,
    roles: Vec<(Span, &'static str)>,
}

impl<'s> Parser<'s> {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }
    fn text(&self) -> &'s str {
        self.peek().map_or("", |t| &self.source[t.raw.span.range()])
    }
    fn at(&self, s: &str) -> bool {
        self.peek().is_some() && self.text() == s
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.peek()?;
        self.pos += 1;
        Some(t)
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.at(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn span(&self) -> Span {
        self.peek()
            .map_or(Span::new(self.source.len(), self.source.len()), |t| {
                t.raw.span
            })
    }
    fn end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .map_or(0, |i| self.tokens[i].raw.span.end)
    }
    fn newline(&self) -> bool {
        if self.pos == 0 {
            return true;
        }
        self.peek()
            .is_some_and(|t| self.source[self.end()..t.raw.span.start].contains(['\n', '\r']))
    }
    fn error(&mut self, code: &'static str, message: &'static str) {
        self.error_at(self.span(), code, message);
    }
    fn error_at(&mut self, span: Span, code: &'static str, message: &'static str) {
        self.valid = false;
        if self.diagnostics.len() < self.limits.max_diagnostics {
            self.diagnostics.push(Diagnostic::new(span, code, message));
        }
    }
    fn expect(&mut self, s: &str) -> bool {
        if self.eat(s) {
            true
        } else {
            self.error("expected-token", "Expected a required delimiter or keyword");
            false
        }
    }
    fn enter(&mut self) -> bool {
        if self.depth >= self.limits.max_depth.min(128) {
            self.error("depth-limit", "Rush nesting limit exceeded");
            self.recover();
            false
        } else {
            self.depth += 1;
            true
        }
    }
    fn recover(&mut self) {
        while self.peek().is_some() && !self.at(";") && !self.at("}") && !self.newline() {
            self.bump();
        }
    }
    fn name(&mut self, role: &'static str) -> Name<'s> {
        let span = self.span();
        if self
            .peek()
            .is_some_and(|t| matches!(t.raw.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent))
        {
            let text = self.text();
            self.bump();
            self.roles.push((span, role));
            Name { text, span }
        } else {
            self.error("expected-name", "Expected an identifier");
            Name {
                text: "",
                span: Span::new(span.start, span.start),
            }
        }
    }
    fn inline_name(&mut self, role: &'static str) -> Name<'s> {
        if self.newline() {
            self.error("expected-name", "Expected an identifier on the same line");
            let start = self.span().start;
            Name {
                text: "",
                span: Span::new(start, start),
            }
        } else {
            self.name(role)
        }
    }
    fn expect_inline(&mut self, token: &str) -> bool {
        if self.newline() {
            self.error("unexpected-newline", "The statement header cannot end here");
            false
        } else {
            self.expect(token)
        }
    }
    fn sequence(&mut self, closing: Option<&str>, indent: Option<usize>) -> Vec<Stmt<'s>> {
        let mut stmts = Vec::new();
        while self.peek().is_some() {
            if closing.is_some_and(|s| self.at(s)) {
                break;
            }
            if let Some(expected) = indent {
                let t = self.peek().unwrap();
                if t.indent < expected {
                    break;
                }
                if t.indent > expected {
                    self.error(
                        "unexpected-indent",
                        "Indentation must match the current block",
                    );
                }
                if t.tabs {
                    self.error("tab-indent", "Use spaces for Rush indentation");
                }
            }
            if self.eat(";") {
                continue;
            }
            let before = self.pos;
            let stmt = self.statement();
            let compound = matches!(
                stmt.kind,
                StmtKind::Function { .. }
                    | StmtKind::If { .. }
                    | StmtKind::While { .. }
                    | StmtKind::For { .. }
            );
            stmts.push(stmt);
            if self.pos == before {
                self.error("unexpected-token", "Expected a statement");
                self.bump();
            }
            if self.eat(";")
                || self.peek().is_none()
                || self.newline()
                || closing.is_some_and(|s| self.at(s))
            {
                continue;
            }
            if !compound {
                self.error(
                    "expected-separator",
                    "Expected a newline or semicolon after the statement",
                );
                self.recover();
            }
        }
        stmts
    }
    fn statement(&mut self) -> Stmt<'s> {
        let token = self.peek().unwrap();
        let start = token.raw.span.start;
        if !self.enter() {
            return Stmt {
                span: token.raw.span,
                kind: StmtKind::Error,
            };
        }
        let kind = match self.text() {
            "fn" => self.function(token.indent),
            "import" => {
                self.bump();
                StmtKind::Import(self.inline_name("module"))
            }
            "let" | "const" | "mut" => {
                let keyword = self.text();
                self.bump();
                let constant = if keyword == "let" {
                    !self.eat("mut")
                } else {
                    keyword != "mut"
                };
                if self.at("(") || self.at("{") {
                    if !constant {
                        self.error(
                            "immutable-pattern",
                            "Destructuring bindings must be immutable",
                        );
                    }
                    let pattern = self.expr(1, false, false);
                    self.expect_inline("=");
                    let value = self.required_expr(false);
                    StmtKind::Destructure { pattern, value }
                } else {
                    let name = self.inline_name("variable");
                    let ty = if self.eat(":") { Some(self.ty()) } else { None };
                    self.expect_inline("=");
                    let value = self.required_expr(false);
                    StmtKind::Declaration {
                        name,
                        constant,
                        ty,
                        value,
                    }
                }
            }
            "return" => {
                self.bump();
                if self.functions == 0 {
                    self.error_at(
                        token.raw.span,
                        "return-outside-function",
                        "return requires a function body",
                    );
                }
                let value =
                    if self.peek().is_none() || self.newline() || self.at(";") || self.at("}") {
                        None
                    } else {
                        Some(self.expr(0, false, true))
                    };
                StmtKind::Return(value)
            }
            "yield" => {
                self.bump();
                if self.functions == 0 && self.loops == 0 {
                    self.error_at(
                        token.raw.span,
                        "yield-outside-body",
                        "yield requires a function or loop body",
                    );
                }
                StmtKind::Yield(self.required_expr(false))
            }
            "if" => self.if_stmt(token.indent),
            "while" => {
                self.bump();
                let condition = self.required_expr(false);
                self.loops += 1;
                let body = self.block(token.indent);
                self.loops -= 1;
                StmtKind::While { condition, body }
            }
            "for" | "foreach" => {
                self.bump();
                let binding = self.inline_name("variable");
                self.expect_inline("in");
                let iterable = self.required_expr(false);
                self.loops += 1;
                let body = self.block(token.indent);
                self.loops -= 1;
                StmtKind::For {
                    binding,
                    iterable,
                    body,
                }
            }
            "break" | "continue" => {
                let is_break = self.eat("break");
                if !is_break {
                    self.bump();
                }
                if self.loops == 0 {
                    self.error_at(
                        token.raw.span,
                        "loop-control-outside-loop",
                        "break and continue require a loop body",
                    );
                }
                if is_break {
                    StmtKind::Break
                } else {
                    StmtKind::Continue
                }
            }
            "async" | "await" => {
                self.error(
                    "unsupported-syntax",
                    "This keyword is reserved but its grammar is not supported yet",
                );
                self.bump();
                self.recover();
                StmtKind::Error
            }
            _ => StmtKind::Expr(self.expr(0, false, true)),
        };
        self.depth -= 1;
        Stmt {
            span: Span::new(start, self.end().max(start)),
            kind,
        }
    }
    fn function(&mut self, indent: usize) -> StmtKind<'s> {
        self.bump();
        let name = self.inline_name("function");
        let parens = !self.newline() && self.eat("(");
        let mut parameters = Vec::new();
        let mut names = std::collections::HashSet::new();
        while self.peek().is_some()
            && !(parens && self.at(")"))
            && !self.at("->")
            && (parens || !self.at("{"))
            && !self.at(":")
            && (parens || !self.newline())
        {
            let before = self.pos;
            let pattern = if parens {
                self.match_pattern()
            } else {
                let name = self.name("parameter");
                Expr {
                    span: name.span,
                    kind: ExprKind::Name(name),
                }
            };
            self.parameter_names(&pattern, &mut names);
            let ty = if self.eat(":") { Some(self.ty()) } else { None };
            parameters.push(Parameter { pattern, ty });
            if self.pos == before {
                self.bump();
                break;
            }
            if !self.eat(",") && parens && !self.at(")") {
                self.error("expected-comma", "Separate parameters with commas");
                break;
            }
        }
        if parens {
            self.expect(")");
        }
        let result = if !self.newline() && self.eat("->") {
            Some(self.ty())
        } else {
            None
        };
        let old_loops = self.loops;
        self.loops = 0;
        self.functions += 1;
        let body = self.block(indent);
        self.functions -= 1;
        self.loops = old_loops;
        StmtKind::Function {
            name,
            parameters,
            result,
            body,
        }
    }
    fn ty(&mut self) -> Type<'s> {
        if !self.enter() {
            return Type {
                name: Name {
                    text: "",
                    span: self.span(),
                },
                arguments: vec![],
            };
        }
        let name = if self.at("null") {
            let span = self.span();
            let text = self.text();
            self.bump();
            self.roles.push((span, "type"));
            Name { text, span }
        } else {
            self.name("type")
        };
        let mut arguments = Vec::new();
        if self.eat("[") {
            while self.peek().is_some() && !self.at("]") {
                let before = self.pos;
                arguments.push(self.ty());
                if self.pos == before {
                    self.bump();
                    break;
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("]");
        }
        self.depth -= 1;
        Type { name, arguments }
    }
    fn block(&mut self, parent_indent: usize) -> Block<'s> {
        let start = self.span().start;
        if self.eat("{") {
            let stmts = self.sequence(Some("}"), None);
            self.expect("}");
            return Block {
                span: Span::new(start, self.end().max(start)),
                stmts,
            };
        }
        self.eat(":");
        if !self.newline() || self.peek().is_none_or(|t| t.indent <= parent_indent) {
            self.error(
                "expected-block",
                "Expected a brace block or an indented body on the next line",
            );
            return Block {
                span: Span::new(start, start),
                stmts: vec![],
            };
        }
        let indent = self.peek().unwrap().indent;
        let stmts = self.sequence(None, Some(indent));
        Block {
            span: Span::new(start, self.end().max(start)),
            stmts,
        }
    }
    fn if_stmt(&mut self, indent: usize) -> StmtKind<'s> {
        self.bump();
        let condition = self.required_expr(false);
        let then_block = self.block(indent);
        let else_block =
            if self.at("else") && (!self.newline() || self.peek().unwrap().indent == indent) {
                let start = self.span().start;
                self.bump();
                if self.at("if") {
                    let stmt = self.statement();
                    Some(Block {
                        span: stmt.span,
                        stmts: vec![stmt],
                    })
                } else {
                    let mut block = self.block(indent);
                    block.span.start = start;
                    Some(block)
                }
            } else {
                None
            };
        StmtKind::If {
            condition,
            then_block,
            else_block,
        }
    }
    fn required_expr(&mut self, multiline: bool) -> Expr<'s> {
        if self.peek().is_none()
            || (!multiline && self.newline())
            || matches!(self.text(), ";" | "}" | ":" | "=>")
        {
            self.error("expected-expression", "Expected an expression");
            return Expr {
                span: Span::new(self.end(), self.end()),
                kind: ExprKind::Error,
            };
        }
        self.expr(0, multiline, true)
    }
    fn expr(&mut self, min_bp: u8, multiline: bool, commands: bool) -> Expr<'s> {
        let start = self.span().start;
        let expression_indent = self.peek().map_or(0, |token| token.indent);
        if !self.enter() {
            return Expr {
                span: Span::new(start, start),
                kind: ExprKind::Error,
            };
        }
        let mut left = self.prefix(multiline);
        let mut chain = 0;
        while self.peek().is_some()
            && (multiline
                || !self.newline()
                || (self.at("|")
                    && self
                        .peek()
                        .is_some_and(|token| token.indent > expression_indent)))
        {
            if chain >= self.limits.max_depth.min(128) {
                self.error("depth-limit", "Expression chain limit exceeded");
                self.recover();
                break;
            }
            let op = self.text();
            if matches!(op, "(" | "[" | ".") && min_bp <= 20 {
                chain += 1;
                if self.eat("(") {
                    let arguments = self.arguments(")");
                    left = Expr {
                        span: Span::new(start, self.end()),
                        kind: ExprKind::Call {
                            callee: Box::new(left),
                            arguments,
                        },
                    };
                } else if self.eat("[") {
                    let index = self.required_expr(true);
                    self.expect("]");
                    left = Expr {
                        span: Span::new(start, self.end()),
                        kind: ExprKind::Index {
                            object: Box::new(left),
                            index: Box::new(index),
                        },
                    };
                } else {
                    self.bump();
                    let field = self.name("property");
                    left = Expr {
                        span: Span::new(start, self.end()),
                        kind: ExprKind::Member {
                            object: Box::new(left),
                            field,
                        },
                    };
                }
                continue;
            }
            if commands
                && !self.newline()
                && min_bp <= 2
                && matches!(left.kind, ExprKind::Name(_) | ExprKind::Member { .. })
                && self.argument_start()
            {
                let mut arguments = Vec::new();
                while self.peek().is_some() && !self.newline() && self.argument_start() {
                    let before = self.pos;
                    arguments.push(self.expr(3, false, false));
                    if self.pos == before {
                        break;
                    }
                }
                left = Expr {
                    span: Span::new(start, self.end()),
                    kind: ExprKind::Call {
                        callee: Box::new(left),
                        arguments,
                    },
                };
                chain += 1;
                continue;
            }
            let Some((lbp, rbp)) = binding(op) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let right = if self.peek().is_none()
                || (!multiline && self.newline())
                || matches!(self.text(), ";" | "}" | ")" | "]" | "," | ":" | "=>")
            {
                self.error(
                    "expected-expression",
                    "Expected an expression after the operator",
                );
                Expr {
                    span: Span::new(self.end(), self.end()),
                    kind: ExprKind::Error,
                }
            } else {
                self.expr(rbp, multiline, op == "|")
            };
            let end = right.span.end;
            let kind = if op == "|" {
                if !matches!(
                    right.kind,
                    ExprKind::Name(_) | ExprKind::Call { .. } | ExprKind::Member { .. }
                ) {
                    self.error_at(
                        right.span,
                        "invalid-pipeline-stage",
                        "A pipeline stage must name or call a function",
                    );
                }
                match left.kind {
                    ExprKind::Pipeline { input, mut stages } => {
                        stages.push(right);
                        ExprKind::Pipeline { input, stages }
                    }
                    _ => ExprKind::Pipeline {
                        input: Box::new(left),
                        stages: vec![right],
                    },
                }
            } else if lbp == 0 {
                if !matches!(
                    left.kind,
                    ExprKind::Name(_) | ExprKind::Member { .. } | ExprKind::Index { .. }
                ) {
                    self.error_at(
                        left.span,
                        "invalid-assignment-target",
                        "Assignment requires a name, member or index",
                    );
                }
                ExprKind::Assign {
                    operator: op,
                    target: Box::new(left),
                    value: Box::new(right),
                }
            } else {
                ExprKind::Binary {
                    operator: op,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            };
            left = Expr {
                span: Span::new(start, end.max(start)),
                kind,
            };
            chain += 1;
        }
        self.depth -= 1;
        left
    }
    fn argument_start(&self) -> bool {
        self.peek().is_some_and(|t| {
            matches!(
                t.raw.kind,
                SyntaxKind::Identifier
                    | SyntaxKind::TypeIdent
                    | SyntaxKind::NumberLit
                    | SyntaxKind::StringLit
            )
        }) || matches!(self.text(), "true" | "false" | "null")
    }
    fn arguments(&mut self, close: &str) -> Vec<Expr<'s>> {
        let mut args = Vec::new();
        while self.peek().is_some() && !self.at(close) {
            let before = self.pos;
            args.push(self.expr(0, true, false));
            if self.pos == before {
                self.bump();
                break;
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect(close);
        args
    }
    fn prefix(&mut self, multiline: bool) -> Expr<'s> {
        let span = self.span();
        let text = self.text();
        // The closing outer parenthesis must be followed by a lambda arrow.
        let spelling = |index: usize| {
            self.tokens
                .get(index)
                .map(|t| &self.source[t.raw.span.start..t.raw.span.end])
        };
        let lambda = if text == "(" {
            let mut cursor = self.pos + 1;
            let mut depth = 1;
            while let Some(token) = spelling(cursor) {
                match token {
                    "(" => depth += 1,
                    ")" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
            depth == 0 && spelling(cursor + 1) == Some("=>")
        } else {
            self.peek().is_some_and(|t| {
                matches!(t.raw.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent)
            }) && spelling(self.pos + 1) == Some("=>")
        };
        if lambda {
            let parens = self.eat("(");
            let mut parameters: Vec<Expr<'s>> = Vec::new();
            let mut names = std::collections::HashSet::new();
            if !parens || !self.at(")") {
                loop {
                    let pattern = self.match_pattern();
                    self.parameter_names(&pattern, &mut names);
                    parameters.push(pattern);
                    if !parens || !self.eat(",") || self.at(")") {
                        break;
                    }
                }
            }
            if parens {
                self.expect(")");
            }
            self.expect("=>");
            let body = self.required_expr(multiline);
            return Expr {
                span: Span::new(span.start, body.span.end.max(span.end)),
                kind: ExprKind::Lambda {
                    parameters,
                    body: Box::new(body),
                },
            };
        }
        if self.eat("if") {
            let condition = self.expr(0, multiline, false);
            self.expect("{");
            let then_value = self.required_expr(true);
            self.expect("}");
            self.expect("else");
            let else_value = if self.at("if") {
                self.expr(0, multiline, false)
            } else {
                self.expect("{");
                let value = self.required_expr(true);
                self.expect("}");
                value
            };
            return Expr {
                span: Span::new(span.start, self.end()),
                kind: ExprKind::If {
                    condition: Box::new(condition),
                    then_value: Box::new(then_value),
                    else_value: Box::new(else_value),
                },
            };
        }
        if text == "match" {
            return self.match_expr(multiline);
        }
        if matches!(text, "-" | "+" | "!" | "not") {
            self.bump();
            let value = if !multiline && self.newline() {
                self.required_expr(false)
            } else {
                self.expr(13, multiline, false)
            };
            return Expr {
                span: Span::new(span.start, value.span.end.max(span.end)),
                kind: ExprKind::Unary {
                    operator: text,
                    value: Box::new(value),
                },
            };
        }
        if self.eat("(") {
            if self.eat(")") {
                return Expr {
                    span: Span::new(span.start, self.end()),
                    kind: ExprKind::Tuple(vec![]),
                };
            }
            let mut value = self.expr(0, true, false);
            if self.eat(",") {
                let mut values = vec![value];
                while self.peek().is_some() && !self.at(")") {
                    let before = self.pos;
                    values.push(self.expr(0, true, false));
                    if self.pos == before || !self.eat(",") {
                        break;
                    }
                }
                self.expect(")");
                return Expr {
                    span: Span::new(span.start, self.end()),
                    kind: ExprKind::Tuple(values),
                };
            }
            self.expect(")");
            value.span = Span::new(span.start, self.end().max(span.end));
            return value;
        }
        if self.eat("[") {
            let elements = self.arguments("]");
            return Expr {
                span: Span::new(span.start, self.end().max(span.end)),
                kind: ExprKind::List(elements),
            };
        }
        if self.eat("{") {
            let mut entries = Vec::new();
            while self.peek().is_some() && !self.at("}") {
                let before = self.pos;
                let key = self.expr(0, true, false);
                self.expect(":");
                let value = self.required_expr(true);
                entries.push((key, value));
                if self.pos == before {
                    self.bump();
                    break;
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("}");
            return Expr {
                span: Span::new(span.start, self.end().max(span.end)),
                kind: ExprKind::Map(entries),
            };
        }
        let kind = match self.peek().map(|t| t.raw.kind) {
            Some(SyntaxKind::Identifier | SyntaxKind::TypeIdent) => {
                self.bump();
                ExprKind::Name(Name { text, span })
            }
            Some(SyntaxKind::NumberLit) => {
                self.bump();
                ExprKind::Number(text)
            }
            Some(SyntaxKind::StringLit) => {
                self.bump();
                ExprKind::String(text)
            }
            Some(SyntaxKind::Keyword) if text == "true" || text == "false" => {
                self.bump();
                ExprKind::Bool(text == "true")
            }
            Some(SyntaxKind::Keyword) if text == "null" => {
                self.bump();
                ExprKind::Null
            }
            Some(SyntaxKind::Keyword) if matches!(text, "async" | "await" | "import") => {
                self.error(
                    "unsupported-syntax",
                    "This keyword is reserved but its grammar is not supported yet",
                );
                self.bump();
                ExprKind::Error
            }
            _ => {
                self.error("expected-expression", "Expected a Rush expression");
                if !matches!(text, "" | ";" | "}" | ")" | "]" | "," | ":" | "=>") {
                    self.bump();
                }
                ExprKind::Error
            }
        };
        Expr { span, kind }
    }
    fn parameter_names(
        &mut self,
        pattern: &Expr<'s>,
        names: &mut std::collections::HashSet<&'s str>,
    ) {
        match &pattern.kind {
            ExprKind::Name(name) => {
                self.roles.push((name.span, "parameter"));
                if name.text != "_" && !names.insert(name.text) {
                    self.error_at(
                        name.span,
                        "duplicate-parameter",
                        "A parameter name must be unique",
                    );
                }
            }
            ExprKind::Tuple(patterns) => {
                for pattern in patterns {
                    self.parameter_names(pattern, names);
                }
            }
            ExprKind::Map(entries) => {
                for (_, pattern) in entries {
                    self.parameter_names(pattern, names);
                }
            }
            _ => self.error_at(
                pattern.span,
                "invalid-parameter-pattern",
                "Expected a name, tuple or record parameter pattern",
            ),
        }
    }

    fn match_pattern(&mut self) -> Expr<'s> {
        let span = self.span();
        if !self.enter() {
            return Expr {
                span,
                kind: ExprKind::Error,
            };
        }
        let result = if self.eat("(") {
            let mut patterns = Vec::new();
            while self.peek().is_some() && !self.at(")") {
                let before = self.pos;
                patterns.push(self.match_pattern());
                if self.pos == before || !self.eat(",") {
                    break;
                }
            }
            self.expect(")");
            Expr {
                span: Span::new(span.start, self.end()),
                kind: ExprKind::Tuple(patterns),
            }
        } else if self.eat("{") {
            let mut entries = Vec::new();
            let mut keys = std::collections::HashSet::new();
            while self.peek().is_some() && !self.at("}") {
                let before = self.pos;
                let name = self.name("field");
                if !keys.insert(name.text) {
                    self.error_at(
                        name.span,
                        "duplicate-pattern-field",
                        "Record pattern field is repeated",
                    );
                }
                self.expect(":");
                let value = self.match_pattern();
                entries.push((
                    Expr {
                        span: name.span,
                        kind: ExprKind::Name(name),
                    },
                    value,
                ));
                if self.pos == before || !self.eat(",") {
                    break;
                }
            }
            self.expect("}");
            Expr {
                span: Span::new(span.start, self.end()),
                kind: ExprKind::Map(entries),
            }
        } else if self
            .peek()
            .is_some_and(|t| matches!(t.raw.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent))
        {
            let name = self.name("binding");
            if self.eat("(") {
                let mut arguments = Vec::new();
                while self.peek().is_some() && !self.at(")") {
                    let before = self.pos;
                    arguments.push(self.match_pattern());
                    if self.pos == before || !self.eat(",") {
                        break;
                    }
                }
                self.expect(")");
                let expected = match name.text {
                    "None" => Some(0),
                    "Some" | "Ok" | "Err" => Some(1),
                    _ => None,
                };
                if expected != Some(arguments.len()) {
                    self.error_at(name.span, "invalid-pattern", "Invalid variant pattern");
                }
                Expr {
                    span: Span::new(span.start, self.end()),
                    kind: ExprKind::Call {
                        callee: Box::new(Expr {
                            span: name.span,
                            kind: ExprKind::Name(name),
                        }),
                        arguments,
                    },
                }
            } else {
                Expr {
                    span: name.span,
                    kind: ExprKind::Name(name),
                }
            }
        } else {
            let pattern = self.expr(3, true, false);
            let allowed = matches!(
                pattern.kind,
                ExprKind::Number(_) | ExprKind::String(_) | ExprKind::Bool(_) | ExprKind::Null
            ) || matches!(&pattern.kind, ExprKind::Unary { operator: "-" | "+", value } if matches!(value.kind, ExprKind::Number(_)));
            if !allowed {
                self.error_at(
                    pattern.span,
                    "invalid-pattern",
                    "Expected a literal, binding or variant pattern",
                );
            }
            pattern
        };
        self.depth -= 1;
        result
    }
    fn match_expr(&mut self, multiline: bool) -> Expr<'s> {
        let start = self.span().start;
        self.bump();
        let value = self.expr(0, multiline, false);
        if !self.expect("{") {
            return Expr {
                span: Span::new(start, self.end()),
                kind: ExprKind::Error,
            };
        }
        let mut arms = Vec::new();
        let mut wildcard = false;
        while self.peek().is_some() && !self.at("}") {
            let before = self.pos;
            let start = self.span().start;
            let pattern = self.match_pattern();
            if wildcard {
                self.error_at(
                    pattern.span,
                    "unreachable-pattern",
                    "A binding or wildcard must be the final match arm",
                );
            }
            let guard = if self.eat("if") {
                self.expect("(");
                let condition = self.expr(0, true, false);
                self.expect(")");
                Some(condition)
            } else {
                None
            };
            if guard.is_none() && matches!(pattern.kind, ExprKind::Name(_)) {
                wildcard = true;
            }
            self.expect("=>");
            let value = self.required_expr(false);
            arms.push(MatchArm {
                span: Span::new(start, value.span.end.max(start)),
                pattern,
                guard,
                value,
            });
            if self.pos == before {
                self.bump();
                break;
            }
            if self.eat(",") || self.newline() {
                continue;
            }
            if !self.at("}") {
                self.error(
                    "expected-comma",
                    "Separate match arms with commas or newlines",
                );
                self.recover();
            }
        }
        if arms.is_empty() {
            self.error("empty-match", "A match expression needs at least one arm");
        }
        self.expect("}");
        Expr {
            span: Span::new(start, self.end().max(start)),
            kind: ExprKind::Match {
                value: Box::new(value),
                arms,
            },
        }
    }
}

pub(crate) fn binding(op: &str) -> Option<(u8, u8)> {
    Some(match op {
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" => (0, 0),
        "|" => (1, 2),
        "or" | "||" => (3, 4),
        "and" | "&&" => (5, 6),
        "==" | "!=" | "<" | ">" | "<=" | ">=" => (7, 8),
        "+" | "-" => (9, 10),
        "*" | "/" | "%" => (11, 12),
        "**" => (14, 14),
        _ => return None,
    })
}
