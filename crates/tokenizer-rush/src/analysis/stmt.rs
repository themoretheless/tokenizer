use super::{BlockExits, Checker, NameReference, Shape, UserDefinition};
use crate::{Block, Expr, ExprKind, Name, Stmt, StmtKind, ValueType};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

impl<'s> Checker<'s> {
    pub(super) fn statements(&mut self, statements: &[Stmt<'s>]) {
        self.hoist_declarations(statements);
        for statement in statements {
            self.statement(statement);
        }
    }

    fn collect_pattern_names(pattern: &Expr<'s>, out: &mut HashSet<&'s str>) {
        match &pattern.kind {
            ExprKind::Name(name) if name.text != "_" => {
                out.insert(name.text);
            }
            ExprKind::Tuple(items) => {
                for item in items {
                    Self::collect_pattern_names(item, out);
                }
            }
            ExprKind::Map(entries) => {
                for (_, item) in entries {
                    Self::collect_pattern_names(item, out);
                }
            }
            _ => {}
        }
    }

    fn bind_pattern_silent(&mut self, pattern: &Expr<'s>, ids: &mut Vec<usize>) {
        match &pattern.kind {
            ExprKind::Name(name) if name.text != "_" => {
                let scope = self.scopes.last_mut().unwrap();
                if !scope.contains_key(name.text) {
                    scope.insert(name.text, (true, name.span, None, None));
                    ids.push(name.span.start);
                }
            }
            ExprKind::Tuple(items) => {
                for item in items {
                    self.bind_pattern_silent(item, ids);
                }
            }
            ExprKind::Map(entries) => {
                for (_, item) in entries {
                    self.bind_pattern_silent(item, ids);
                }
            }
            _ => {}
        }
    }

    fn is_reserved_type_name(name: &Name<'s>) -> bool {
        crate::builtin_catalog()
            .iter()
            .any(|(n, _)| *n == name.text)
            || matches!(name.text, "Option" | "Result" | "list" | "tuple")
            || ValueType::annotation(&crate::Type {
                name: name.clone(),
                path: vec![name.clone()],
                arguments: Vec::new(),
            })
            .is_ok()
    }

    fn hoist_declarations(&mut self, statements: &[Stmt<'s>]) {
        let mut claimed = HashSet::new();
        let mut hoisted_types = Vec::new();
        let mut hoisted_functions = Vec::new();

        // Pass 1a: Register import, struct, enum, and function bindings in scope.
        for (idx, statement) in statements.iter().enumerate() {
            match &statement.kind {
                StmtKind::Declaration { name, .. } => {
                    claimed.insert(name.text);
                }
                StmtKind::Destructure { pattern, .. } => {
                    Self::collect_pattern_names(pattern, &mut claimed);
                }
                StmtKind::Import(name) => {
                    if claimed.insert(name.text)
                        && !self.scopes.last().unwrap().contains_key(name.text)
                    {
                        self.scopes
                            .last_mut()
                            .unwrap()
                            .insert(name.text, (true, name.span, None, None));
                        self.hoisted_declarations.insert(name.span.start);
                        self.imports.insert(name.text.to_owned());
                        self.binding_modules
                            .insert(name.span.start, name.text.to_owned());
                        if let Some(interface) = self
                            .module_exports
                            .get(name.text)
                            .filter(|i| !i.fields.is_empty())
                            .cloned()
                        {
                            for (id, definition) in &interface.types {
                                self.user_types.insert(id.clone(), definition.clone());
                            }
                            for (field, shape) in &interface.fields {
                                let id = match shape {
                                    Shape::Constructor(id, _) => Some(id.clone()),
                                    Shape::Record(fields) => fields.values().find_map(|s| {
                                        if let Shape::Constructor(id, _) = s {
                                            Some(id.clone())
                                        } else {
                                            None
                                        }
                                    }),
                                    _ => None,
                                };
                                if let Some(id) = id {
                                    self.type_names
                                        .insert(format!("{}.{}", name.text, field), id);
                                }
                            }
                            self.record_fields.insert(
                                name.span.start,
                                interface.fields.keys().cloned().collect(),
                            );
                            self.binding_shapes
                                .insert(name.span.start, Shape::Record(interface.fields.clone()));
                        }
                    }
                }
                StmtKind::Struct { name, .. } | StmtKind::Enum { name, .. } => {
                    if self.scopes.len() == 1
                        && !Self::is_reserved_type_name(name)
                        && claimed.insert(name.text)
                        && !self.scopes.last().unwrap().contains_key(name.text)
                    {
                        self.scopes
                            .last_mut()
                            .unwrap()
                            .insert(name.text, (true, name.span, None, None));
                        self.hoisted_declarations.insert(name.span.start);
                        let identity = self.identity(name.text);
                        self.type_names
                            .insert(name.text.to_owned(), identity.clone());
                        self.user_types.insert(
                            identity,
                            Rc::new(UserDefinition {
                                span: name.span,
                                module: self.module.clone(),
                                field_spans: HashMap::new(),
                                variant_spans: HashMap::new(),
                                fields: None,
                                variants: Default::default(),
                            }),
                        );
                        hoisted_types.push(idx);
                    }
                }
                StmtKind::Function {
                    name,
                    parameters,
                    body,
                    is_async,
                    ..
                } if claimed.insert(name.text)
                    && !self.scopes.last().unwrap().contains_key(name.text) =>
                {
                    let arity =
                        Some(std::iter::once(parameters.len()..=parameters.len()).collect());
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(name.text, (true, name.span, arity, None));
                    self.hoisted_declarations.insert(name.span.start);
                    // Async functions suspend by design; a synchronous call
                    // spawns a task and returns its handle, so they are not
                    // "suspending" for the coroutine-call check.
                    if !*is_async && Self::contains_yield(&body.stmts) {
                        self.suspending_functions.insert(name.span.start);
                    }
                    hoisted_functions.push(idx);
                }
                _ => {}
            }
        }

        // Pass 1b: Resolve struct fields and enum variants now that all module type names exist.
        for &idx in &hoisted_types {
            match &statements[idx].kind {
                StmtKind::Struct { name, fields } => {
                    let field_spans = fields
                        .iter()
                        .map(|(n, _)| (n.text.to_owned(), n.span))
                        .collect();
                    let fields = fields
                        .iter()
                        .filter_map(|(name, ty)| {
                            ValueType::annotation_with(ty, &|n| self.resolve_type(n))
                                .ok()
                                .map(|ty| (name.text.to_owned(), ty))
                        })
                        .collect();
                    let id = self.identity(name.text);
                    if let Some(definition) = self.user_types.get_mut(&id) {
                        let definition = Rc::make_mut(definition);
                        definition.fields = Some(fields);
                        definition.field_spans = field_spans;
                    }
                    self.constructor_aliases
                        .insert(name.span.start, (id.clone(), None));
                    self.function_results
                        .insert(name.span.start, ValueType::User(id));
                }
                StmtKind::Enum { name, variants } => {
                    let variant_spans = variants
                        .iter()
                        .map(|(n, _)| (n.text.to_owned(), n.span))
                        .collect();
                    let mut shapes = BTreeMap::new();
                    let mut contracts = BTreeMap::new();
                    let id = self.identity(name.text);
                    for (variant, types) in variants {
                        let types = types
                            .iter()
                            .filter_map(|ty| {
                                ValueType::annotation_with(ty, &|n| self.resolve_type(n)).ok()
                            })
                            .collect();
                        contracts.insert(variant.text.to_owned(), types);
                        shapes.insert(
                            variant.text.to_owned(),
                            Shape::Constructor(id.clone(), Some(variant.text.to_owned())),
                        );
                    }
                    if let Some(definition) = self.user_types.get_mut(&id) {
                        let definition = Rc::make_mut(definition);
                        definition.variants = contracts;
                        definition.variant_spans = variant_spans;
                    }
                    self.record_fields
                        .insert(name.span.start, shapes.keys().cloned().collect());
                    self.binding_shapes
                        .insert(name.span.start, Shape::Record(shapes));
                }
                _ => {}
            }
        }

        // Pass 1c: Initialize function signatures and pre-converge contracts across the scope.
        for &idx in &hoisted_functions {
            if let StmtKind::Function {
                name,
                parameters,
                result,
                ..
            } = &statements[idx].kind
            {
                if let Some(ty) = result
                    .as_ref()
                    .and_then(|ty| ValueType::annotation_with(ty, &|n| self.resolve_type(n)).ok())
                {
                    self.function_results.insert(name.span.start, ty);
                }
                let parameter_types = parameters
                    .iter()
                    .map(|p| {
                        p.ty.as_ref().and_then(|ty| {
                            ValueType::annotation_with(ty, &|n| self.resolve_type(n)).ok()
                        })
                    })
                    .collect();
                self.function_parameters
                    .insert(name.span.start, parameter_types);
            }
        }

        if !hoisted_functions.is_empty() {
            let saved_valid = self.valid;
            let saved_diag_len = self.diagnostics.len();
            let mut temp_ids = Vec::new();
            for _ in 0..4 {
                let mut changed = false;
                for &idx in &hoisted_functions {
                    let StmtKind::Function {
                        name,
                        parameters,
                        body,
                        result,
                        ..
                    } = &statements[idx].kind
                    else {
                        continue;
                    };
                    let explicit_result = result.as_ref().and_then(|ty| {
                        ValueType::annotation_with(ty, &|n| self.resolve_type(n)).ok()
                    });
                    let mut seed = explicit_result
                        .clone()
                        .or_else(|| self.function_results.get(&name.span.start).cloned());
                    let prev_seed = seed.clone();
                    let prev_params = self.function_parameters.get(&name.span.start).cloned();

                    self.scopes.push(HashMap::new());
                    for parameter in parameters {
                        self.bind_pattern_silent(&parameter.pattern, &mut temp_ids);
                        if let Some(annotation) = &parameter.ty
                            && let Ok(ty) =
                                ValueType::annotation_with(annotation, &|n| self.resolve_type(n))
                        {
                            self.pattern_type(&parameter.pattern, &ty);
                        }
                    }
                    let parameter_ids: HashSet<_> = self
                        .scopes
                        .last()
                        .unwrap()
                        .values()
                        .map(|b| b.1.start)
                        .collect();
                    self.constrain_body(&body.stmts, seed.clone(), &parameter_ids);
                    let mut candidates = Vec::new();
                    self.return_candidates(&body.stmts, &mut candidates);
                    if explicit_result.is_none() && BlockExits::inspect(&body.stmts).next {
                        candidates.push(Shape::Typed(ValueType::Null));
                    }
                    let known: Vec<_> = candidates
                        .iter()
                        .filter_map(|s| {
                            if let Shape::Typed(t) = s {
                                Some(t.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                    if seed.is_none() && !known.is_empty() && known.iter().all(|t| t == &known[0]) {
                        seed = Some(known[0].clone());
                        self.function_results
                            .insert(name.span.start, known[0].clone());
                    }
                    let inferred: Vec<Option<ValueType>> = parameters
                        .iter()
                        .map(|p| {
                            p.ty.as_ref()
                                .and_then(|t| {
                                    ValueType::annotation_with(t, &|n| self.resolve_type(n)).ok()
                                })
                                .or_else(|| {
                                    if let ExprKind::Name(n) = &p.pattern.kind {
                                        self.scopes
                                            .last()
                                            .unwrap()
                                            .get(n.text)
                                            .and_then(|b| self.binding_types.get(&b.1.start))
                                            .cloned()
                                    } else {
                                        None
                                    }
                                })
                        })
                        .collect();
                    self.function_parameters
                        .insert(name.span.start, inferred.clone());
                    if Self::body_calls_suspending(
                        &body.stmts,
                        &self.scopes,
                        &self.suspending_functions,
                    ) && self.suspending_functions.insert(name.span.start)
                    {
                        changed = true;
                    }
                    self.scopes.pop();
                    if seed != prev_seed || Some(inferred) != prev_params {
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            for id in temp_ids {
                self.binding_types.remove(&id);
                self.binding_shapes.remove(&id);
                self.record_fields.remove(&id);
            }
            self.valid = saved_valid;
            self.diagnostics.truncate(saved_diag_len);
        }
    }

    fn body_calls_suspending(
        statements: &[Stmt<'s>],
        scopes: &[HashMap<&'s str, super::Binding>],
        suspending: &HashSet<usize>,
    ) -> bool {
        statements.iter().any(|stmt| match &stmt.kind {
            StmtKind::Expr(e) | StmtKind::Show(e) | StmtKind::Return(Some(e)) => {
                Self::expr_calls_suspending(e, scopes, suspending)
            }
            StmtKind::Declaration { value, .. } | StmtKind::Destructure { value, .. } => {
                Self::expr_calls_suspending(value, scopes, suspending)
            }
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                Self::expr_calls_suspending(condition, scopes, suspending)
                    || Self::body_calls_suspending(&then_block.stmts, scopes, suspending)
                    || else_block
                        .as_ref()
                        .is_some_and(|b| Self::body_calls_suspending(&b.stmts, scopes, suspending))
            }
            StmtKind::While { condition, body } => {
                Self::expr_calls_suspending(condition, scopes, suspending)
                    || Self::body_calls_suspending(&body.stmts, scopes, suspending)
            }
            StmtKind::For { iterable, body, .. } => {
                Self::expr_calls_suspending(iterable, scopes, suspending)
                    || Self::body_calls_suspending(&body.stmts, scopes, suspending)
            }
            StmtKind::Region { budget, body, .. } => {
                budget
                    .as_ref()
                    .is_some_and(|b| Self::expr_calls_suspending(b, scopes, suspending))
                    || Self::body_calls_suspending(&body.stmts, scopes, suspending)
            }
            _ => false,
        })
    }

    fn expr_calls_suspending(
        expr: &Expr<'s>,
        scopes: &[HashMap<&'s str, super::Binding>],
        suspending: &HashSet<usize>,
    ) -> bool {
        match &expr.kind {
            ExprKind::Call { callee, arguments } => {
                let direct = if let ExprKind::Name(name) = &callee.kind {
                    scopes
                        .iter()
                        .rev()
                        .find_map(|s| s.get(name.text))
                        .is_some_and(|b| suspending.contains(&b.1.start))
                } else {
                    false
                };
                direct
                    || Self::expr_calls_suspending(callee, scopes, suspending)
                    || arguments
                        .iter()
                        .any(|a| Self::expr_calls_suspending(a, scopes, suspending))
            }
            ExprKind::Binary { left, right, .. } => {
                Self::expr_calls_suspending(left, scopes, suspending)
                    || Self::expr_calls_suspending(right, scopes, suspending)
            }
            ExprKind::Pipeline { input, stages } => {
                Self::expr_calls_suspending(input, scopes, suspending)
                    || stages.iter().any(|stage| {
                        let direct = if let ExprKind::Name(name) = &stage.kind {
                            scopes
                                .iter()
                                .rev()
                                .find_map(|s| s.get(name.text))
                                .is_some_and(|b| suspending.contains(&b.1.start))
                        } else {
                            false
                        };
                        direct || Self::expr_calls_suspending(stage, scopes, suspending)
                    })
            }
            ExprKind::Unary { value, .. }
            | ExprKind::Try(value)
            | ExprKind::Member { object: value, .. } => {
                Self::expr_calls_suspending(value, scopes, suspending)
            }
            ExprKind::Index { object, index } => {
                Self::expr_calls_suspending(object, scopes, suspending)
                    || Self::expr_calls_suspending(index, scopes, suspending)
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                Self::expr_calls_suspending(condition, scopes, suspending)
                    || Self::expr_calls_suspending(then_value, scopes, suspending)
                    || Self::expr_calls_suspending(else_value, scopes, suspending)
            }
            ExprKind::List(items) | ExprKind::Tuple(items) => items
                .iter()
                .any(|i| Self::expr_calls_suspending(i, scopes, suspending)),
            ExprKind::Map(entries) => entries.iter().any(|(k, v)| {
                Self::expr_calls_suspending(k, scopes, suspending)
                    || Self::expr_calls_suspending(v, scopes, suspending)
            }),
            _ => false,
        }
    }

    pub(super) fn block(&mut self, block: &Block<'s>) {
        self.scopes.push(HashMap::new());
        self.scope_spans.push(block.span);
        self.statements(&block.stmts);
        self.scopes.pop();
        self.scope_spans.pop();
    }

    pub(super) fn contains_yield(statements: &[Stmt<'s>]) -> bool {
        statements.iter().any(|statement| match &statement.kind {
            StmtKind::Yield(_) => true,
            StmtKind::Expr(expression) => {
                matches!(&expression.kind, ExprKind::Await(_))
            }
            StmtKind::If {
                then_block,
                else_block,
                ..
            } => {
                Self::contains_yield(&then_block.stmts)
                    || else_block
                        .as_ref()
                        .is_some_and(|block| Self::contains_yield(&block.stmts))
            }
            StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                Self::contains_yield(&body.stmts)
            }
            StmtKind::Region { body, .. } => Self::contains_yield(&body.stmts),
            _ => false,
        })
    }

    pub(super) fn known_suspending(&self, value: &Expr<'s>) -> bool {
        if let Shape::Function(_, _, suspends) = self.shape(value) {
            return suspends;
        }
        if let ExprKind::Name(name) = &value.kind {
            self.scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name.text))
                .is_some_and(|binding| self.suspending_functions.contains(&binding.1.start))
        } else {
            false
        }
    }

    pub(super) fn declare_user_type(&mut self, name: &Name<'s>) {
        if self.scopes.len() != 1 {
            self.error(
                name.span,
                "nested-type",
                "User types must be declared at module scope",
            );
        }
        let reserved = Self::is_reserved_type_name(name);
        if reserved {
            self.error(
                name.span,
                "reserved-type",
                "Type name is reserved by the language",
            );
        }
        let scope_start = self.scope_spans.last().unwrap().start;
        self.declare(name, true, scope_start);
        let identity = self.identity(name.text);
        self.type_names
            .insert(name.text.to_owned(), identity.clone());
        self.user_types.entry(identity).or_insert_with(|| {
            Rc::new(UserDefinition {
                span: name.span,
                module: self.module.clone(),
                field_spans: HashMap::new(),
                variant_spans: HashMap::new(),
                fields: None,
                variants: Default::default(),
            })
        });
    }

    pub(super) fn statement(&mut self, statement: &Stmt<'s>) {
        match &statement.kind {
            StmtKind::Struct { name, fields } => {
                self.declare_user_type(name);
                let field_spans = fields
                    .iter()
                    .map(|(n, _)| (n.text.to_owned(), n.span))
                    .collect();
                if self.collect_members {
                    for (field, _) in fields {
                        self.references.push(NameReference {
                            usage: field.span,
                            definition: Some(field.span),
                            definition_module: None,
                        });
                    }
                }
                let fields = fields
                    .iter()
                    .filter_map(|(name, ty)| {
                        self.checked_annotation(ty)
                            .map(|ty| (name.text.to_owned(), ty))
                    })
                    .collect();
                if let Some(definition) = self.user_types.get_mut(&self.identity(name.text)) {
                    let definition = Rc::make_mut(definition);
                    definition.fields = Some(fields);
                    definition.field_spans = field_spans;
                }
                self.constructor_aliases
                    .insert(name.span.start, (self.identity(name.text), None));
                self.function_results
                    .insert(name.span.start, ValueType::User(self.identity(name.text)));
            }
            StmtKind::Enum { name, variants } => {
                self.declare_user_type(name);
                let variant_spans = variants
                    .iter()
                    .map(|(n, _)| (n.text.to_owned(), n.span))
                    .collect();
                if self.collect_members {
                    for (variant, _) in variants {
                        self.references.push(NameReference {
                            usage: variant.span,
                            definition: Some(variant.span),
                            definition_module: None,
                        });
                    }
                }
                let mut shapes = BTreeMap::new();
                let mut contracts = BTreeMap::new();
                for (variant, types) in variants {
                    let types = types
                        .iter()
                        .filter_map(|ty| self.checked_annotation(ty))
                        .collect();
                    contracts.insert(variant.text.to_owned(), types);
                    shapes.insert(
                        variant.text.to_owned(),
                        Shape::Constructor(self.identity(name.text), Some(variant.text.to_owned())),
                    );
                }
                if let Some(definition) = self.user_types.get_mut(&self.identity(name.text)) {
                    let definition = Rc::make_mut(definition);
                    definition.variants = contracts;
                    definition.variant_spans = variant_spans;
                }
                self.record_fields
                    .insert(name.span.start, shapes.keys().cloned().collect());
                self.binding_shapes
                    .insert(name.span.start, Shape::Record(shapes));
            }
            StmtKind::Export(names) => {
                if self.scopes.len() != 1 {
                    self.error(
                        statement.span,
                        "nested-export",
                        "Exports must be declared at module scope",
                    );
                }
                for name in names {
                    if !self.exports.insert(name.text.to_owned()) {
                        self.error(
                            name.span,
                            "duplicate-export",
                            "Name is exported more than once",
                        );
                    }
                    let definition = self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                        .map(|binding| binding.1);
                    if definition.is_none() {
                        self.error(
                            name.span,
                            "unknown-export",
                            "Export must refer to an existing module binding",
                        );
                    }
                    if self.module.is_some()
                        && let Some(binding) = self.scopes[0].get(name.text)
                    {
                        self.require_contract(&self.binding_shape(binding), name.span);
                    }
                    self.references.push(NameReference {
                        definition_module: None,
                        usage: name.span,
                        definition,
                    });
                }
            }
            StmtKind::Import(name) => {
                let scope_start = self.scope_spans.last().unwrap().start;
                self.declare(name, true, scope_start);
                self.imports.insert(name.text.to_owned());
                self.binding_modules
                    .insert(name.span.start, name.text.to_owned());
                if let Some(interface) = self
                    .module_exports
                    .get(name.text)
                    .filter(|i| !i.fields.is_empty())
                    .cloned()
                {
                    for (id, definition) in &interface.types {
                        self.user_types.insert(id.clone(), definition.clone());
                    }
                    for (field, shape) in &interface.fields {
                        let id = match shape {
                            Shape::Constructor(id, _) => Some(id.clone()),
                            Shape::Record(fields) => fields.values().find_map(|s| {
                                if let Shape::Constructor(id, _) = s {
                                    Some(id.clone())
                                } else {
                                    None
                                }
                            }),
                            _ => None,
                        };
                        if let Some(id) = id {
                            self.type_names
                                .insert(format!("{}.{}", name.text, field), id);
                        }
                    }
                    self.record_fields
                        .insert(name.span.start, interface.fields.keys().cloned().collect());
                    self.binding_shapes
                        .insert(name.span.start, Shape::Record(interface.fields.clone()));
                }
            }
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
                        if matches!(
                            shape,
                            Shape::Record(_)
                                | Shape::Tuple(_)
                                | Shape::List(_)
                                | Shape::Alternatives(_)
                        ) {
                            self.binding_shapes.insert(binding.1.start, shape);
                        }
                        if let Some(builtin) = builtin {
                            self.builtin_aliases.insert(binding.1.start, builtin);
                        }
                        if let Some(parameters) = parameters {
                            self.function_parameters.insert(binding.1.start, parameters);
                        }
                        if result != Shape::Unknown {
                            self.function_shapes.insert(binding.1.start, result.clone());
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
                is_async,
            } => {
                // Async functions suspend by design and are called
                // synchronously: the call spawns a task and returns a handle.
                if !*is_async && Self::contains_yield(&body.stmts) {
                    self.suspending_functions.insert(name.span.start);
                }
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
                self.return_types.push(result_type.clone());
                self.inferred_returns.push(Vec::new());
                let pre_inferred_params = self.function_parameters.get(&name.span.start).cloned();
                let parameter_types: Vec<Option<ValueType>> = parameters
                    .iter()
                    .enumerate()
                    .map(|(idx, parameter)| {
                        parameter
                            .ty
                            .as_ref()
                            .and_then(|ty| {
                                ValueType::annotation_with(ty, &|name| self.resolve_type(name)).ok()
                            })
                            .or_else(|| {
                                pre_inferred_params
                                    .as_ref()
                                    .and_then(|v| v.get(idx))
                                    .cloned()
                                    .flatten()
                            })
                    })
                    .collect();
                self.function_parameters
                    .insert(name.span.start, parameter_types.clone());
                let scope_start = self.scope_spans.last().unwrap().start;
                self.declare(name, true, scope_start);
                self.scopes
                    .last_mut()
                    .unwrap()
                    .get_mut(name.text)
                    .unwrap()
                    .2 = Some(std::iter::once(parameters.len()..=parameters.len()).collect());
                self.scopes.push(HashMap::new());
                self.scope_spans.push(body.span);
                for (idx, parameter) in parameters.iter().enumerate() {
                    self.pattern(&parameter.pattern, body.span.start);
                    if let Some(annotation) = &parameter.ty {
                        if let Some(ty) = self.checked_annotation(annotation) {
                            if self.calls {
                                self.check_pattern_shape(
                                    &parameter.pattern,
                                    &Shape::Typed(ty.clone()),
                                );
                            }
                            self.pattern_type(&parameter.pattern, &ty);
                        }
                    } else if let Some(Some(ty)) = parameter_types.get(idx) {
                        self.pattern_type(&parameter.pattern, ty);
                    }
                }
                let parameter_ids: HashSet<_> = self
                    .scopes
                    .last()
                    .unwrap()
                    .values()
                    .map(|b| b.1.start)
                    .collect();
                let mut seed = result_type
                    .clone()
                    .or_else(|| self.function_results.get(&name.span.start).cloned());
                for _ in 0..4 {
                    let previous_seed = seed.clone();
                    let previous_parameters: Vec<_> = parameter_ids
                        .iter()
                        .map(|id| self.binding_types.get(id).cloned())
                        .collect();
                    self.constrain_body(&body.stmts, seed.clone(), &parameter_ids);
                    let mut candidates = Vec::new();
                    self.return_candidates(&body.stmts, &mut candidates);
                    let known: Vec<_> = candidates
                        .iter()
                        .filter_map(|s| {
                            if let Shape::Typed(t) = s {
                                Some(t.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                    if seed.is_none() && !known.is_empty() && known.iter().all(|t| t == &known[0]) {
                        seed = Some(known[0].clone());
                        self.function_results
                            .insert(name.span.start, known[0].clone());
                    }
                    let inferred = parameters
                        .iter()
                        .map(|p| {
                            p.ty.as_ref()
                                .and_then(|t| {
                                    ValueType::annotation_with(t, &|n| self.resolve_type(n)).ok()
                                })
                                .or_else(|| {
                                    if let ExprKind::Name(n) = &p.pattern.kind {
                                        self.scopes
                                            .last()
                                            .unwrap()
                                            .get(n.text)
                                            .and_then(|b| self.binding_types.get(&b.1.start))
                                            .cloned()
                                    } else {
                                        None
                                    }
                                })
                        })
                        .collect();
                    self.function_parameters.insert(name.span.start, inferred);
                    if seed == previous_seed
                        && parameter_ids
                            .iter()
                            .zip(&previous_parameters)
                            .all(|(id, previous)| self.binding_types.get(id) == previous.as_ref())
                    {
                        break;
                    }
                }
                self.statements(&body.stmts);
                let mut returns = self.inferred_returns.pop().unwrap();
                if result_type.is_none() {
                    if BlockExits::inspect(&body.stmts).next {
                        returns.push(Shape::Typed(ValueType::Null));
                    }
                    let inferred = if returns.iter().all(|shape| Some(shape) == returns.first()) {
                        returns.first().cloned().unwrap_or(Shape::Unknown)
                    } else {
                        Shape::Alternatives(returns)
                    };
                    if let Shape::Typed(ty) = &inferred {
                        self.function_results.insert(name.span.start, ty.clone());
                    }
                    self.function_shapes.insert(name.span.start, inferred);
                }
                self.return_types.pop();
                self.scopes.pop();
                self.scope_spans.pop();
            }
            StmtKind::Declaration {
                role: _,
                name,
                constant,
                value,
                ty,
            } => {
                self.expr(value);
                let imported_module = self.expression_module(value);
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
                let known_type = annotation.or_else(|| match self.shape(value) {
                    Shape::Typed(ty) => Some(ty),
                    _ => None,
                });
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
                let suspending = *constant && self.known_suspending(value);
                if *constant && let Some(module) = imported_module {
                    self.binding_modules.insert(name.span.start, module);
                }
                let constructor = if *constant {
                    self.user_constructor(value)
                } else {
                    None
                };
                let binding_shape = if *constant {
                    self.shape(value)
                } else {
                    Shape::Unknown
                };
                self.declare(name, *constant, statement.span.end);
                if suspending {
                    self.suspending_functions.insert(name.span.start);
                }
                if let Some(constructor) = constructor {
                    self.constructor_aliases
                        .insert(name.span.start, constructor);
                }
                if matches!(
                    binding_shape,
                    Shape::Record(_) | Shape::Tuple(_) | Shape::List(_) | Shape::Alternatives(_)
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
                if callable_result != Shape::Unknown {
                    self.function_shapes
                        .insert(name.span.start, callable_result.clone());
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
                if let Some(value) = value {
                    self.expr(value);
                    if self.region_depth > 0
                        && !self.is_promote_call(value)
                        && !self.is_move_call(value)
                    {
                        self.check_escape(
                            value,
                            0,
                            "Returned value may capture cells allocated inside a region; they will be promoted to an outer region",
                        );
                    }
                }
                let returned = value
                    .as_ref()
                    .map_or(Shape::Typed(ValueType::Null), |value| self.shape(value));
                if let Some(returns) = self.inferred_returns.last_mut() {
                    returns.push(returned);
                }

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
            }
            StmtKind::Show(value) | StmtKind::Yield(value) | StmtKind::Expr(value) => {
                self.expr(value)
            }
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
                if matches!(
                    element,
                    Shape::Record(_) | Shape::Tuple(_) | Shape::List(_) | Shape::Alternatives(_)
                ) {
                    self.binding_shapes.insert(binding.span.start, element);
                }
                self.statements(&body.stmts);
                self.scopes.pop();
                self.scope_spans.pop();
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
            StmtKind::Region {
                strict,
                budget,
                body,
                ..
            } => {
                if let Some(budget) = budget {
                    self.expr(budget);
                    if self.calls && self.shape(budget).rejects(&ValueType::Number) {
                        self.error(
                            budget.span,
                            "region-budget",
                            "Region budget must be a number",
                        );
                    }
                }
                self.region_depth += 1;
                self.region_strict.push(*strict);
                self.block(body);
                self.region_strict.pop();
                self.region_depth -= 1;
            }
        }
    }
}
