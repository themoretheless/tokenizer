//! Lexical binding checks. Unknown names remain available to the host.
use crate::{Block, Expr, ExprKind, HostFunction, Name, Parse, Stmt, StmtKind, ValueType};
use std::collections::HashMap;
use std::rc::Rc;
use themoretheless_tokenizer_core::{Diagnostic, Span};

/// Source locations for a lexically resolved reference or an external name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameReference {
    pub usage: Span,
    pub definition: Option<Span>,
}

/// A binding and the half-open byte-offset interval where it is visible.
/// The module interval includes the end-of-file cursor (end = source length + 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexicalBinding {
    pub name: String,
    pub definition: Span,
    pub visible: Span,
    pub depth: usize,
    /// Statically known fields, without invoking user or host functions.
    pub members: Vec<String>,
    /// Fields available after a dotted path relative to this binding.
    pub member_paths: std::collections::BTreeMap<String, Vec<String>>,
}

/// Known members of the receiver at a member-name byte span (empty at a trailing dot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberCompletion {
    pub name_span: Span,
    pub members: Vec<String>,
}

pub(crate) fn editor(
    parsed: &mut Parse<'_>,
    limit: usize,
) -> (
    Vec<NameReference>,
    Vec<LexicalBinding>,
    Vec<MemberCompletion>,
) {
    editor_with_hosts(parsed, limit, false, HashMap::new())
}

pub(crate) fn editor_with_hosts(
    parsed: &mut Parse<'_>,
    limit: usize,
    calls: bool,
    hosts: HashMap<String, Rc<HostFunction>>,
) -> (
    Vec<NameReference>,
    Vec<LexicalBinding>,
    Vec<MemberCompletion>,
) {
    let trailing_member = parsed.source.trim_end().ends_with('.')
        && parsed.diagnostics.len() == 1
        && parsed.diagnostics[0].code == "expected-name";
    inspect_inner(parsed, limit, calls, hosts, trailing_member, true)
}

pub(crate) fn check(parsed: &mut Parse<'_>, limit: usize) -> Vec<NameReference> {
    check_calls(parsed, limit, false)
}
pub(crate) fn check_calls(parsed: &mut Parse<'_>, limit: usize, calls: bool) -> Vec<NameReference> {
    check_host_calls(parsed, limit, calls, HashMap::new())
}
pub(crate) fn check_host_calls(
    parsed: &mut Parse<'_>,
    limit: usize,
    calls: bool,
    host_functions: HashMap<String, Rc<HostFunction>>,
) -> Vec<NameReference> {
    inspect(parsed, limit, calls, host_functions).0
}
pub(crate) fn inspect(
    parsed: &mut Parse<'_>,
    limit: usize,
    calls: bool,
    host_functions: HashMap<String, Rc<HostFunction>>,
) -> (Vec<NameReference>, Vec<LexicalBinding>) {
    let (references, bindings, _) =
        inspect_inner(parsed, limit, calls, host_functions, false, false);
    (references, bindings)
}
fn inspect_inner(
    parsed: &mut Parse<'_>,
    limit: usize,
    calls: bool,
    host_functions: HashMap<String, Rc<HostFunction>>,
    recover_trailing_member: bool,
    collect_members: bool,
) -> (
    Vec<NameReference>,
    Vec<LexicalBinding>,
    Vec<MemberCompletion>,
) {
    // Only the editor may inspect a missing final member name. Other syntax errors
    // still suppress metadata, and execution continues to reject this source.
    if !parsed.is_valid() && !recover_trailing_member {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut checker = Checker {
        scopes: vec![HashMap::new()],
        scope_spans: vec![Span::new(0, parsed.source.len() + 1)],
        bindings: Vec::new(),
        binding_types: HashMap::new(),
        binding_shapes: HashMap::new(),
        record_fields: HashMap::new(),
        return_types: Vec::new(),
        function_results: HashMap::new(),
        function_parameters: HashMap::new(),
        builtin_aliases: HashMap::new(),
        references: Vec::new(),
        member_completions: Vec::new(),
        collect_members,
        calls,
        host_functions,
        diagnostics: Vec::new(),
        valid: true,
        limit,
    };
    checker.statements(&parsed.module.items);
    for binding in &mut checker.bindings {
        if let Some(shape) = checker.binding_shapes.get(&binding.definition.start) {
            shape.member_paths("", &mut binding.member_paths);
        }
        if let Some(fields) = checker.record_fields.get(&binding.definition.start) {
            binding.members = fields.clone();
        }
        if let Some(ValueType::Mesh) = checker.binding_types.get(&binding.definition.start) {
            binding.members = vec!["triangles".into(), "vertices".into()];
        }
        if let Some(ValueType::Vector(size)) = checker.binding_types.get(&binding.definition.start)
        {
            binding.members = ["x", "y", "z", "w"]
                .iter()
                .take(*size)
                .map(|name| (*name).to_owned())
                .collect();
        }
    }
    parsed.valid &= checker.valid;
    parsed.diagnostics.extend(checker.diagnostics);
    (
        checker.references,
        checker.bindings,
        checker.member_completions,
    )
}
type PatternSignature<'s> = (
    &'s str,
    Option<Vec<std::ops::RangeInclusive<usize>>>,
    Option<Rc<HostFunction>>,
    Option<ValueType>,
    Option<Vec<Option<ValueType>>>,
    Shape,
    Option<crate::Builtin>,
    Shape,
);

type Binding = (
    bool,
    Span,
    Option<Vec<std::ops::RangeInclusive<usize>>>,
    Option<Rc<HostFunction>>,
);

struct Checker<'s> {
    member_completions: Vec<MemberCompletion>,
    collect_members: bool,
    host_functions: HashMap<String, Rc<HostFunction>>,
    calls: bool,
    scopes: Vec<HashMap<&'s str, Binding>>,
    scope_spans: Vec<Span>,
    bindings: Vec<LexicalBinding>,
    binding_types: HashMap<usize, ValueType>,
    binding_shapes: HashMap<usize, Shape>,
    record_fields: HashMap<usize, Vec<String>>,
    return_types: Vec<Option<ValueType>>,
    function_results: HashMap<usize, ValueType>,
    function_parameters: HashMap<usize, Vec<Option<ValueType>>>,
    builtin_aliases: HashMap<usize, crate::Builtin>,
    references: Vec<NameReference>,
    diagnostics: Vec<Diagnostic>,
    valid: bool,
    limit: usize,
}
// Only exits that can reach a following statement matter for missing-return.
// Return and continue stop the current block; loops consume their own breaks.
#[derive(Clone, Copy, Default)]
struct BlockExits {
    next: bool,
    breaks: bool,
}
impl BlockExits {
    fn inspect(statements: &[Stmt<'_>]) -> Self {
        let mut exits = Self {
            next: true,
            breaks: false,
        };
        for statement in statements {
            if !exits.next {
                break;
            }
            let next = match &statement.kind {
                StmtKind::Return(_) | StmtKind::Continue => Self::default(),
                StmtKind::Break => Self {
                    next: false,
                    breaks: true,
                },
                StmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    let left = Self::inspect(&then_block.stmts);
                    let right = else_block.as_ref().map_or(
                        Self {
                            next: true,
                            breaks: false,
                        },
                        |block| Self::inspect(&block.stmts),
                    );
                    match condition.kind {
                        ExprKind::Bool(true) => left,
                        ExprKind::Bool(false) => right,
                        _ => Self {
                            next: left.next || right.next,
                            breaks: left.breaks || right.breaks,
                        },
                    }
                }
                StmtKind::While { condition, body } => Self {
                    next: !matches!(condition.kind, ExprKind::Bool(true))
                        || Self::inspect(&body.stmts).breaks,
                    breaks: false,
                },
                // An iterable can be empty. A break exits only this loop.
                StmtKind::For { .. } => Self {
                    next: true,
                    breaks: false,
                },
                _ => Self {
                    next: true,
                    breaks: false,
                },
            };
            exits.next = next.next;
            exits.breaks |= next.breaks;
        }
        exits
    }
}
impl<'s> Checker<'s> {
    fn literal_number(expression: &Expr<'_>) -> Option<f64> {
        match &expression.kind {
            ExprKind::Number(text) => text.replace('_', "").parse().ok(),
            ExprKind::Unary {
                operator: "+",
                value,
            } => Self::literal_number(value),
            ExprKind::Unary {
                operator: "-",
                value,
            } => Self::literal_number(value).map(|n| -n),
            _ => None,
        }
    }
    fn literal_string(expression: &Expr<'_>) -> Option<String> {
        let ExprKind::String(text) = &expression.kind else {
            return None;
        };
        crate::string_literal::characters(text)
            .collect::<Result<String, _>>()
            .ok()
    }
    fn check_index(&mut self, object: &Expr<'s>, index: &Expr<'s>) {
        if let Some(fields) = self.known_record_fields(object) {
            if self.shape(index).rejects(&ValueType::String) {
                self.error(index.span, "index-type", "Record index must be a string");
            } else if let Some(key) = Self::literal_string(index)
                && !fields.contains(&key)
            {
                self.error(
                    index.span,
                    "unknown-record-field",
                    "Unknown field in record",
                );
            }
            return;
        }
        let length = match self.shape(object) {
            Shape::Typed(ValueType::Vector(size)) => Some(size),
            Shape::Tuple(items) | Shape::List(items) => Some(items.len()),
            Shape::Typed(ValueType::Tuple(items)) => Some(items.len()),
            Shape::Typed(ValueType::List(_)) => None,
            Shape::Typed(_) => {
                self.error(
                    object.span,
                    "index-object",
                    "Value does not support indexing",
                );
                return;
            }
            _ => return,
        };
        if self.shape(index).rejects(&ValueType::Number) {
            self.error(
                index.span,
                "index-type",
                "Collection index must be an integer",
            );
        } else if let Some(number) = Self::literal_number(index)
            && (!number.is_finite()
                || number < 0.0
                || number.fract() != 0.0
                || number >= usize::MAX as f64
                || length.is_some_and(|size| number >= size as f64))
        {
            self.error(
                index.span,
                "index-bounds",
                "Invalid or out-of-bounds collection index",
            );
        }
    }
    fn error(&mut self, span: Span, code: &'static str, message: &'static str) {
        self.valid = false;
        if self.diagnostics.len() < self.limit {
            self.diagnostics.push(Diagnostic::new(span, code, message));
        }
    }
    fn declare(&mut self, name: &Name<'s>, constant: bool, start: usize) {
        let scope = self.scopes.last_mut().unwrap();
        if scope.contains_key(name.text) {
            self.error(
                name.span,
                "duplicate-binding",
                "Name is already declared in this scope",
            );
        } else {
            scope.insert(name.text, (constant, name.span, None, None));
            self.bindings.push(LexicalBinding {
                name: name.text.to_owned(),
                definition: name.span,
                visible: Span::new(start, self.scope_spans.last().unwrap().end),
                depth: self.scopes.len() - 1,
                members: Vec::new(),
                member_paths: std::collections::BTreeMap::new(),
            });
        }
    }
    fn statements(&mut self, statements: &[Stmt<'s>]) {
        for statement in statements {
            self.statement(statement);
        }
    }
    fn block(&mut self, block: &Block<'s>) {
        self.scopes.push(HashMap::new());
        self.scope_spans.push(block.span);
        self.statements(&block.stmts);
        self.scopes.pop();
        self.scope_spans.pop();
    }
    fn statement(&mut self, statement: &Stmt<'s>) {
        match &statement.kind {
            StmtKind::Import(name) => self.declare(name, true, statement.span.end),
            StmtKind::Destructure { pattern, value } => {
                self.expr(value);
                if self.calls {
                    self.check_pattern_shape(pattern, &self.shape(value));
                }
                let mut signatures = Vec::new();
                self.pattern_signatures(pattern, value, &mut signatures);
                self.pattern(pattern, statement.span.end);
                for (name, arity, host, ty, parameters, result, builtin, shape) in signatures {
                    if let Some(binding) = self.scopes.last_mut().unwrap().get_mut(name) {
                        binding.2 = arity;
                        binding.3 = host;
                        if let Shape::Record(fields) = &shape {
                            self.record_fields
                                .insert(binding.1.start, fields.keys().cloned().collect());
                        }
                        if matches!(shape, Shape::Record(_) | Shape::Tuple(_) | Shape::List(_)) {
                            self.binding_shapes.insert(binding.1.start, shape);
                        }
                        if let Some(builtin) = builtin {
                            self.builtin_aliases.insert(binding.1.start, builtin);
                        }
                        if let Some(parameters) = parameters {
                            self.function_parameters.insert(binding.1.start, parameters);
                        }
                        if let Shape::Typed(result) = result {
                            self.function_results.insert(binding.1.start, result);
                        }
                        if let Some(ty) = ty {
                            self.binding_types.insert(binding.1.start, ty);
                        }
                    }
                }
            }
            StmtKind::Function {
                name,
                parameters,
                body,
                result,
            } => {
                let result_type = result.as_ref().and_then(|ty| self.checked_annotation(ty));
                if self.calls
                    && let Some(expected) = &result_type
                    && Shape::Typed(ValueType::Null).rejects(expected)
                    && BlockExits::inspect(&body.stmts).next
                {
                    self.error(
                        name.span,
                        "missing-return",
                        "Function can finish without returning its annotated result",
                    );
                }
                if let Some(ty) = &result_type {
                    self.function_results.insert(name.span.start, ty.clone());
                }
                self.return_types.push(result_type);
                let parameter_types = parameters
                    .iter()
                    .map(|parameter| {
                        parameter
                            .ty
                            .as_ref()
                            .and_then(|ty| ValueType::annotation(ty).ok())
                    })
                    .collect();
                self.function_parameters
                    .insert(name.span.start, parameter_types);
                self.declare(name, true, statement.span.start);
                self.scopes
                    .last_mut()
                    .unwrap()
                    .get_mut(name.text)
                    .unwrap()
                    .2 = Some(std::iter::once(parameters.len()..=parameters.len()).collect());
                self.scopes.push(HashMap::new());
                self.scope_spans.push(body.span);
                for parameter in parameters {
                    self.pattern(&parameter.pattern, body.span.start);
                    if let Some(annotation) = &parameter.ty
                        && let Some(ty) = self.checked_annotation(annotation)
                    {
                        if self.calls {
                            self.check_pattern_shape(&parameter.pattern, &Shape::Typed(ty.clone()));
                        }
                        self.pattern_type(&parameter.pattern, &ty);
                    }
                }
                self.statements(&body.stmts);
                self.return_types.pop();
                self.scopes.pop();
                self.scope_spans.pop();
            }
            StmtKind::Declaration {
                name,
                constant,
                value,
                ty,
            } => {
                self.expr(value);
                let annotation = ty.as_ref().and_then(|ty| self.checked_annotation(ty));
                if self.calls
                    && let Some(expected) = &annotation
                    && self.shape(value).rejects(expected)
                {
                    self.error(
                        value.span,
                        "annotation-type",
                        "Initializer type does not match annotation",
                    );
                }
                let arity = if *constant {
                    self.known_arity(value)
                } else {
                    None
                };
                let host = if *constant {
                    self.known_host(value)
                } else {
                    None
                };
                // Inspect the initializer before introducing its binding (shadowing).
                let known_type = if *constant {
                    annotation.or_else(|| match self.shape(value) {
                        Shape::Typed(ty) => Some(ty),
                        _ => None,
                    })
                } else {
                    None
                };
                let callable_result = if *constant {
                    self.call_result_shape(value)
                } else {
                    Shape::Unknown
                };
                let callable_parameters = if *constant {
                    self.known_user_parameters(value)
                } else {
                    None
                };
                let fields = if *constant {
                    self.known_record_fields(value)
                } else {
                    None
                };
                let builtin = if *constant {
                    self.known_builtin(value)
                } else {
                    None
                };
                let binding_shape = if *constant {
                    self.shape(value)
                } else {
                    Shape::Unknown
                };
                self.declare(name, *constant, statement.span.end);
                if matches!(
                    binding_shape,
                    Shape::Record(_) | Shape::Tuple(_) | Shape::List(_)
                ) {
                    self.binding_shapes.insert(name.span.start, binding_shape);
                }
                if let Some(builtin) = builtin {
                    self.builtin_aliases.insert(name.span.start, builtin);
                }
                if let Some(fields) = fields {
                    self.record_fields.insert(name.span.start, fields);
                }
                if let Some(parameters) = callable_parameters {
                    self.function_parameters.insert(name.span.start, parameters);
                }
                if let Shape::Typed(ty) = callable_result {
                    self.function_results.insert(name.span.start, ty);
                }
                if let Some(ty) = known_type {
                    self.binding_types.insert(name.span.start, ty);
                }
                self.scopes
                    .last_mut()
                    .unwrap()
                    .get_mut(name.text)
                    .unwrap()
                    .3 = host;
                self.scopes
                    .last_mut()
                    .unwrap()
                    .get_mut(name.text)
                    .unwrap()
                    .2 = arity;
            }
            StmtKind::Return(value) => {
                if self.calls
                    && let Some(Some(expected)) = self.return_types.last()
                {
                    let shape = value
                        .as_ref()
                        .map_or(Shape::Typed(ValueType::Null), |value| self.shape(value));
                    if shape.rejects(expected) {
                        self.error(
                            value.as_ref().map_or(statement.span, |value| value.span),
                            "return-type",
                            "Return value does not match result annotation",
                        );
                    }
                }
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            StmtKind::Yield(value) | StmtKind::Expr(value) => self.expr(value),
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.condition(condition);
                self.block(then_block);
                if let Some(block) = else_block {
                    self.block(block);
                }
            }
            StmtKind::While { condition, body } => {
                self.condition(condition);
                self.block(body);
            }
            StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                self.expr(iterable);
                let element = match self.shape(iterable) {
                    Shape::Typed(ValueType::List(element)) => Shape::Typed(*element),
                    Shape::List(items) => items
                        .first()
                        .filter(|first| items.iter().all(|item| item == *first))
                        .cloned()
                        .unwrap_or(Shape::Unknown),
                    _ => Shape::Unknown,
                };
                self.scopes.push(HashMap::new());
                self.scope_spans.push(body.span);
                self.declare(binding, true, body.span.start);
                if let Shape::Typed(ty) = &element {
                    self.binding_types.insert(binding.span.start, ty.clone());
                }
                if let Shape::Record(fields) = &element {
                    self.record_fields
                        .insert(binding.span.start, fields.keys().cloned().collect());
                }
                if matches!(element, Shape::Record(_) | Shape::Tuple(_) | Shape::List(_)) {
                    self.binding_shapes.insert(binding.span.start, element);
                }
                self.statements(&body.stmts);
                self.scopes.pop();
                self.scope_spans.pop();
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
        }
    }
    fn match_bindings(&mut self, pattern: &Expr<'s>, start: usize) {
        match &pattern.kind {
            ExprKind::Name(name) if name.text != "_" => self.declare(name, true, start),
            ExprKind::Call { arguments, .. } | ExprKind::Tuple(arguments) => {
                for argument in arguments {
                    self.match_bindings(argument, start);
                }
            }
            ExprKind::Map(entries) => {
                for (_, pattern) in entries {
                    self.match_bindings(pattern, start);
                }
            }
            _ => {}
        }
    }
    fn pattern(&mut self, pattern: &Expr<'s>, start: usize) {
        match &pattern.kind {
            ExprKind::Name(name) if name.text == "_" => {}
            ExprKind::Name(name) => self.declare(name, true, start),
            ExprKind::Tuple(values) => {
                for value in values {
                    self.pattern(value, start);
                }
            }
            ExprKind::Map(entries) => {
                let mut fields = std::collections::HashSet::new();
                for (key, value) in entries {
                    if let ExprKind::Name(name) = &key.kind {
                        if !fields.insert(name.text) {
                            self.error(
                                name.span,
                                "duplicate-pattern-field",
                                "Record pattern field is repeated",
                            );
                        }
                    } else {
                        self.error(
                            key.span,
                            "invalid-binding-pattern",
                            "Record pattern keys must be names",
                        );
                    }
                    self.pattern(value, start);
                }
            }
            _ => self.error(
                pattern.span,
                "invalid-binding-pattern",
                "Expected a name or tuple binding pattern",
            ),
        }
    }
    fn checked_annotation(&mut self, ty: &crate::Type<'s>) -> Option<ValueType> {
        match ValueType::annotation(ty) {
            Ok(ty) => Some(ty),
            Err(error) => {
                if self.calls {
                    self.error(
                        error.span,
                        "unsupported-annotation",
                        "Unknown type or invalid type arguments",
                    );
                }
                None
            }
        }
    }
    fn check_pattern_shape(&mut self, pattern: &Expr<'s>, shape: &Shape) {
        if let ExprKind::Map(patterns) = &pattern.kind {
            match shape {
                Shape::Record(fields) => {
                    for (key, pattern) in patterns {
                        if let ExprKind::Name(name) = &key.kind {
                            if let Some(shape) = fields.get(name.text) {
                                self.check_pattern_shape(pattern, shape);
                            } else {
                                self.error(
                                    name.span,
                                    "unknown-record-field",
                                    "Record pattern requires a missing field",
                                );
                            }
                        }
                    }
                }
                Shape::Typed(_) | Shape::List(_) | Shape::Tuple(_) => {
                    self.error(
                        pattern.span,
                        "record-pattern",
                        "Record pattern requires a record value",
                    );
                }
                Shape::Unknown | Shape::Unsupported => {}
            }
            return;
        }
        let ExprKind::Tuple(patterns) = &pattern.kind else {
            return;
        };
        let items = match shape {
            Shape::Tuple(items) => items.clone(),
            Shape::Typed(ValueType::Tuple(types)) => {
                types.iter().cloned().map(Shape::Typed).collect()
            }
            Shape::Unknown => return,
            _ => {
                self.error(
                    pattern.span,
                    "tuple-pattern",
                    "Tuple pattern requires a tuple value",
                );
                return;
            }
        };
        if patterns.len() != items.len() {
            self.error(
                pattern.span,
                "tuple-pattern",
                "Tuple pattern length does not match value",
            );
            return;
        }
        for (pattern, shape) in patterns.iter().zip(&items) {
            self.check_pattern_shape(pattern, shape);
        }
    }
    fn pattern_type(&mut self, pattern: &Expr<'s>, ty: &ValueType) {
        match (&pattern.kind, ty) {
            (ExprKind::Name(name), _) if name.text != "_" => {
                self.binding_types.insert(name.span.start, ty.clone());
            }
            (ExprKind::Tuple(patterns), ValueType::Tuple(types))
                if patterns.len() == types.len() =>
            {
                for (pattern, ty) in patterns.iter().zip(types) {
                    self.pattern_type(pattern, ty);
                }
            }
            _ => {}
        }
    }
    // Resolve the entire initializer before declaring pattern names: a new name
    // may shadow a function used by another element of that same initializer.
    fn pattern_signatures(
        &self,
        pattern: &Expr<'s>,
        value: &Expr<'s>,
        signatures: &mut Vec<PatternSignature<'s>>,
    ) {
        match (&pattern.kind, &value.kind) {
            (ExprKind::Name(name), _) if name.text != "_" => {
                let ty = match self.shape(value) {
                    Shape::Typed(ty) => Some(ty),
                    _ => None,
                };
                signatures.push((
                    name.text,
                    self.known_arity(value),
                    self.known_host(value),
                    ty,
                    self.known_user_parameters(value),
                    self.call_result_shape(value),
                    self.known_builtin(value),
                    self.shape(value),
                ));
            }
            (ExprKind::Tuple(patterns), ExprKind::Tuple(values))
                if patterns.len() == values.len() =>
            {
                for (pattern, value) in patterns.iter().zip(values) {
                    self.pattern_signatures(pattern, value, signatures);
                }
            }
            (ExprKind::Map(patterns), ExprKind::Map(values))
                if values
                    .iter()
                    .all(|(key, _)| matches!(key.kind, ExprKind::Name(_))) =>
            {
                for (key, pattern) in patterns {
                    if let ExprKind::Name(name) = &key.kind
                        && let Some((_, value)) = values.iter().rev().find(|(key, _)| {
                            matches!(&key.kind, ExprKind::Name(key) if key.text == name.text)
                        })
                    {
                        self.pattern_signatures(pattern, value, signatures);
                    }
                }
            }
            _ => self.pattern_shape_signatures(pattern, &self.shape(value), signatures),
        }
    }
    fn pattern_shape_signatures(
        &self,
        pattern: &Expr<'s>,
        shape: &Shape,
        signatures: &mut Vec<PatternSignature<'s>>,
    ) {
        match (&pattern.kind, shape) {
            (ExprKind::Name(name), _) if name.text != "_" => {
                let ty = if let Shape::Typed(ty) = shape {
                    Some(ty.clone())
                } else {
                    None
                };
                signatures.push((
                    name.text,
                    None,
                    None,
                    ty,
                    None,
                    Shape::Unknown,
                    None,
                    shape.clone(),
                ));
            }
            (ExprKind::Map(patterns), Shape::Record(fields)) => {
                for (key, pattern) in patterns {
                    if let ExprKind::Name(name) = &key.kind
                        && let Some(field) = fields.get(name.text)
                    {
                        self.pattern_shape_signatures(pattern, field, signatures);
                    }
                }
            }
            (ExprKind::Tuple(patterns), Shape::Tuple(values)) if patterns.len() == values.len() => {
                for (pattern, value) in patterns.iter().zip(values) {
                    self.pattern_shape_signatures(pattern, value, signatures);
                }
            }
            (ExprKind::Tuple(patterns), Shape::Typed(ValueType::Tuple(types)))
                if patterns.len() == types.len() =>
            {
                for (pattern, ty) in patterns.iter().zip(types) {
                    self.pattern_shape_signatures(pattern, &Shape::Typed(ty.clone()), signatures);
                }
            }
            _ => {}
        }
    }
    fn known_builtin(&self, expression: &Expr<'s>) -> Option<crate::Builtin> {
        match &expression.kind {
            ExprKind::Name(name) => {
                if let Some(binding) = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                {
                    return self.builtin_aliases.get(&binding.1.start).copied();
                }
                crate::builtin_catalog()
                    .iter()
                    .find(|(entry, _)| *entry == name.text)
                    .map(|(_, builtin)| *builtin)
            }
            ExprKind::If {
                then_value,
                else_value,
                ..
            } => {
                let left = self.known_builtin(then_value)?;
                (Some(left) == self.known_builtin(else_value)).then_some(left)
            }
            _ => None,
        }
    }
    fn known_arity(&self, expression: &Expr<'s>) -> Option<Vec<std::ops::RangeInclusive<usize>>> {
        match &expression.kind {
            ExprKind::Lambda { parameters, .. } => {
                Some(std::iter::once(parameters.len()..=parameters.len()).collect())
            }
            ExprKind::Name(name) => {
                if let Some(binding) = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                {
                    return binding.2.clone();
                }
                crate::builtin_catalog()
                    .iter()
                    .find(|(builtin, _)| *builtin == name.text)
                    .map(|(_, builtin)| vec![builtin.arity()])
                    .or_else(|| {
                        self.host_functions.get(name.text).map(|function| {
                            std::iter::once(function.parameters.len()..=function.parameters.len())
                                .collect()
                        })
                    })
            }
            ExprKind::If {
                then_value,
                else_value,
                ..
            } => {
                let mut alternatives = self.known_arity(then_value)?;
                for arity in self.known_arity(else_value)? {
                    if !alternatives.contains(&arity) {
                        alternatives.push(arity);
                    }
                }
                Some(alternatives)
            }
            _ => None,
        }
    }
    fn known_host(&self, expression: &Expr<'s>) -> Option<Rc<HostFunction>> {
        if self.host_functions.is_empty() {
            return None;
        }
        let ExprKind::Name(name) = &expression.kind else {
            return None;
        };
        if let Some(binding) = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name.text))
        {
            return binding.3.clone();
        }
        if crate::builtin_catalog()
            .iter()
            .any(|(builtin, _)| *builtin == name.text)
        {
            return None;
        }
        self.host_functions.get(name.text).cloned()
    }
    fn known_record_fields(&self, expression: &Expr<'s>) -> Option<Vec<String>> {
        match self.shape(expression) {
            Shape::Record(fields) => Some(fields.into_keys().collect()),
            _ => None,
        }
    }
    fn known_user_parameters(&self, expression: &Expr<'s>) -> Option<Vec<Option<ValueType>>> {
        let ExprKind::Name(name) = &expression.kind else {
            return None;
        };
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name.text))
            .and_then(|binding| self.function_parameters.get(&binding.1.start))
            .cloned()
    }
    fn call_result_shape(&self, callee: &Expr<'s>) -> Shape {
        if let Some(builtin) = self.known_builtin(callee) {
            use crate::Builtin;
            let size = match builtin {
                Builtin::Vec2 => Some(2),
                Builtin::Vec3
                | Builtin::Cross
                | Builtin::TransformPoint
                | Builtin::TransformDirection => Some(3),
                Builtin::Vec4 => Some(4),
                _ => None,
            };
            if let Some(size) = size {
                return Shape::Typed(ValueType::Vector(size));
            }
            let ty = match builtin {
                Builtin::Identity
                | Builtin::Translation
                | Builtin::Scaling
                | Builtin::RotationX
                | Builtin::RotationY
                | Builtin::RotationZ
                | Builtin::RotationMatrix => Some(ValueType::Matrix4),
                Builtin::AxisAngle | Builtin::Slerp => Some(ValueType::Quaternion),
                Builtin::Degrees | Builtin::Radians => Some(ValueType::Angle),
                Builtin::Mesh | Builtin::GridMesh | Builtin::Transform => Some(ValueType::Mesh),
                Builtin::Polygon | Builtin::Translate | Builtin::Rotate => Some(ValueType::Polygon),
                _ => None,
            };
            if let Some(ty) = ty {
                return Shape::Typed(ty);
            }
        }
        if let Some(function) = self.known_host(callee) {
            return Shape::Typed(function.result.clone());
        }
        if let ExprKind::Name(name) = &callee.kind
            && let Some(binding) = self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name.text))
        {
            return self
                .function_results
                .get(&binding.1.start)
                .map_or(Shape::Unknown, |ty| Shape::Typed(ty.clone()));
        }
        if let ExprKind::Name(name) = &callee.kind
            && !self
                .scopes
                .iter()
                .rev()
                .any(|scope| scope.contains_key(name.text))
        {
            let dimensions = match name.text {
                "vec2" => Some(2),
                "vec3" => Some(3),
                "vec4" => Some(4),
                _ => None,
            };
            if let Some(dimensions) = dimensions {
                return Shape::Typed(ValueType::Vector(dimensions));
            }
        }
        Shape::Unknown
    }
    fn applied_result_shape(&self, callee: &Expr<'s>, supplied: &[Shape]) -> Shape {
        use crate::Builtin;
        match (self.known_builtin(callee), supplied) {
            (Some(Builtin::Normalize), [shape @ Shape::Typed(ValueType::Vector(_))]) => {
                shape.clone()
            }
            (
                Some(Builtin::Lerp),
                [
                    left @ Shape::Typed(ValueType::Vector(_) | ValueType::Number),
                    right,
                    _,
                ],
            ) if left == right => left.clone(),
            _ => self.call_result_shape(callee),
        }
    }
    fn shape(&self, expression: &Expr<'s>) -> Shape {
        match &expression.kind {
            ExprKind::Name(name) => self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name.text))
                .map_or(Shape::Unknown, |binding| {
                    self.binding_shapes
                        .get(&binding.1.start)
                        .cloned()
                        .unwrap_or_else(|| {
                            self.binding_types
                                .get(&binding.1.start)
                                .map_or(Shape::Unknown, |ty| Shape::Typed(ty.clone()))
                        })
                }),
            ExprKind::Number(_) => Shape::Typed(ValueType::Number),
            ExprKind::String(_) => Shape::Typed(ValueType::String),
            ExprKind::Bool(_) => Shape::Typed(ValueType::Bool),
            ExprKind::Null => Shape::Typed(ValueType::Null),
            ExprKind::List(items) => {
                Shape::List(items.iter().map(|item| self.shape(item)).collect())
            }
            ExprKind::Tuple(items) => {
                Shape::Tuple(items.iter().map(|item| self.shape(item)).collect())
            }
            ExprKind::Map(entries) => {
                let mut fields = std::collections::BTreeMap::new();
                for (key, value) in entries {
                    let ExprKind::Name(name) = &key.kind else {
                        return Shape::Unsupported;
                    };
                    fields.insert(name.text.to_owned(), self.shape(value));
                }
                Shape::Record(fields)
            }
            ExprKind::Lambda { .. } => Shape::Unsupported,
            ExprKind::Call { callee, arguments } => self.applied_result_shape(
                callee,
                &arguments
                    .iter()
                    .map(|arg| self.shape(arg))
                    .collect::<Vec<_>>(),
            ),
            ExprKind::Index { object, index } => {
                let position = Self::literal_number(index)
                    .filter(|n| {
                        n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < usize::MAX as f64
                    })
                    .map(|n| n as usize);
                match self.shape(object) {
                    Shape::Record(mut fields) => Self::literal_string(index)
                        .and_then(|key| fields.remove(&key))
                        .unwrap_or(Shape::Unknown),
                    Shape::Typed(ValueType::Vector(_)) => Shape::Typed(ValueType::Number),
                    Shape::Typed(ValueType::List(element)) => Shape::Typed(*element),
                    Shape::Typed(ValueType::Tuple(items)) => position
                        .and_then(|i| items.get(i).cloned())
                        .map_or(Shape::Unknown, Shape::Typed),
                    Shape::Tuple(items) | Shape::List(items) => position
                        .and_then(|i| items.get(i).cloned())
                        .unwrap_or(Shape::Unknown),
                    _ => Shape::Unknown,
                }
            }
            ExprKind::Pipeline { input, stages } => {
                let mut result = self.shape(input);
                for stage in stages {
                    let mut supplied = vec![result];
                    let callee = if let ExprKind::Call { callee, arguments } = &stage.kind {
                        supplied.extend(arguments.iter().map(|arg| self.shape(arg)));
                        callee.as_ref()
                    } else {
                        stage
                    };
                    result = self.applied_result_shape(callee, &supplied);
                }
                result
            }
            ExprKind::Member { object, field } => {
                if let Shape::Record(fields) = self.shape(object) {
                    return fields.get(field.text).cloned().unwrap_or(Shape::Unknown);
                }
                if let Shape::Typed(ValueType::Mesh) = self.shape(object) {
                    return match field.text {
                        "vertices" => Shape::Typed(ValueType::List(Box::new(ValueType::Vector(3)))),
                        "triangles" => Shape::Typed(ValueType::List(Box::new(ValueType::List(
                            Box::new(ValueType::Number),
                        )))),
                        _ => Shape::Unknown,
                    };
                }
                let axis = match field.text {
                    "x" => Some(0),
                    "y" => Some(1),
                    "z" => Some(2),
                    "w" => Some(3),
                    _ => None,
                };
                if let (Shape::Typed(ValueType::Vector(size)), Some(axis)) =
                    (self.shape(object), axis)
                    && axis < size
                {
                    Shape::Typed(ValueType::Number)
                } else {
                    Shape::Unknown
                }
            }
            ExprKind::Unary {
                operator: "not" | "!",
                ..
            } => Shape::Typed(ValueType::Bool),
            ExprKind::Unary {
                operator: "+" | "-",
                value,
            } => {
                let shape = self.shape(value);
                match shape {
                    Shape::Typed(ValueType::Number | ValueType::Vector(_)) => shape,
                    _ => Shape::Unknown,
                }
            }
            ExprKind::Binary {
                operator: "==" | "!=" | "<" | ">" | "<=" | ">=" | "and" | "or" | "&&" | "||",
                ..
            } => Shape::Typed(ValueType::Bool),
            ExprKind::Binary {
                left,
                right,
                operator,
            } => {
                let left = self.shape(left);
                let right = self.shape(right);
                match (&left, &right, *operator) {
                    (Shape::Typed(ValueType::Matrix4), Shape::Typed(ValueType::Matrix4), "*")
                    | (
                        Shape::Typed(ValueType::Quaternion),
                        Shape::Typed(ValueType::Quaternion),
                        "*",
                    )
                    | (Shape::Typed(ValueType::Angle), Shape::Typed(ValueType::Angle), "+" | "-")
                    | (
                        Shape::Typed(ValueType::Angle),
                        Shape::Typed(ValueType::Number),
                        "*" | "/",
                    ) => return left,
                    (Shape::Typed(ValueType::Number), Shape::Typed(ValueType::Angle), "*") => {
                        return right;
                    }
                    (
                        Shape::Typed(ValueType::Vector(a)),
                        Shape::Typed(ValueType::Vector(b)),
                        "+" | "-",
                    ) if a == b => return left,
                    (
                        Shape::Typed(ValueType::Vector(_)),
                        Shape::Typed(ValueType::Number),
                        "*" | "/",
                    ) => return left,
                    (Shape::Typed(ValueType::Number), Shape::Typed(ValueType::Vector(_)), "*") => {
                        return right;
                    }
                    _ => {}
                }
                if right == left
                    && (left == Shape::Typed(ValueType::Number)
                        || (*operator == "+" && left == Shape::Typed(ValueType::String)))
                {
                    left
                } else {
                    Shape::Unknown
                }
            }
            ExprKind::If {
                then_value,
                else_value,
                ..
            } => {
                let left = self.shape(then_value);
                if left == self.shape(else_value) {
                    left
                } else {
                    Shape::Unknown
                }
            }
            _ => Shape::Unknown,
        }
    }
    fn known_non_callable(&self, value: &Expr<'s>) -> bool {
        matches!(value.kind, ExprKind::Map(_))
            || matches!(
                self.shape(value),
                Shape::Typed(_) | Shape::List(_) | Shape::Tuple(_) | Shape::Record(_)
            )
    }
    fn check_math_arguments(&mut self, builtin: crate::Builtin, supplied: &[(Span, Shape)]) {
        use crate::Builtin;
        let signature: &[ValueType] = match builtin {
            Builtin::Translation | Builtin::Scaling => &[ValueType::Vector(3)],
            Builtin::TransformPoint | Builtin::TransformDirection => {
                &[ValueType::Matrix4, ValueType::Vector(3)]
            }
            Builtin::RotationMatrix => &[ValueType::Quaternion],
            Builtin::Slerp => &[
                ValueType::Quaternion,
                ValueType::Quaternion,
                ValueType::Number,
            ],
            Builtin::Transform => &[ValueType::Mesh, ValueType::Matrix4],
            Builtin::Translate => &[ValueType::Polygon, ValueType::Vector(2)],
            Builtin::Rotate => &[ValueType::Polygon],
            Builtin::AxisAngle => &[ValueType::Vector(3)],
            Builtin::Degrees | Builtin::Radians => &[ValueType::Number],
            _ => &[],
        };
        for ((span, shape), expected) in supplied.iter().zip(signature) {
            if shape.rejects(expected) {
                self.error(
                    *span,
                    "argument-type",
                    "Transformation argument does not match operation",
                );
            }
        }
        let angle_position = match builtin {
            Builtin::RotationX | Builtin::RotationY | Builtin::RotationZ => Some(0),
            Builtin::AxisAngle | Builtin::Rotate => Some(1),
            _ => None,
        };
        if let Some((span, shape)) = angle_position.and_then(|index| supplied.get(index))
            && shape.rejects(&ValueType::Number)
            && shape.rejects(&ValueType::Angle)
        {
            self.error(
                *span,
                "argument-type",
                "Rotation angle must be a number or angle",
            );
        }
        let vector_count = match builtin {
            Builtin::Normalize | Builtin::Length => 1,
            Builtin::Cross | Builtin::Dot => 2,
            _ => 0,
        };
        for (span, shape) in supplied.iter().take(vector_count) {
            let rejected = if builtin == Builtin::Cross {
                shape.rejects(&ValueType::Vector(3))
            } else {
                !matches!(shape, Shape::Unknown | Shape::Typed(ValueType::Vector(_)))
            };
            if rejected {
                self.error(
                    *span,
                    "argument-type",
                    "Mathematical argument requires a compatible vector",
                );
            }
        }
        if builtin == Builtin::Lerp {
            for (span, shape) in supplied.iter().take(2) {
                if !matches!(
                    shape,
                    Shape::Unknown | Shape::Typed(ValueType::Number | ValueType::Vector(_))
                ) {
                    self.error(*span, "argument-type", "lerp requires numbers or vectors");
                }
            }
            if let Some((span, shape)) = supplied.get(2)
                && shape.rejects(&ValueType::Number)
            {
                self.error(*span, "argument-type", "lerp factor must be a number");
            }
        }
        if matches!(builtin, Builtin::Dot | Builtin::Lerp)
            && let [(_, left), (span, right), ..] = supplied
        {
            match (left, right) {
                (Shape::Typed(ValueType::Vector(a)), Shape::Typed(ValueType::Vector(b)))
                    if a != b =>
                {
                    self.error(*span, "vector-dimensions", "Vector dimensions must match");
                }
                (Shape::Typed(ValueType::Vector(_)), Shape::Typed(ValueType::Number))
                | (Shape::Typed(ValueType::Number), Shape::Typed(ValueType::Vector(_)))
                    if builtin == Builtin::Lerp =>
                {
                    self.error(
                        *span,
                        "argument-type",
                        "lerp endpoints must have matching types",
                    );
                }
                _ => (),
            }
        }
    }
    fn call(
        &mut self,
        callee: &Expr<'s>,
        arguments: &[Expr<'s>],
        implicit: Option<(Span, Shape)>,
    ) -> Shape {
        if self.calls
            && let Some(builtin) = self.known_builtin(callee)
        {
            let supplied = implicit
                .iter()
                .cloned()
                .chain(arguments.iter().map(|arg| (arg.span, self.shape(arg))))
                .collect::<Vec<_>>();
            self.check_math_arguments(builtin, &supplied);
        }
        let result_shape = self.applied_result_shape(
            callee,
            &implicit
                .iter()
                .map(|(_, shape)| shape.clone())
                .chain(arguments.iter().map(|arg| self.shape(arg)))
                .collect::<Vec<_>>(),
        );
        if self.calls && self.known_non_callable(callee) {
            self.error(
                callee.span,
                "not-callable",
                "Value cannot be called as a function",
            );
        }
        if self.calls
            && let Some(builtin) = self.known_builtin(callee)
        {
            for &(position, expected) in builtin.callback_arities() {
                if let Some(index) = position.checked_sub(usize::from(implicit.is_some()))
                    && let Some(callback) = arguments.get(index)
                    && self.known_non_callable(callback)
                {
                    self.error(callback.span, "callback-type", "Callback must be callable");
                }
                if let Some(index) = position.checked_sub(usize::from(implicit.is_some()))
                    && let Some(callback) = arguments.get(index)
                    && let Some(arities) = self.known_arity(callback)
                    && !arities.iter().any(|arity| arity.contains(&expected))
                {
                    self.error(
                        callback.span,
                        "callback-argument-count",
                        "Callback argument count does not match operation",
                    );
                }
            }
        }
        if self.calls
            && let Some(arity) = self.known_arity(callee)
            && !arity
                .iter()
                .any(|range| range.contains(&(arguments.len() + usize::from(implicit.is_some()))))
        {
            self.error(
                callee.span,
                "argument-count",
                "Argument count does not match function signature",
            );
        }
        let user_parameters = self.known_user_parameters(callee);
        if self.calls
            && let Some(parameters) = user_parameters
        {
            let supplied = implicit
                .iter()
                .cloned()
                .chain(
                    arguments
                        .iter()
                        .map(|argument| (argument.span, self.shape(argument))),
                )
                .collect::<Vec<_>>();
            for ((span, shape), expected) in supplied.iter().zip(parameters) {
                if let Some(expected) = expected
                    && shape.rejects(&expected)
                {
                    self.error(
                        *span,
                        "argument-type",
                        "Argument type does not match function annotation",
                    );
                }
            }
        }
        let host = self.known_host(callee);
        if self.calls
            && let Some(host) = &host
        {
            let supplied = implicit
                .into_iter()
                .chain(
                    arguments
                        .iter()
                        .map(|argument| (argument.span, self.shape(argument))),
                )
                .collect::<Vec<_>>();
            for ((span, shape), expected) in supplied.iter().zip(&host.parameters) {
                if shape.rejects(expected) {
                    self.error(
                        *span,
                        "argument-type",
                        "Argument type does not match registered host signature",
                    );
                }
            }
        }
        self.expr(callee);
        for argument in arguments {
            self.expr(argument);
        }
        result_shape
    }
    fn condition(&mut self, expression: &Expr<'s>) {
        if self.calls && self.shape(expression).rejects(&ValueType::Bool) {
            self.error(
                expression.span,
                "condition-type",
                "Condition must be a boolean",
            );
        }
        self.expr(expression);
    }
    fn expr(&mut self, expr: &Expr<'s>) {
        match &expr.kind {
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                self.condition(condition);
                self.expr(then_value);
                self.expr(else_value);
            }
            ExprKind::Lambda { parameters, body } => {
                self.scopes.push(HashMap::new());
                self.scope_spans.push(body.span);
                for parameter in parameters {
                    self.pattern(parameter, body.span.start);
                }
                self.expr(body);
                self.scopes.pop();
                self.scope_spans.pop();
            }
            ExprKind::Assign { target, value, .. } => {
                if let ExprKind::Name(name) = &target.kind
                    && self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                        .map(|binding| binding.0)
                        == Some(true)
                {
                    self.error(
                        name.span,
                        "immutable-binding",
                        "Cannot assign to an immutable binding",
                    );
                }
                self.expr(target);
                self.expr(value);
            }
            ExprKind::Unary {
                operator: "not" | "!",
                value,
            } => self.condition(value),
            ExprKind::Unary {
                operator: "+" | "-",
                value,
            } => {
                let shape = self.shape(value);
                if self.calls
                    && [
                        ValueType::Number,
                        ValueType::Vector(2),
                        ValueType::Vector(3),
                        ValueType::Vector(4),
                    ]
                    .iter()
                    .all(|ty| shape.rejects(ty))
                {
                    self.error(
                        value.span,
                        "unary-operand",
                        "Unary sign requires a number or vector",
                    );
                }
                self.expr(value);
            }
            ExprKind::Unary { value, .. } => self.expr(value),
            ExprKind::Binary {
                left,
                right,
                operator: "and" | "or" | "&&" | "||",
            } => {
                self.condition(left);
                self.condition(right);
            }
            ExprKind::Binary {
                left,
                right,
                operator,
            } => {
                if self.calls && matches!(*operator, "<" | ">" | "<=" | ">=") {
                    for operand in [left, right] {
                        if self.shape(operand).rejects(&ValueType::Number) {
                            self.error(
                                operand.span,
                                "comparison-operand",
                                "Ordered comparison requires a number",
                            );
                        }
                    }
                }
                if self.calls && matches!(*operator, "+" | "-" | "*" | "/" | "%" | "**") {
                    let a = self.shape(left);
                    let b = self.shape(right);
                    if let (Shape::Typed(a), Shape::Typed(b)) = (&a, &b)
                        && (matches!(a, ValueType::Vector(_)) || matches!(b, ValueType::Vector(_)))
                    {
                        let valid = match (a, b, *operator) {
                            (ValueType::Vector(a), ValueType::Vector(b), "+" | "-") => a == b,
                            (ValueType::Vector(_), ValueType::Number, "*" | "/")
                            | (ValueType::Number, ValueType::Vector(_), "*") => true,
                            _ => false,
                        };
                        if !valid {
                            self.error(
                                expr.span,
                                "vector-operands",
                                "Invalid vector operands or dimensions",
                            );
                        }
                    }
                    if let (Shape::Typed(a), Shape::Typed(b)) = (&a, &b)
                        && (matches!(
                            a,
                            ValueType::Matrix4 | ValueType::Quaternion | ValueType::Angle
                        ) || matches!(
                            b,
                            ValueType::Matrix4 | ValueType::Quaternion | ValueType::Angle
                        ))
                        && !matches!(a, ValueType::Vector(_))
                        && !matches!(b, ValueType::Vector(_))
                    {
                        let valid = matches!(
                            (a, b, *operator),
                            (ValueType::Matrix4, ValueType::Matrix4, "*")
                                | (ValueType::Quaternion, ValueType::Quaternion, "*")
                                | (ValueType::Angle, ValueType::Angle, "+" | "-")
                                | (ValueType::Angle, ValueType::Number, "*" | "/")
                                | (ValueType::Number, ValueType::Angle, "*")
                        );
                        if !valid {
                            self.error(
                                expr.span,
                                "math-operands",
                                "Invalid matrix, quaternion or angle operands",
                            );
                        }
                    }
                }
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Call { callee, arguments } => {
                self.call(callee, arguments, None);
            }
            ExprKind::Member { object, field } => {
                let check_member = self.calls && !field.text.is_empty();
                if check_member {
                    let unsupported = match self.shape(object) {
                        Shape::Typed(ty) => !matches!(ty, ValueType::Vector(_) | ValueType::Mesh),
                        Shape::List(_) | Shape::Tuple(_) => true,
                        _ => false,
                    };
                    if unsupported {
                        self.error(
                            field.span,
                            "member-object",
                            "Value does not support member access",
                        );
                    }
                }
                if self.collect_members {
                    let members = match self.shape(object) {
                        Shape::Record(fields) => fields.into_keys().collect(),
                        Shape::Typed(ValueType::Vector(size)) => ["x", "y", "z", "w"]
                            .iter()
                            .take(size)
                            .map(|name| (*name).to_owned())
                            .collect(),
                        Shape::Typed(ValueType::Mesh) => {
                            vec!["triangles".into(), "vertices".into()]
                        }
                        _ => Vec::new(),
                    };
                    self.member_completions.push(MemberCompletion {
                        name_span: field.span,
                        members,
                    });
                }
                if check_member
                    && matches!(self.shape(object), Shape::Typed(ValueType::Mesh))
                    && !matches!(field.text, "vertices" | "triangles")
                {
                    self.error(field.span, "mesh-field", "Unknown mesh field");
                }
                if check_member
                    && let Some(fields) = self.known_record_fields(object)
                    && !fields.iter().any(|name| name == field.text)
                {
                    self.error(
                        field.span,
                        "unknown-record-field",
                        "Unknown field in record",
                    );
                }
                if check_member && let Shape::Typed(ValueType::Vector(size)) = self.shape(object) {
                    let axis = match field.text {
                        "x" => Some(0),
                        "y" => Some(1),
                        "z" => Some(2),
                        "w" => Some(3),
                        _ => None,
                    };
                    if axis.is_none_or(|axis| axis >= size) {
                        self.error(
                            field.span,
                            "vector-component",
                            "Unknown or out-of-bounds vector component",
                        );
                    }
                }
                self.expr(object);
            }
            ExprKind::Index { object, index } => {
                if self.calls {
                    self.check_index(object, index);
                }
                self.expr(object);
                self.expr(index);
            }
            ExprKind::List(values) | ExprKind::Tuple(values) => {
                for value in values {
                    self.expr(value);
                }
            }
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    if !matches!(key.kind, ExprKind::Name(_)) {
                        self.expr(key);
                    }
                    self.expr(value);
                }
            }
            ExprKind::Pipeline { input, stages } => {
                let shape = if self.calls {
                    self.shape(input)
                } else {
                    Shape::Unknown
                };
                let mut supplied = (input.span, shape);
                self.expr(input);
                for stage in stages {
                    let result = match &stage.kind {
                        ExprKind::Call { callee, arguments } => {
                            self.call(callee, arguments, Some(supplied))
                        }
                        _ => self.call(stage, &[], Some(supplied)),
                    };
                    supplied = (stage.span, result);
                }
            }
            ExprKind::Match { value, arms } => {
                self.expr(value);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.scope_spans.push(arm.span);
                    self.match_bindings(&arm.pattern, arm.pattern.span.end);
                    if let Some(guard) = &arm.guard {
                        self.condition(guard);
                    }
                    self.expr(&arm.value);
                    self.scopes.pop();
                    self.scope_spans.pop();
                }
            }
            ExprKind::Name(name) => {
                let definition = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                    .map(|binding| binding.1);
                self.references.push(NameReference {
                    usage: name.span,
                    definition,
                });
            }
            ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Bool(_)
            | ExprKind::Null
            | ExprKind::Error => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Shape {
    Unknown,
    Typed(ValueType),
    List(Vec<Shape>),
    Tuple(Vec<Shape>),
    Record(std::collections::BTreeMap<String, Shape>),
    // Functions and records with unknown keys have no current ValueType contract.
    Unsupported,
}
impl Shape {
    fn member_paths(
        &self,
        prefix: &str,
        output: &mut std::collections::BTreeMap<String, Vec<String>>,
    ) {
        match self {
            Self::Record(fields) => {
                if !prefix.is_empty() {
                    output.insert(prefix.to_owned(), fields.keys().cloned().collect());
                }
                for (field, shape) in fields {
                    let path = if prefix.is_empty() {
                        field.clone()
                    } else {
                        format!("{prefix}.{field}")
                    };
                    shape.member_paths(&path, output);
                }
            }
            Self::Typed(ValueType::Mesh) if !prefix.is_empty() => {
                output.insert(
                    prefix.to_owned(),
                    vec!["triangles".into(), "vertices".into()],
                );
            }
            Self::Typed(ValueType::Vector(size)) if !prefix.is_empty() => {
                output.insert(
                    prefix.to_owned(),
                    ["x", "y", "z", "w"]
                        .iter()
                        .take(*size)
                        .map(|name| (*name).to_owned())
                        .collect(),
                );
            }
            _ => {}
        }
    }
    fn rejects(&self, expected: &ValueType) -> bool {
        match self {
            Self::Unknown => false,
            Self::Unsupported | Self::Record(_) => true,
            Self::Typed(actual) => disjoint(actual, expected),
            Self::List(items) => match expected {
                ValueType::List(element) => items.iter().any(|item| item.rejects(element)),
                _ => true,
            },
            Self::Tuple(items) => match expected {
                ValueType::Tuple(types) => {
                    items.len() != types.len()
                        || items.iter().zip(types).any(|(item, ty)| item.rejects(ty))
                }
                _ => true,
            },
        }
    }
}
// True only when the two runtime contracts accept no common value.
fn disjoint(actual: &ValueType, expected: &ValueType) -> bool {
    match (actual, expected) {
        (ValueType::List(_), ValueType::List(_)) | (ValueType::Option(_), ValueType::Option(_)) => {
            false
        }
        (ValueType::Tuple(a), ValueType::Tuple(b)) => {
            a.len() != b.len() || a.iter().zip(b).any(|(a, b)| disjoint(a, b))
        }
        (ValueType::Result(ok_a, err_a), ValueType::Result(ok_b, err_b)) => {
            disjoint(ok_a, ok_b) && disjoint(err_a, err_b)
        }
        _ => actual != expected,
    }
}
