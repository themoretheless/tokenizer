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
    fn text_at(&self, offset: usize) -> &'s str {
        self.tokens
            .get(self.pos + offset)
            .map_or("", |t| &self.source[t.raw.span.range()])
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
                StmtKind::Struct { .. }
                    | StmtKind::Enum { .. }
                    | StmtKind::Function { .. }
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
    fn user_type(&mut self) -> StmtKind<'s> {
        let is_enum = self.at("enum");
        self.bump();
        let name = self.inline_name("type");
        self.expect_inline("{");
        let mut fields = Vec::new();
        let mut variants = Vec::new();
        let mut names = std::collections::HashSet::new();
        while self.peek().is_some() && !self.at("}") {
            let before = self.pos;
            let member = self.name(if is_enum { "variant" } else { "field" });
            if !names.insert(member.text) {
                self.error_at(
                    member.span,
                    "duplicate-type-member",
                    "Type member is repeated",
                );
            }
            if is_enum {
                let mut types = Vec::new();
                if self.eat("(") {
                    while self.peek().is_some() && !self.at(")") {
                        let before = self.pos;
                        types.push(self.ty());
                        if self.pos == before || !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")");
                }
                variants.push((member, types));
            } else {
                self.expect(":");
                fields.push((member, self.ty()));
            }
            if self.pos == before {
                self.bump();
                break;
            }
            if !self.eat(",") && !self.at("}") && !self.newline() {
                self.error(
                    "expected-separator",
                    "Expected a comma or newline between type members",
                );
                break;
            }
        }
        self.expect("}");
        if is_enum {
            if variants.is_empty() {
                self.error_at(
                    name.span,
                    "empty-enum",
                    "Enum requires at least one variant",
                );
            }
            StmtKind::Enum { name, variants }
        } else {
            StmtKind::Struct { name, fields }
        }
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
            "struct" | "enum" => self.user_type(),
            "fn" => self.function(token.indent),
            "export" => {
                self.bump();
                let mut names = vec![self.inline_name("export")];
                while self.eat(",") {
                    names.push(self.inline_name("export"));
                }
                StmtKind::Export(names)
            }
            "import" => {
                self.bump();
                StmtKind::Import(self.inline_name("module"))
            }
            "let" | "const" | "mut" | "param" | "node" => {
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
                        role: match keyword {
                            "param" => DeclarationRole::Parameter,
                            "node" => DeclarationRole::Node,
                            _ => DeclarationRole::Binding,
                        },
                        name,
                        constant,
                        ty,
                        value,
                    }
                }
            }
            "show" => {
                self.bump();
                StmtKind::Show(self.required_expr(false))
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
            "strict" if self.text_at(1) == "region" => {
                self.bump();
                self.region_stmt(token.indent, true)
            }
            "region" => self.region_stmt(token.indent, false),
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
                path: vec![],
                arguments: vec![],
            };
        }
        let mut name = if self.at("null") {
            let span = self.span();
            let text = self.text();
            self.bump();
            self.roles.push((span, "type"));
            Name { text, span }
        } else {
            self.name("type")
        };
        let mut path = Vec::new();
        while self.eat(".") {
            if path.is_empty() {
                path.push(name.clone());
            }
            let part = self.name("type");
            path.push(part.clone());
            name.span.end = part.span.end;
            name.text = &self.source[name.span.start..name.span.end];
        }
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
        Type {
            name,
            path,
            arguments,
        }
    }
    fn block(&mut self, parent_indent: usize) -> Block<'s> {
        let start = self.span().start;
        if self.eat("{") {
            let stmts = self.sequence(Some("}"), None);
            self.expect("}");
            return Block {
                span: Span::new(start, self.end().max(start)),
                stmts: std::rc::Rc::new(stmts),
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
                stmts: std::rc::Rc::new(vec![]),
            };
        }
        let indent = self.peek().unwrap().indent;
        let stmts = self.sequence(None, Some(indent));
        Block {
            span: Span::new(start, self.end().max(start)),
            stmts: std::rc::Rc::new(stmts),
        }
    }
    /// Parses `region [name] [(budget)] { ... }`; the `strict` keyword was
    /// already consumed by the caller, the `region` keyword is consumed here.
    fn region_stmt(&mut self, indent: usize, strict: bool) -> StmtKind<'s> {
        self.bump();
        let name = if !self.newline()
            && self.peek().is_some_and(|t| {
                matches!(t.raw.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent)
            }) {
            Some(self.name("region"))
        } else {
            None
        };
        let budget = if self.at("(") {
            self.bump();
            let budget = self.required_expr(false);
            self.expect(")");
            Some(budget)
        } else {
            None
        };
        let body = self.block(indent);
        StmtKind::Region {
            name,
            strict,
            budget,
            body,
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
                        stmts: std::rc::Rc::new(vec![stmt]),
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
            if op == "?" && min_bp <= 20 {
                let operator_span = self.span();
                self.bump();
                chain += 1;
                if self.functions == 0 {
                    self.error_at(
                        operator_span,
                        "try-outside-function",
                        "? requires a function with an Option or Result return type",
                    );
                }
                left = Expr {
                    span: Span::new(start, self.end()),
                    kind: ExprKind::Try(Box::new(left)),
                };
                continue;
            }
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
                let span = self.peek().unwrap().raw.span;
                self.bump();
                if text.starts_with("f\"")
                    || text.starts_with("f'")
                    || text.starts_with("F\"")
                    || text.starts_with("F'")
                {
                    self.parse_interpolated_string(text, span)
                } else {
                    ExprKind::String(text)
                }
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
            let mut callee = Expr {
                span: name.span,
                kind: ExprKind::Name(name.clone()),
            };
            let mut qualified = false;
            while self.eat(".") {
                qualified = true;
                let field = self.name("variant");
                callee = Expr {
                    span: Span::new(name.span.start, field.span.end),
                    kind: ExprKind::Member {
                        object: Box::new(callee),
                        field,
                    },
                };
            }
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
                if !qualified && expected != Some(arguments.len()) {
                    self.error_at(name.span, "invalid-pattern", "Invalid variant pattern");
                }
                Expr {
                    span: Span::new(span.start, self.end()),
                    kind: ExprKind::Call {
                        callee: Box::new(callee),
                        arguments,
                    },
                }
            } else {
                if qualified {
                    self.error_at(
                        callee.span,
                        "invalid-pattern",
                        "Variant patterns require parentheses",
                    );
                }
                callee
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

    fn parse_interpolated_string(&mut self, text: &'s str, span: Span) -> ExprKind<'s> {
        let is_format = text.starts_with("f\"")
            || text.starts_with("f'")
            || text.starts_with("F\"")
            || text.starts_with("F'");
        if !is_format {
            return ExprKind::String(text);
        }
        let quote = text.as_bytes()[1] as char;
        if text.len() < 3 || !text.ends_with(quote) {
            self.error("unclosed-string", "Unclosed format string");
            return ExprKind::Error;
        }

        let content = &text[2..text.len() - 1];
        let content_start_offset = span.start + 2;

        let mut parts = Vec::new();
        let mut current_literal = String::new();
        let mut byte_idx = 0;

        while byte_idx < content.len() {
            let rest = &content[byte_idx..];
            let c = rest.chars().next().unwrap();

            if c == '\\' {
                byte_idx += 1;
                if byte_idx >= content.len() {
                    break;
                }
                let next_ch = content[byte_idx..].chars().next().unwrap();
                byte_idx += next_ch.len_utf8();
                match next_ch {
                    'n' => current_literal.push('\n'),
                    'r' => current_literal.push('\r'),
                    't' => current_literal.push('\t'),
                    '\\' => current_literal.push('\\'),
                    '"' => current_literal.push('"'),
                    '\'' => current_literal.push('\''),
                    '{' => current_literal.push('{'),
                    '}' => current_literal.push('}'),
                    'u' if content[byte_idx..].starts_with('{') => {
                        byte_idx += 1;
                        let mut code = 0u32;
                        let mut digits = 0;
                        while byte_idx < content.len() {
                            let ch = content[byte_idx..].chars().next().unwrap();
                            byte_idx += ch.len_utf8();
                            if ch == '}' {
                                break;
                            }
                            if let Some(digit) = ch.to_digit(16) {
                                code = code * 16 + digit;
                                digits += 1;
                            } else {
                                self.error("invalid-escape", "Invalid hex in Unicode escape");
                                break;
                            }
                        }
                        if digits > 0
                            && let Some(u_char) = char::from_u32(code)
                        {
                            current_literal.push(u_char);
                        }
                    }
                    other => {
                        current_literal.push('\\');
                        current_literal.push(other);
                    }
                }
            } else if c == '{' {
                if rest[1..].starts_with('{') {
                    // Escaped {{
                    current_literal.push('{');
                    byte_idx += 2;
                } else {
                    // Expression start
                    if !current_literal.is_empty() {
                        parts.push(InterpolationPart::Literal(std::mem::take(
                            &mut current_literal,
                        )));
                    }
                    byte_idx += 1;
                    let expr_start_idx = byte_idx;
                    let mut brace_depth = 1;
                    let mut in_str = false;
                    let mut str_quote = ' ';
                    let mut expr_end_idx = None;

                    while byte_idx < content.len() {
                        let ch = content[byte_idx..].chars().next().unwrap();
                        let ch_len = ch.len_utf8();
                        if in_str {
                            if ch == '\\' && byte_idx + 1 < content.len() {
                                byte_idx +=
                                    1 + content[byte_idx + 1..].chars().next().unwrap().len_utf8();
                                continue;
                            } else if ch == str_quote {
                                in_str = false;
                            }
                        } else {
                            match ch {
                                '"' | '\'' => {
                                    in_str = true;
                                    str_quote = ch;
                                }
                                '{' => brace_depth += 1,
                                '}' => {
                                    brace_depth -= 1;
                                    if brace_depth == 0 {
                                        expr_end_idx = Some(byte_idx);
                                        byte_idx += 1;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        byte_idx += ch_len;
                    }

                    let Some(expr_end) = expr_end_idx else {
                        self.error(
                            "unclosed-interpolation",
                            "Expected } to close interpolated expression",
                        );
                        return ExprKind::Error;
                    };

                    let expr_source = &content[expr_start_idx..expr_end];
                    let expr_abs_start = content_start_offset + expr_start_idx;
                    if expr_source.trim().is_empty() {
                        self.error(
                            "empty-interpolation",
                            "Interpolated expression cannot be empty",
                        );
                    } else {
                        let parsed_expr = crate::parse_with(expr_source, self.limits);
                        for d in &parsed_expr.diagnostics {
                            self.diagnostics.push(Diagnostic::new(
                                Span::new(
                                    expr_abs_start + d.span.start,
                                    expr_abs_start + d.span.end,
                                ),
                                d.code,
                                d.message,
                            ));
                        }
                        if let Some(stmt) = parsed_expr.module.items.into_iter().next() {
                            if let StmtKind::Expr(mut e) = stmt.kind {
                                offset_expr_spans(&mut e, expr_abs_start);
                                parts.push(InterpolationPart::Expr(e));
                            } else {
                                self.error(
                                    "invalid-interpolation",
                                    "Expected expression in interpolation",
                                );
                            }
                        }
                    }
                }
            } else if c == '}' {
                if rest[1..].starts_with('}') {
                    current_literal.push('}');
                    byte_idx += 2;
                } else {
                    self.error(
                        "stray-brace",
                        "Single '}' is not allowed in format string; use '}}' to escape",
                    );
                    current_literal.push('}');
                    byte_idx += 1;
                }
            } else {
                current_literal.push(c);
                byte_idx += c.len_utf8();
            }
        }

        if !current_literal.is_empty() {
            parts.push(InterpolationPart::Literal(current_literal));
        }

        ExprKind::Interpolate(parts)
    }
}

fn offset_expr_spans(expr: &mut Expr<'_>, offset: usize) {
    expr.span.start += offset;
    expr.span.end += offset;
    match &mut expr.kind {
        ExprKind::Try(sub) => offset_expr_spans(sub, offset),
        ExprKind::If {
            condition,
            then_value,
            else_value,
        } => {
            offset_expr_spans(condition, offset);
            offset_expr_spans(then_value, offset);
            offset_expr_spans(else_value, offset);
        }
        ExprKind::Lambda { parameters, body } => {
            for param in parameters {
                offset_expr_spans(param, offset);
            }
            offset_expr_spans(body, offset);
        }
        ExprKind::Name(name) => {
            name.span.start += offset;
            name.span.end += offset;
        }
        ExprKind::Unary { value, .. } => offset_expr_spans(value, offset),
        ExprKind::Binary { left, right, .. } => {
            offset_expr_spans(left, offset);
            offset_expr_spans(right, offset);
        }
        ExprKind::Assign { target, value, .. } => {
            offset_expr_spans(target, offset);
            offset_expr_spans(value, offset);
        }
        ExprKind::Call { callee, arguments } => {
            offset_expr_spans(callee, offset);
            for arg in arguments {
                offset_expr_spans(arg, offset);
            }
        }
        ExprKind::Member { object, field } => {
            offset_expr_spans(object, offset);
            field.span.start += offset;
            field.span.end += offset;
        }
        ExprKind::Index { object, index } => {
            offset_expr_spans(object, offset);
            offset_expr_spans(index, offset);
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                offset_expr_spans(item, offset);
            }
        }
        ExprKind::Map(entries) => {
            for (k, v) in entries {
                offset_expr_spans(k, offset);
                offset_expr_spans(v, offset);
            }
        }
        ExprKind::Pipeline { input, stages } => {
            offset_expr_spans(input, offset);
            for stage in stages {
                offset_expr_spans(stage, offset);
            }
        }
        ExprKind::Match { value, arms } => {
            offset_expr_spans(value, offset);
            for arm in arms {
                arm.span.start += offset;
                arm.span.end += offset;
                offset_expr_spans(&mut arm.pattern, offset);
                if let Some(guard) = &mut arm.guard {
                    offset_expr_spans(guard, offset);
                }
                offset_expr_spans(&mut arm.value, offset);
            }
        }
        ExprKind::Interpolate(parts) => {
            for part in parts {
                if let InterpolationPart::Expr(sub) = part {
                    offset_expr_spans(sub, offset);
                }
            }
        }
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Null
        | ExprKind::Error => {}
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
