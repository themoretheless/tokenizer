use super::{
    Binding, Closure, Environment, FunctionBody, RecordMap, Result, Runtime, RuntimeError, Value,
    ValueType, memory,
};
use crate::{Expr, ExprKind, Stmt, StmtKind};
use std::rc::Rc;
use themoretheless_tokenizer_core::Span;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    Next,
    Return,
    Break,
    Continue,
}

impl<'s> Runtime<'_, 's> {
    pub(super) fn match_value(
        &mut self,
        pattern: &Expr<'s>,
        value: &Value<'s>,
        local: &mut Environment<'s>,
    ) -> Result<bool> {
        self.charge(1, pattern.span)?;
        match &pattern.kind {
            ExprKind::Name(name) => {
                if name.text != "_" {
                    local
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(pattern.span, e))?;
                }
                Ok(true)
            }
            ExprKind::Tuple(patterns) => {
                let Value::Tuple(values) = value else {
                    return Ok(false);
                };
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (pattern, value) in patterns.iter().zip(values) {
                    if !self.match_value(pattern, value, local)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            ExprKind::Map(patterns) => {
                let fields = match value {
                    Value::Record(fields) => fields,
                    Value::UserData(data) if data.variant.is_none() => {
                        let Some(Value::Record(fields)) = data.values.first() else {
                            return Ok(false);
                        };
                        fields
                    }
                    _ => return Ok(false),
                };
                for (key, pattern) in patterns {
                    let ExprKind::Name(name) = &key.kind else {
                        return Ok(false);
                    };
                    let Some(value) = fields.get(name.text) else {
                        return Ok(false);
                    };
                    if !self.match_value(pattern, value, local)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            ExprKind::Call { callee, arguments } => {
                let values = match (&callee.kind, value) {
                    (ExprKind::Name(name), Value::Variant(tag, values)) if name.text == *tag => {
                        values
                    }
                    (_, Value::UserData(data)) => {
                        let Value::Function(constructor) = self.expr(callee, local)? else {
                            return Ok(false);
                        };
                        let FunctionBody::Constructor {
                            type_name: expected,
                            variant: tag,
                            ..
                        } = &*constructor.body
                        else {
                            return Ok(false);
                        };
                        if expected != &data.type_name || tag != &data.variant {
                            return Ok(false);
                        }
                        &data.values
                    }
                    _ => return Ok(false),
                };
                if arguments.len() != values.len() {
                    return Ok(false);
                }
                for (pattern, value) in arguments.iter().zip(values) {
                    if !self.match_value(pattern, value, local)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => {
                let literal = self.expr(pattern, local)?;
                self.equal(&literal, value, pattern.span)
            }
        }
    }

    pub(super) fn bind_pattern(
        &self,
        pattern: &Expr<'s>,
        value: &Value<'s>,
        bindings: &mut Environment<'s>,
    ) -> Result<()> {
        self.check(pattern.span)?;
        if let (ExprKind::Map(_), Value::UserData(data)) = (&pattern.kind, value)
            && data.variant.is_none()
            && let Some(record @ Value::Record(_)) = data.values.first()
        {
            return self.bind_pattern(pattern, record, bindings);
        }
        match (&pattern.kind, value) {
            (ExprKind::Name(name), _) => {
                if name.text != "_" {
                    bindings
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(pattern.span, e))?;
                }
                Ok(())
            }
            (ExprKind::Map(entries), Value::Record(fields)) => {
                for (key, pattern) in entries {
                    let ExprKind::Name(name) = &key.kind else {
                        return self.error(key.span, "Record pattern keys must be names");
                    };
                    let Some(value) = fields.get(name.text) else {
                        return self.error(key.span, "Missing record pattern field");
                    };
                    self.bind_pattern(pattern, value, bindings)?;
                }
                Ok(())
            }
            (ExprKind::Tuple(patterns), Value::Tuple(values)) if patterns.len() == values.len() => {
                for (pattern, value) in patterns.iter().zip(values) {
                    self.bind_pattern(pattern, value, bindings)?;
                }
                Ok(())
            }
            _ => self.error(pattern.span, "Value does not match binding pattern"),
        }
    }

    pub(super) fn module_statements(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let result = self.statements(statements, environment)?;
        let exports: Vec<_> = statements
            .iter()
            .filter_map(|statement| {
                if let StmtKind::Export(names) = &statement.kind {
                    Some(names)
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        if exports.is_empty() {
            return Ok(result);
        }
        self.collection_growth(
            0,
            exports.len(),
            statements.first().map_or(Span::new(0, 0), |s| s.span),
        )?;
        let mut values = Vec::with_capacity(exports.len());
        for name in exports {
            self.string_growth(0, name.text.len(), name.span)?;
            let value = self.expr(
                &Expr {
                    span: name.span,
                    kind: ExprKind::Name(name.clone()),
                },
                environment,
            )?;
            values.push((name.text.to_owned(), value));
        }
        Ok((Value::Record(RecordMap::from_iter(values)), Flow::Next))
    }

    pub(super) fn scoped_module(
        &mut self,
        statements: &[Stmt<'s>],
        mut environment: Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let result = self.module_statements(statements, &mut environment);
        drop(environment);
        self.reclaim_cells();
        result
    }

    pub(super) fn scoped_statements(
        &mut self,
        statements: &[Stmt<'s>],
        mut environment: Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let result = self.statements(statements, &mut environment);
        drop(environment);
        self.reclaim_cells();
        result
    }

    pub(super) fn declare_type(
        &mut self,
        statement: &Stmt<'s>,
        environment: &mut Environment<'s>,
    ) -> Result<()> {
        match &statement.kind {
            StmtKind::Struct { name, fields } => {
                let id = format!("{}::{}", self.module.unwrap_or("<entry>"), name.text);
                self.user_types.insert((self.module, name.text), id.clone());
                let fields = fields
                    .iter()
                    .map(|(name, ty)| Ok((name.text.to_owned(), self.annotation(ty)?)))
                    .collect::<Result<Vec<_>>>()?;
                self.struct_fields
                    .insert(id.clone(), fields.iter().cloned().collect());
                let constructor = self.constructor(name, id, None, fields, Vec::new())?;
                environment
                    .insert(name.text, constructor)
                    .map_err(|e| self.environment_error(name.span, e))?;
            }
            StmtKind::Enum { name, variants } => {
                let id = format!("{}::{}", self.module.unwrap_or("<entry>"), name.text);
                self.user_types.insert((self.module, name.text), id.clone());
                let mut constructors = Vec::with_capacity(variants.len());
                self.collection_growth(0, variants.len(), name.span)?;
                for (variant, types) in variants {
                    self.string_growth(0, variant.text.len(), variant.span)?;
                    let types = types
                        .iter()
                        .map(|ty| self.annotation(ty))
                        .collect::<Result<Vec<_>>>()?;
                    constructors.push((
                        variant.text.to_owned(),
                        self.constructor(
                            variant,
                            id.clone(),
                            Some(variant.text.to_owned()),
                            Vec::new(),
                            types,
                        )?,
                    ));
                }
                environment
                    .insert(
                        name.text,
                        Value::Record(RecordMap::from_unique_entries(constructors)),
                    )
                    .map_err(|e| self.environment_error(name.span, e))?;
            }
            _ => unreachable!("type declaration"),
        }
        Ok(())
    }

    pub(super) fn import_module(
        &mut self,
        name: &crate::Name<'s>,
        environment: &mut Environment<'s>,
    ) -> Result<()> {
        if let Ok(index) = self
            .module_cache
            .binary_search_by_key(&name.text, |(key, _)| *key)
        {
            environment
                .insert(name.text, self.module_cache[index].1.clone())
                .map_err(|e| self.environment_error(name.span, e))?;
            return Ok(());
        }
        if self.loading.contains(&name.text) {
            return self.error(name.span, "Cyclic module import");
        }
        let module = self
            .modules
            .get(name.text)
            .cloned()
            .ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span: name.span,
                message: format!("Unknown module: {}", name.text),
            })?;
        if self.depth >= self.max_depth {
            return self.error(name.span, "Execution limit exceeded");
        }
        if let Err(error) = self.loading.reserve(1) {
            return self.error(
                name.span,
                &format!("Runtime module stack allocation failed: {error:?}"),
            );
        }
        // Reserve an export slot for this module and every active
        // ancestor before executing any module body. Nested imports
        // must not consume their callers' reserved capacity.
        if let Err(error) = self.module_cache.reserve(self.loading.len() + 1) {
            return self.error(
                name.span,
                &format!("Runtime module cache allocation failed: {error:?}"),
            );
        }
        let module_environment = self
            .module_globals
            .try_clone()
            .map_err(|e| self.environment_error(name.span, e))?;
        self.loading
            .push(name.text)
            .expect("module stack reserved before execution");
        self.depth += 1;
        let previous_module = self.module.replace(name.text);
        let previous_references = std::mem::replace(
            &mut self.current_references,
            self.references[&Some(name.text)].clone(),
        );
        let result = self.scoped_module(&module.items, module_environment);
        self.module = previous_module;
        self.current_references = previous_references;
        self.depth -= 1;
        self.loading.pop();
        let (exports, _) = result?;
        if !matches!(exports, Value::Record(_)) {
            return self.error(name.span, "Module must evaluate to an export record");
        }
        self.enforce_retained_limit(name.span, Some(&exports))?;
        let index = self
            .module_cache
            .binary_search_by_key(&name.text, |(key, _)| *key)
            .expect_err("an active module cannot already be cached");
        self.module_cache
            .push((name.text, exports.clone()))
            .expect("module export slot reserved before execution");
        self.module_cache[index..].rotate_right(1);
        environment
            .insert(name.text, exports)
            .map_err(|e| self.environment_error(name.span, e))?;
        Ok(())
    }

    pub(super) fn build_function_closure(
        &mut self,
        statement: &Stmt<'s>,
        environment: &Environment<'s>,
        excluded_defs: &[Span],
    ) -> Result<Closure<'s>> {
        let StmtKind::Function {
            name,
            parameters,
            body,
            result,
        } = &statement.kind
        else {
            unreachable!("function declaration");
        };
        let shared_body = self.shared_body(statement.span, || FunctionBody::Block(body.clone()));
        Ok(Closure {
            parameters: parameters.iter().map(|p| p.pattern.clone()).collect(),
            parameter_types: parameters
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    p.ty.as_ref()
                        .map(|ty| self.annotation(ty))
                        .transpose()
                        .map(|ty| {
                            ty.or_else(|| {
                                self.contracts
                                    .get(&self.module)
                                    .and_then(|m| m.get(&name.span.start))
                                    .and_then(|c| c.0.get(i))
                                    .cloned()
                                    .flatten()
                                    .map(|ty| self.runtime_contract(&ty))
                            })
                        })
                })
                .collect::<Result<Vec<_>>>()?,
            result_type: result
                .as_ref()
                .map(|ty| self.annotation(ty))
                .transpose()?
                .or_else(|| {
                    self.contracts
                        .get(&self.module)
                        .and_then(|m| m.get(&name.span.start))
                        .and_then(|c| c.1.as_ref())
                        .map(|ty| self.runtime_contract(ty))
                }),
            name: Some(name.text),
            module: self.module,
            body: shared_body,
            environment: self.capture_excluding(statement.span, environment, excluded_defs)?,
            references: self.current_references.clone(),
            siblings: Rc::new(Vec::new()),
        })
    }

    pub(super) fn scope_has_forward_fn_ref(&self, statements: &[Stmt<'s>]) -> bool {
        let Some(first) = statements.first() else {
            return false;
        };
        let Some(last) = statements.last() else {
            return false;
        };
        let scope_start = first.span.start;
        let scope_end = last.span.end;
        let refs = &self.current_references;
        let start = refs.partition_point(|r| r.usage.start < scope_start);
        statements.iter().any(|stmt| {
            let StmtKind::Function { name, .. } = &stmt.kind else {
                return false;
            };
            refs[start..]
                .iter()
                .take_while(|r| r.usage.start < stmt.span.start && r.usage.start < scope_end)
                .any(|r| r.definition == Some(name.span))
        })
    }

    pub(super) fn hoist_scope_functions(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<()> {
        let fn_stmts: Vec<&Stmt<'s>> = statements
            .iter()
            .filter(|s| matches!(s.kind, StmtKind::Function { .. }))
            .collect();
        if fn_stmts.is_empty() {
            return Ok(());
        }
        if fn_stmts.len() == 1 {
            let stmt = fn_stmts[0];
            let StmtKind::Function { name, .. } = &stmt.kind else {
                unreachable!()
            };
            let closure = self.build_function_closure(stmt, environment, &[name.span])?;
            environment
                .insert(name.text, Value::Function(Rc::new(closure)))
                .map_err(|e| self.environment_error(name.span, e))?;
            return Ok(());
        }
        let excluded_defs: Vec<Span> = fn_stmts
            .iter()
            .filter_map(|s| {
                if let StmtKind::Function { name, .. } = &s.kind {
                    Some(name.span)
                } else {
                    None
                }
            })
            .collect();
        let mut base_closures = Vec::with_capacity(fn_stmts.len());
        for stmt in &fn_stmts {
            base_closures.push(self.build_function_closure(stmt, environment, &excluded_defs)?);
        }
        let siblings = Rc::new(base_closures);
        for (stmt, base) in fn_stmts.iter().zip(siblings.iter()) {
            let StmtKind::Function { name, .. } = &stmt.kind else {
                unreachable!()
            };
            let mut closure = base.clone();
            closure.siblings = siblings.clone();
            environment
                .insert(name.text, Value::Function(Rc::new(closure)))
                .map_err(|e| self.environment_error(name.span, e))?;
        }
        Ok(())
    }

    pub(super) fn hoist_scope_declarations(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
        has_forward_fn: bool,
    ) -> Result<()> {
        let first_type_or_fn = statements.iter().position(|s| {
            matches!(
                s.kind,
                StmtKind::Struct { .. } | StmtKind::Enum { .. } | StmtKind::Function { .. }
            )
        });
        let Some(first_idx) = first_type_or_fn else {
            return Ok(());
        };
        for (idx, statement) in statements.iter().enumerate() {
            if idx > first_idx
                && let StmtKind::Import(name) = &statement.kind
            {
                self.import_module(name, environment)?;
            }
        }
        for statement in statements {
            match &statement.kind {
                StmtKind::Struct { name, .. } | StmtKind::Enum { name, .. } => {
                    let id = format!("{}::{}", self.module.unwrap_or("<entry>"), name.text);
                    self.user_types.insert((self.module, name.text), id);
                }
                _ => {}
            }
        }
        for statement in statements {
            if matches!(
                statement.kind,
                StmtKind::Struct { .. } | StmtKind::Enum { .. }
            ) {
                self.declare_type(statement, environment)?;
            }
        }
        if has_forward_fn {
            self.hoist_scope_functions(statements, environment)?;
        }
        Ok(())
    }

    pub(super) fn statements(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let has_forward_fn = self.scope_has_forward_fn_ref(statements);
        self.hoist_scope_declarations(statements, environment, has_forward_fn)?;
        let last_fn = if has_forward_fn {
            statements
                .iter()
                .rposition(|s| matches!(s.kind, StmtKind::Function { .. }))
        } else {
            None
        };
        let mut value = Value::Null;
        for (idx, statement) in statements.iter().enumerate() {
            self.check(statement.span)?;
            if self.remaining == 0 {
                return self.error(statement.span, "Execution limit exceeded");
            }
            self.remaining -= 1;
            match &statement.kind {
                StmtKind::Struct { .. } | StmtKind::Enum { .. } => {
                    value = Value::Null;
                }
                StmtKind::Export(_) => value = Value::Null,
                StmtKind::Import(name) => {
                    value = Value::Null;
                    self.import_module(name, environment)?;
                    if last_fn.is_some_and(|last| idx < last) {
                        self.hoist_scope_functions(statements, environment)?;
                    }
                }

                StmtKind::Destructure {
                    pattern,
                    value: expression,
                } => {
                    value = self.expr(expression, environment)?;
                    let mut bindings = Environment::new(&self.memory);
                    self.bind_pattern(pattern, &value, &mut bindings)?;
                    environment
                        .extend(bindings)
                        .map_err(|e| self.environment_error(statement.span, e))?;
                    if last_fn.is_some_and(|last| idx < last) {
                        self.hoist_scope_functions(statements, environment)?;
                    }
                }
                StmtKind::Declaration {
                    role: _,
                    name,
                    constant,
                    value: expression,
                    ty,
                } => {
                    let contract = ty.as_ref().map(|ty| self.annotation(ty)).transpose()?;
                    value = self.expr(expression, environment)?;
                    if let Some(ty) = &contract {
                        if !ty.accepts(&value) {
                            return self.error(
                                expression.span,
                                "Value does not match its type annotation",
                            );
                        }
                        value = value.coerce_for_type(ty);
                    }
                    if *constant {
                        environment
                            .insert(name.text, value.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                    } else {
                        let cell = self.allocate_cell(
                            value.clone(),
                            contract.or_else(|| ValueType::inferred(&value)),
                            expression.span,
                        )?;
                        environment
                            .insert_binding(name.text, Binding::Cell(cell))
                            .map_err(|e| self.environment_error(name.span, e))?;
                    }
                    if last_fn.is_some_and(|last| idx < last) {
                        self.hoist_scope_functions(statements, environment)?;
                    }
                }
                StmtKind::Function { name, .. } => {
                    if has_forward_fn {
                        value = match environment.get(name.text) {
                            Some(Binding::Value(v)) => v.clone(),
                            _ => Value::Null,
                        };
                    } else {
                        let closure =
                            self.build_function_closure(statement, environment, &[name.span])?;
                        value = Value::Function(Rc::new(closure));
                        environment
                            .insert(name.text, value.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                    }
                }
                StmtKind::Return(expression) => {
                    return Ok((
                        match expression {
                            Some(expression) => self.expr(expression, environment)?,
                            None => Value::Null,
                        },
                        Flow::Return,
                    ));
                }
                StmtKind::Break => return Ok((Value::Null, Flow::Break)),
                StmtKind::Continue => return Ok((Value::Null, Flow::Continue)),
                StmtKind::While { condition, body } => {
                    // The outer binding map is loop-invariant (bodies only
                    // mutate through shared cells), so snapshot it once and
                    // give every iteration a cheap pooled child scope.
                    let base = memory::Shared::new(
                        &self.memory,
                        environment
                            .try_clone()
                            .map_err(|e| self.environment_error(statement.span, e))?,
                    )
                    .map_err(|e| self.environment_error(statement.span, e))?;
                    loop {
                        let Value::Bool(keep_going) = self.expr(condition, environment)? else {
                            return self.error(condition.span, "Condition must be boolean");
                        };
                        if !keep_going {
                            break;
                        }
                        let mut scope = self.take_call_environment(&base);
                        // Implicit per-iteration arena: scratch cells die in
                        // bulk at the end of the iteration; escaping values
                        // promote like in an explicit `region` block.
                        let iteration = self.enter_region(None, None);
                        let result = self.statements(&body.stmts, &mut scope);
                        self.recycle_call_environment(scope);
                        self.reclaim_cells();
                        self.exit_region(iteration, statement.span)?;
                        let result = result?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = Value::Null;
                }
                StmtKind::For {
                    binding,
                    iterable,
                    body,
                } => {
                    let source = self.expr(iterable, environment)?;
                    let mut cursor = self.sequence_cursor(source, iterable.span)?;
                    let base = memory::Shared::new(
                        &self.memory,
                        environment
                            .try_clone()
                            .map_err(|e| self.environment_error(statement.span, e))?,
                    )
                    .map_err(|e| self.environment_error(statement.span, e))?;
                    while let Some(item) = self.sequence_next(&mut cursor, iterable.span)? {
                        let mut scope = self.take_call_environment(&base);
                        scope
                            .insert(binding.text, item)
                            .map_err(|e| self.environment_error(binding.span, e))?;
                        // Same implicit per-iteration arena as in `while`.
                        let iteration = self.enter_region(None, None);
                        let result = self.statements(&body.stmts, &mut scope);
                        self.recycle_call_environment(scope);
                        self.reclaim_cells();
                        self.exit_region(iteration, statement.span)?;
                        let result = result?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = Value::Null;
                }
                StmtKind::Show(expression) | StmtKind::Expr(expression) => {
                    value = self.expr(expression, environment)?
                }
                StmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    let Value::Bool(condition) = self.expr(condition, environment)? else {
                        return self.error(statement.span, "Condition must be boolean");
                    };
                    let block = if condition {
                        Some(then_block)
                    } else {
                        else_block.as_ref()
                    };
                    if let Some(block) = block {
                        let result = self.scoped_statements(
                            &block.stmts,
                            environment
                                .try_clone()
                                .map_err(|e| self.environment_error(statement.span, e))?,
                        )?;
                        if result.1 != Flow::Next {
                            return Ok(result);
                        }
                        value = result.0;
                    } else {
                        value = Value::Null;
                    }
                }
                StmtKind::Region {
                    name,
                    strict: _,
                    budget,
                    body,
                } => {
                    let budget = self.eval_region_budget(budget.as_ref(), environment)?;
                    let scope = environment
                        .try_clone()
                        .map_err(|e| self.environment_error(statement.span, e))?;
                    let region = self.enter_region(budget, name.as_ref().map(|name| name.text));
                    let result = self.scoped_statements(&body.stmts, scope);
                    self.exit_region(region, statement.span)?;
                    let result = result?;
                    if result.1 != Flow::Next {
                        return Ok(result);
                    }
                    value = result.0;
                }
                _ => return self.error(statement.span, "Statement is not executable yet"),
            }
        }
        Ok((value, Flow::Next))
    }
}
