use super::engine_value::EngineValue as Value;
use super::*;
impl<'s> Runtime<'_, 's> {
    fn buffer<T: memory::ItemDepth>(
        &self,
        items: impl IntoIterator<Item = T>,
        span: Span,
    ) -> Result<memory::Buffer<T>> {
        memory::Buffer::from_iter(&self.memory, items).map_err(|e| self.environment_error(span, e))
    }
    fn shared<T>(&self, value: T, span: Span) -> Result<memory::Shared<T>> {
        memory::Shared::new(&self.memory, value).map_err(|e| self.environment_error(span, e))
    }
    fn slots<T>(&self) -> memory::Slots<T> {
        memory::Slots::new(&self.memory, 0).expect("empty slots")
    }
    fn import(&self, value: &super::Value<'s>, span: Span) -> Result<Value<'s>> {
        Value::import(value, &self.memory).map_err(|e| self.environment_error(span, e))
    }

    pub(super) fn cell_index(&self, cell: &Rc<CellId>, span: Span) -> Result<usize> {
        if cell.released.as_ptr() != Rc::as_ptr(&self.released_cells) {
            return self.error(span, "Mutable capture belongs to another execution");
        }
        if !self
            .cell_ids
            .get(cell.index)
            .is_some_and(|id| id.as_ptr() == Rc::as_ptr(cell))
        {
            return self.error(span, "Mutable capture is no longer available");
        }
        Ok(cell.index)
    }
    pub(super) fn capture(
        &self,
        span: Span,
        environment: &Environment<'s>,
    ) -> Result<memory::Shared<Environment<'s>>> {
        let references = &self.current_references;
        let start = references.partition_point(|reference| reference.usage.start < span.start);
        let mut captured = Environment::new(&self.memory);
        for reference in &references[start..] {
            if reference.usage.start >= span.end {
                break;
            }
            if reference.definition.is_some_and(|definition| {
                definition.start >= span.start && definition.end <= span.end
            }) {
                continue;
            }
            if !captured.contains_key(reference.name)
                && let Some(binding) = environment.get(reference.name)
            {
                captured
                    .insert_binding(reference.name, binding.clone())
                    .map_err(|e| self.environment_error(span, e))?;
            }
        }
        memory::Shared::new(&self.memory, captured).map_err(|e| self.environment_error(span, e))
    }
    pub(super) fn reclaim_cells(&mut self) {
        // Dropping a value can release captured bindings. Drain again without
        // holding a RefCell borrow across that drop.
        loop {
            let released = self.released_cells.borrow_mut().pop();
            let Some(index) = released else {
                break;
            };
            self.cells[index] = (Value::Null, None);
            self.free_cells
                .push(index)
                .expect("free list reserved with cell");
        }
    }
    pub(super) fn allocate_cell(
        &mut self,
        value: Value<'s>,
        contract: Option<memory::Shared<ast_memory::RuntimeType>>,
        span: Span,
    ) -> Result<Rc<CellId>> {
        self.reclaim_cells();
        self.cell_allocations += 1;
        if self.cell_allocations >= self.cell_collection_interval {
            self.collect_cell_cycles(span)?;
            self.cell_allocations = 0;
            self.cell_collection_interval = (self.cells.len() - self.free_cells.len()).max(64);
        }
        let allocation = self.lease(memory::rc_bytes::<CellId>(), span)?;
        let index = if let Some(index) = self.free_cells.pop() {
            self.cells[index] = (value, contract);
            index
        } else {
            let index = self.cells.len();
            self.cells
                .reserve(1)
                .and_then(|()| self.cell_ids.reserve(1))
                .and_then(|()| self.cell_id_storage.reserve(1))
                .and_then(|()| self.free_cells.reserve(index + 1 - self.free_cells.len()))
                .and_then(|()| {
                    let mut released = self.released_cells.borrow_mut();
                    let additional = index + 1 - released.len();
                    released.reserve(additional)
                })
                .map_err(|error| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: format!("Runtime cell allocation failed: {error:?}"),
                })?;
            // Reserve every table, including reclamation queues, before
            // publishing a cell. Dropping a cell ID must never allocate.
            self.cells
                .push((value, contract))
                .expect("reserved cell slot");
            self.cell_ids
                .push(Weak::new())
                .expect("reserved cell ID slot");
            self.cell_id_storage
                .push(None)
                .expect("reserved lease slot");
            index
        };
        let id = Rc::new(CellId {
            index,
            released: Rc::downgrade(&self.released_cells),
            _allocation: allocation.clone(),
            _release_allocation: self._release_allocation.clone(),
        });
        self.cell_ids[index] = Rc::downgrade(&id);
        self.cell_id_storage[index] = Some(allocation);
        Ok(id)
    }
    pub(super) fn host_value_size(
        &mut self,
        value: &Value<'s>,
        span: Span,
        depth: usize,
    ) -> Result<()> {
        if self.max_collection_items == usize::MAX && self.max_string_bytes == usize::MAX {
            return Ok(());
        }
        self.charge(1, span)?;
        if depth > 64 {
            return self.error(span, "Host value nesting limit exceeded");
        }
        match value {
            Value::String(text) => self.string_growth(0, text.len(), span)?,
            Value::List(items) | Value::Tuple(items) => {
                self.collection_growth(0, items.len(), span)?;
                for item in items {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            // Option/Result payloads have fixed arity, not collection length.
            // Their nested data still needs validation.
            Value::Variant(_, items) => {
                for item in items {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
            Value::Record(fields) => {
                self.collection_growth(0, fields.len(), span)?;
                for (key, value) in fields {
                    self.string_growth(0, key.len(), span)?;
                    self.host_value_size(value, span, depth + 1)?;
                }
            }
            Value::Polygon(polygon) => self.collection_growth(0, polygon.points().len(), span)?,
            Value::Mesh(mesh) => {
                self.collection_growth(0, mesh.vertices().len(), span)?;
                self.collection_growth(0, mesh.triangles().len(), span)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn string_growth(&self, current: usize, added: usize, span: Span) -> Result<()> {
        if current
            .checked_add(added)
            .is_none_or(|size| size > self.max_string_bytes)
        {
            return self.error(span, "String byte limit exceeded");
        }
        Ok(())
    }
    pub(super) fn collection_growth(&self, current: usize, added: usize, span: Span) -> Result<()> {
        if current
            .checked_add(added)
            .is_none_or(|size| size > self.max_collection_items)
        {
            return self.error(span, "Collection item limit exceeded");
        }
        Ok(())
    }
    pub(super) fn annotation(
        &self,
        ty: &crate::Type<'_>,
    ) -> Result<memory::Shared<ast_memory::RuntimeType>> {
        let storage = self
            .memory
            .reservation(ast_memory::annotation(ty))
            .map_err(|e| self.environment_error(ty.name.span, e))?;
        let value = ValueType::annotation(ty).map_err(|mut error| {
            error.module = self.module.map(str::to_owned);
            error
        })?;
        self.shared(ast_memory::RuntimeType::new(value, storage), ty.name.span)
    }
    fn lease(&self, bytes: usize, span: Span) -> Result<memory::Shared<memory::Reservation>> {
        let storage = self
            .memory
            .reservation(bytes)
            .map_err(|e| self.environment_error(span, e))?;
        self.shared(storage, span)
    }
    fn sequence(&self, mut value: Sequence<'s>, span: Span) -> Result<Rc<Sequence<'s>>> {
        value._allocation = Some(self.lease(memory::rc_bytes::<Sequence<'s>>(), span)?);
        Ok(Rc::new(value))
    }
    pub(super) fn check(&self, span: Span) -> Result<()> {
        if self.cancellation.is_cancelled() {
            self.error(span, "Execution cancelled")
        } else {
            Ok(())
        }
    }
    pub(super) fn charge(&mut self, amount: usize, span: Span) -> Result<()> {
        self.check(span)?;
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Execution limit exceeded".into(),
            })?;
        Ok(())
    }
    pub(super) fn sequence_cursor(
        &mut self,
        value: Value<'s>,
        span: Span,
    ) -> Result<SequenceCursor<'s>> {
        let sequence = match value {
            Value::Sequence(sequence) => sequence,
            Value::Range { start, end, step } => self.sequence(
                Sequence {
                    source: SequenceSource::Range { start, end, step },
                    stages: SequenceStages::default(),
                    _allocation: None,
                },
                span,
            )?,
            Value::List(items) => self.sequence(
                Sequence {
                    source: SequenceSource::List(items),
                    stages: SequenceStages::default(),
                    _allocation: None,
                },
                span,
            )?,
            _ => return self.error(span, "Expected a list or sequence"),
        };
        Ok(SequenceCursor {
            sequence,
            index: 0,
            previous: None,
            host: None,
            finished: false,
        })
    }

    pub(super) fn sequence_next(
        &mut self,
        cursor: &mut SequenceCursor<'s>,
        span: Span,
    ) -> Result<Option<Value<'s>>> {
        if cursor.finished {
            return Ok(None);
        }
        'candidate: loop {
            self.charge(1, span)?;
            let mut item = match &cursor.sequence.source {
                SequenceSource::Host(source) => {
                    if cursor.host.is_none() {
                        cursor.host =
                            Some(source.factory.open(self.cancellation).map_err(|message| {
                                RuntimeError {
                                    stack: Vec::new(),
                                    location: None,
                                    module: self.module.map(str::to_owned),
                                    span,
                                    message,
                                }
                            })?);
                        self.check(span)?;
                    }
                    let next = cursor
                        .host
                        .as_mut()
                        .unwrap()
                        .next(self.cancellation)
                        .map_err(|message| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message,
                        })?;
                    self.check(span)?;
                    let Some(item) = next else {
                        cursor.finished = true;
                        cursor.host = None;
                        return Ok(None);
                    };
                    let item = self.import(&item, span)?;
                    self.host_value_size(&item, span, 0)?;
                    if !source.item_type.accepts_engine(&item) {
                        return self
                            .error(span, "Host sequence item does not match its declared type");
                    }
                    item
                }
                SequenceSource::List(items) => match items.get(cursor.index) {
                    Some(item) => item.clone(),
                    None => return Ok(None),
                },
                SequenceSource::Range { start, end, step } => {
                    let current = (cursor.index as f64).mul_add(*step, *start);
                    if cursor.previous == Some(current) || !current.is_finite() {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    if if *step > 0.0 {
                        current >= *end
                    } else {
                        current <= *end
                    } {
                        return Ok(None);
                    }
                    cursor.previous = Some(current);
                    Value::Number(current)
                }
            };
            cursor.index = cursor.index.checked_add(1).ok_or_else(|| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message: "Sequence index overflow".into(),
            })?;
            for stage in cursor.sequence.stages.iter() {
                let previous_module = self.module;
                self.module = stage.module;
                let result = self
                    .call(
                        stage.callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        stage.span,
                    )
                    .and_then(|value| {
                        if stage.filter && !matches!(value, Value::Bool(_)) {
                            self.error(stage.span, "Filter callback must return a boolean")
                        } else {
                            Ok(value)
                        }
                    });
                self.module = previous_module;
                let result = result?;
                if stage.filter {
                    if result == Value::Bool(false) {
                        continue 'candidate;
                    }
                } else {
                    item = result;
                }
            }
            return Ok(Some(item));
        }
    }

    pub(super) fn equal(
        &mut self,
        left: &Value<'_>,
        right: &Value<'_>,
        span: Span,
    ) -> Result<bool> {
        let mut pending = self.slots();
        pending
            .push((left, right))
            .map_err(|e| self.environment_error(span, e))?;
        while let Some((left, right)) = pending.pop() {
            self.charge(1, span)?;
            match (left, right) {
                (
                    Value::Sequence(_) | Value::Function(_) | Value::Host(_) | Value::Builtin(_),
                    _,
                )
                | (
                    _,
                    Value::Sequence(_) | Value::Function(_) | Value::Host(_) | Value::Builtin(_),
                ) => {
                    return self.error(span, "Functions and lazy sequences cannot be compared");
                }
                (Value::Variant(a, left), Value::Variant(b, right)) => {
                    if a != b || left.len() != right.len() {
                        return Ok(false);
                    }
                    self.charge(left.len(), span)?;
                    pending
                        .extend(left.iter().zip(right))
                        .map_err(|e| self.environment_error(span, e))?;
                }
                (Value::List(a), Value::List(b)) | (Value::Tuple(a), Value::Tuple(b)) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    self.charge(a.len(), span)?;
                    pending
                        .extend(a.iter().zip(b.iter()))
                        .map_err(|e| self.environment_error(span, e))?;
                }
                (Value::Record(a), Value::Record(b)) => {
                    if a.len() != b.len() {
                        return Ok(false);
                    }
                    self.charge(a.len(), span)?;
                    for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
                        self.charge(ka.len().max(kb.len()), span)?;
                        if ka != kb {
                            return Ok(false);
                        }
                        pending
                            .push((va, vb))
                            .map_err(|e| self.environment_error(span, e))?;
                    }
                }
                (Value::String(a), Value::String(b)) => {
                    self.charge(a.len().max(b.len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                (Value::Mesh(a), Value::Mesh(b)) => {
                    self.charge(a.vertices().len().max(b.vertices().len()), span)?;
                    self.charge(a.triangles().len().max(b.triangles().len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                (Value::Polygon(a), Value::Polygon(b)) => {
                    self.charge(a.points().len().max(b.points().len()), span)?;
                    if a != b {
                        return Ok(false);
                    }
                }
                _ => {
                    if left != right {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
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
                let Value::Record(fields) = value else {
                    return Ok(false);
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
                let ExprKind::Name(name) = &callee.kind else {
                    return Ok(false);
                };
                let Value::Variant(tag, values) = value else {
                    return Ok(false);
                };
                if name.text != *tag || arguments.len() != values.len() {
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
    pub(super) fn statements(
        &mut self,
        statements: &[Stmt<'s>],
        environment: &mut Environment<'s>,
    ) -> Result<(Value<'s>, Flow)> {
        let mut value = Value::Null;
        for statement in statements {
            self.check(statement.span)?;
            if self.remaining == 0 {
                return self.error(statement.span, "Execution limit exceeded");
            }
            self.remaining -= 1;
            match &statement.kind {
                StmtKind::Import(name) => {
                    value = Value::Null;
                    if let Ok(index) = self
                        .module_cache
                        .binary_search_by_key(&name.text, |(key, _)| *key)
                    {
                        environment
                            .insert(name.text, self.module_cache[index].1.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                        continue;
                    }
                    if self.loading.contains(&name.text) {
                        return self.error(name.span, "Cyclic module import");
                    }
                    let module =
                        self.modules
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
                    let result = self.scoped_statements(&module.items, module_environment);
                    self.module = previous_module;
                    self.current_references = previous_references;
                    self.depth -= 1;
                    self.loading.pop();
                    let (exports, _) = result?;
                    if !matches!(exports, Value::Record(_)) {
                        return self.error(name.span, "Module must evaluate to an export record");
                    }
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
                }
                StmtKind::Declaration {
                    name,
                    constant,
                    value: expression,
                    ty,
                } => {
                    let contract = ty.as_ref().map(|ty| self.annotation(ty)).transpose()?;
                    value = self.expr(expression, environment)?;
                    if contract
                        .as_ref()
                        .is_some_and(|ty| !ty.accepts_engine(&value))
                    {
                        return self
                            .error(expression.span, "Value does not match its type annotation");
                    }
                    if *constant {
                        environment
                            .insert(name.text, value.clone())
                            .map_err(|e| self.environment_error(name.span, e))?;
                    } else {
                        let cell = self.allocate_cell(value.clone(), contract, expression.span)?;
                        environment
                            .insert_binding(name.text, Binding::Cell(cell))
                            .map_err(|e| self.environment_error(name.span, e))?;
                    }
                }
                StmtKind::Function {
                    name,
                    parameters,
                    body,
                    result,
                } => {
                    let syntax_bytes = parameters.len() * std::mem::size_of::<Expr<'s>>()
                        + parameters
                            .iter()
                            .map(|p| ast_memory::expression(&p.pattern))
                            .sum::<usize>()
                        + ast_memory::block(body);
                    let allocation = self.lease(
                        syntax_bytes + memory::rc_bytes::<Closure<'s>>(),
                        statement.span,
                    )?;
                    let mut parameter_types = self.slots();
                    for p in parameters {
                        parameter_types
                            .push(p.ty.as_ref().map(|ty| self.annotation(ty)).transpose()?)
                            .map_err(|e| self.environment_error(statement.span, e))?;
                    }
                    let parameter_types = memory::Buffer::from_slots(&self.memory, parameter_types)
                        .map_err(|e| self.environment_error(statement.span, e))?;
                    value = Value::Function(Rc::new(Closure {
                        _allocation: allocation,
                        parameters: parameters.iter().map(|p| p.pattern.clone()).collect(),
                        parameter_types,
                        result_type: result.as_ref().map(|ty| self.annotation(ty)).transpose()?,
                        name: Some(name.text),
                        module: self.module,
                        body: FunctionBody::Block(body.clone()),
                        environment: self.capture(statement.span, environment)?,
                        references: self.current_references.clone(),
                    }));
                    environment
                        .insert(name.text, value.clone())
                        .map_err(|e| self.environment_error(name.span, e))?;
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
                    loop {
                        let Value::Bool(keep_going) = self.expr(condition, environment)? else {
                            return self.error(condition.span, "Condition must be boolean");
                        };
                        if !keep_going {
                            break;
                        }
                        let result = self.scoped_statements(
                            &body.stmts,
                            environment
                                .try_clone()
                                .map_err(|e| self.environment_error(statement.span, e))?,
                        )?;
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
                    while let Some(item) = self.sequence_next(&mut cursor, iterable.span)? {
                        let mut local = environment
                            .try_clone()
                            .map_err(|e| self.environment_error(statement.span, e))?;
                        local
                            .insert(binding.text, item)
                            .map_err(|e| self.environment_error(binding.span, e))?;
                        let result = self.scoped_statements(&body.stmts, local)?;
                        match result.1 {
                            Flow::Return => return Ok(result),
                            Flow::Break => break,
                            _ => {}
                        }
                    }
                    value = Value::Null;
                }
                StmtKind::Expr(expression) => value = self.expr(expression, environment)?,
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
                _ => return self.error(statement.span, "Statement is not executable yet"),
            }
        }
        Ok((value, Flow::Next))
    }
    pub(super) fn environment_error(
        &self,
        span: Span,
        error: memory::AllocationError,
    ) -> RuntimeError {
        RuntimeError {
            stack: Vec::new(),
            location: None,
            module: self.module.map(str::to_owned),
            span,
            message: match error {
                memory::AllocationError::Limit => {
                    "Instance memory limit exceeded (allocation failed: Limit)".into()
                }
                memory::AllocationError::Depth => "Host value nesting limit exceeded".into(),
                _ => format!("Runtime environment allocation failed: {error:?}"),
            },
        }
    }
    pub(super) fn error<T>(&self, span: Span, message: &str) -> Result<T> {
        Err(RuntimeError {
            stack: Vec::new(),
            location: None,
            module: self.module.map(str::to_owned),
            span,
            message: message.into(),
        })
    }
    pub(super) fn expr(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        self.check(expression.span)?;
        if self.remaining == 0 || self.depth >= self.max_depth {
            return self.error(expression.span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        self.depth += 1;
        let result = self.inner(expression, environment);
        self.depth -= 1;
        result
    }
    // Keep recursive dispatch small even in unoptimized builds. Each arm's
    // temporaries belong to its own frame, preserving the depth-64 stack ceiling.
    pub(super) fn inner(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        match &expression.kind {
            ExprKind::String(_) => self.inner_string(expression, environment),
            ExprKind::Map(_) => self.inner_record(expression, environment),
            ExprKind::Number(_) => self.inner_number(expression, environment),
            ExprKind::Bool(_) => self.inner_bool(expression, environment),
            ExprKind::Null => self.inner_null(expression, environment),
            ExprKind::Name(_) => self.inner_name(expression, environment),
            ExprKind::Assign { .. } => self.inner_assign(expression, environment),
            ExprKind::Lambda { .. } => self.inner_lambda(expression, environment),
            ExprKind::Tuple(_) | ExprKind::List(_) => {
                self.inner_collection(expression, environment)
            }
            ExprKind::If { .. } => self.inner_if(expression, environment),
            ExprKind::Member { .. } => self.inner_member(expression, environment),
            ExprKind::Index { .. } => self.inner_index(expression, environment),
            ExprKind::Match { .. } => self.inner_match(expression, environment),
            ExprKind::Pipeline { .. } => self.inner_pipeline(expression, environment),
            ExprKind::Call { .. } => self.inner_call(expression, environment),
            ExprKind::Unary { .. } => self.inner_unary(expression, environment),
            ExprKind::Binary { .. } => self.inner_binary(expression, environment),
            _ => self.error(expression.span, "Expression is not executable yet"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_string(
        &mut self,
        expression: &Expr<'s>,
        _environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::String(text) => {
                self.charge(text.len().saturating_sub(2), span)?;
                let mut decoded = self.slots();
                for character in crate::string_literal::characters(text) {
                    let character = character.map_err(|message| RuntimeError {
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                        stack: Vec::new(),
                        location: None,
                    })?;
                    self.string_growth(decoded.len(), character.len_utf8(), span)?;
                    let mut bytes = [0; 4];
                    decoded
                        .extend(character.encode_utf8(&mut bytes).bytes())
                        .map_err(|e| self.environment_error(span, e))?;
                }
                Ok(Value::String(
                    memory::Text::from_bytes(&self.memory, decoded)
                        .map_err(|e| self.environment_error(span, e))?,
                ))
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_record(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Map(entries) => {
                self.collection_growth(0, entries.len(), span)?;
                let mut fields = self.slots::<(memory::Text, Value)>();
                for (key, expression) in entries {
                    let key = match &key.kind {
                        ExprKind::Name(name) => {
                            self.string_growth(0, name.text.len(), key.span)?;
                            memory::Text::from_str(&self.memory, name.text)
                                .map_err(|e| self.environment_error(span, e))?
                        }
                        _ => match self.expr(key, environment)? {
                            Value::String(key) => key,
                            _ => {
                                return self.error(key.span, "Record key must be a name or string");
                            }
                        },
                    };
                    self.string_growth(0, key.len(), span)?;
                    if fields.iter().any(|(k, _)| k == &key) {
                        return self.error(span, "Duplicate record field");
                    }
                    fields
                        .push((key, self.expr(expression, environment)?))
                        .map_err(|e| self.environment_error(span, e))?;
                }
                Ok(Value::Record(
                    memory::Record::from_slots(&self.memory, fields)
                        .map_err(|e| self.environment_error(span, e))?,
                ))
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_number(
        &mut self,
        expression: &Expr<'s>,
        _environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Number(text) => {
                let cleaned = if text.contains('_') {
                    Some(
                        memory::Text::from_chars(&self.memory, text.chars().filter(|c| *c != '_'))
                            .map_err(|e| self.environment_error(span, e))?,
                    )
                } else {
                    None
                };
                cleaned
                    .as_ref()
                    .map_or(*text, |v| v.as_str())
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map(Value::Number)
                    .ok_or_else(|| RuntimeError {
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Unsupported or non-finite number".into(),
                        stack: Vec::new(),
                        location: None,
                    })
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_bool(
        &mut self,
        expression: &Expr<'s>,
        _environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let _span = expression.span;
        match &expression.kind {
            ExprKind::Bool(value) => Ok(Value::Bool(*value)),
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_null(
        &mut self,
        expression: &Expr<'s>,
        _environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let _span = expression.span;
        match &expression.kind {
            ExprKind::Null => Ok(Value::Null),
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_name(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Name(name) => match environment.get(name.text) {
                Some(Binding::Value(value)) => Ok(value.clone()),
                Some(Binding::Cell(cell)) => Ok(self.cells[self.cell_index(cell, span)?].0.clone()),
                None => self.error(span, &format!("Unknown name: {}", name.text)),
            },
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_assign(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                let ExprKind::Name(name) = &target.kind else {
                    return self.error(span, "Only variable assignment is supported");
                };
                let Some(Binding::Cell(cell)) = environment.get(name.text) else {
                    return self.error(span, "Assignment requires a mutable variable");
                };
                let index = self.cell_index(cell, target.span)?;
                let assigned = if *operator == "=" {
                    self.expr(value, environment)?
                } else {
                    let operator = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        _ => return self.error(span, "Unknown assignment operator"),
                    };
                    let _syntax = self
                        .memory
                        .reservation(
                            2 * std::mem::size_of::<Expr<'s>>()
                                + ast_memory::expression(target)
                                + ast_memory::expression(value),
                        )
                        .map_err(|e| self.environment_error(span, e))?;
                    self.expr(
                        &Expr {
                            span,
                            kind: ExprKind::Binary {
                                operator,
                                left: target.clone(),
                                right: value.clone(),
                            },
                        },
                        environment,
                    )?
                };
                if self.cells[index]
                    .1
                    .as_ref()
                    .is_some_and(|ty| !ty.accepts_engine(&assigned))
                {
                    return self.error(span, "Assignment violates variable type");
                }
                self.cells[index].0 = assigned.clone();
                Ok(assigned)
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_lambda(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Lambda { parameters, body } => {
                let allocation = self.lease(
                    ast_memory::expressions(parameters)
                        + ast_memory::expression(body)
                        + memory::rc_bytes::<Closure<'s>>(),
                    span,
                )?;
                Ok(Value::Function(Rc::new(Closure {
                    _allocation: allocation,
                    parameters: parameters.clone(),
                    parameter_types: self
                        .buffer(std::iter::repeat_n(None, parameters.len()), span)?,
                    result_type: None,
                    body: FunctionBody::Expression((**body).clone()),
                    name: None,
                    module: self.module,
                    environment: self.capture(expression.span, environment)?,
                    references: self.current_references.clone(),
                })))
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_collection(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Tuple(items) | ExprKind::List(items) => {
                self.collection_growth(0, items.len(), span)?;
                let mut values = self.slots();
                for item in items {
                    values
                        .push(self.expr(item, environment)?)
                        .map_err(|e| self.environment_error(span, e))?;
                }
                let values = memory::Buffer::from_slots(&self.memory, values)
                    .map_err(|e| self.environment_error(span, e))?;
                Ok(if matches!(expression.kind, ExprKind::Tuple(_)) {
                    Value::Tuple(values)
                } else {
                    Value::List(values)
                })
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_if(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                let Value::Bool(condition) = self.expr(condition, environment)? else {
                    return self.error(span, "Condition must be boolean");
                };
                self.expr(if condition { then_value } else { else_value }, environment)
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_member(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Member { object, field } => {
                let object = self.expr(object, environment)?;
                if let Value::Mesh(mesh) = &object {
                    return match field.text {
                        "vertices" => {
                            self.collection_growth(0, mesh.vertices().len(), span)?;
                            self.charge(mesh.vertices().len(), span)?;
                            let mut values = self.slots();
                            for v in mesh.vertices() {
                                values
                                    .push(self.vector(v.iter().copied(), span)?)
                                    .map_err(|e| self.environment_error(span, e))?;
                            }
                            Ok(Value::List(
                                memory::Buffer::from_slots(&self.memory, values)
                                    .map_err(|e| self.environment_error(span, e))?,
                            ))
                        }
                        "triangles" => {
                            self.collection_growth(0, mesh.triangles().len(), span)?;
                            self.collection_growth(0, 3, span)?;
                            let mut values = self.slots();
                            for t in mesh.triangles() {
                                values
                                    .push(Value::List(self.buffer(
                                        t.iter().map(|i| Value::Number(*i as f64)),
                                        span,
                                    )?))
                                    .map_err(|e| self.environment_error(span, e))?;
                            }
                            Ok(Value::List(
                                memory::Buffer::from_slots(&self.memory, values)
                                    .map_err(|e| self.environment_error(span, e))?,
                            ))
                        }
                        _ => self.error(field.span, "Unknown mesh field"),
                    };
                }
                if let Value::Record(fields) = object {
                    return fields.get(field.text).cloned().ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span: field.span,
                        message: format!("Unknown field: {}", field.text),
                    });
                }
                if !matches!(object, Value::Vector(_)) {
                    return self.error(field.span, "Value does not support member access");
                }
                let axis = match field.text {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return self.error(field.span, "Unknown vector component"),
                };
                match object {
                    Value::Vector(values) => values
                        .get(axis)
                        .copied()
                        .map(Value::Number)
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span: field.span,
                            message: "Vector component out of bounds".into(),
                        }),
                    _ => self.error(span, "Member access requires a vector"),
                }
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_index(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Index { object, index } => {
                let object = self.expr(object, environment)?;
                let index = self.expr(index, environment)?;
                if let Value::Record(fields) = &object {
                    let Value::String(key) = index else {
                        return self.error(span, "Record index must be a string");
                    };
                    return fields.get(&key).cloned().ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Unknown field: {key}"),
                    });
                }
                let Value::Number(index) = index else {
                    return self.error(span, "Index must be an integer");
                };
                if index < 0.0 || index.fract() != 0.0 || index >= usize::MAX as f64 {
                    return self.error(span, "Invalid collection index");
                }
                let result = match object {
                    Value::List(values) | Value::Tuple(values) => {
                        values.get(index as usize).cloned()
                    }
                    Value::Vector(values) => values.get(index as usize).copied().map(Value::Number),
                    _ => return self.error(span, "Indexing requires a list or vector"),
                };
                result.ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "Index out of bounds".into(),
                })
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_match(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Match { value, arms } => {
                let value = self.expr(value, environment)?;
                for arm in arms {
                    let mut local = environment
                        .try_clone()
                        .map_err(|e| self.environment_error(span, e))?;
                    let matched = self.match_value(&arm.pattern, &value, &mut local)?;
                    if matched {
                        if let Some(guard) = &arm.guard {
                            match self.expr(guard, &local)? {
                                Value::Bool(true) => {}
                                Value::Bool(false) => continue,
                                _ => return self.error(guard.span, "Match guard must be boolean"),
                            }
                        }
                        return self.expr(&arm.value, &local);
                    }
                }
                self.error(span, "No matching pattern")
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_pipeline(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Pipeline { input, stages } => {
                let mut value = self.expr(input, environment)?;
                for stage in stages {
                    let (callee, supplied) = match &stage.kind {
                        ExprKind::Call { callee, arguments } => {
                            (callee.as_ref(), arguments.as_slice())
                        }
                        _ => (stage, &[][..]),
                    };
                    let function = self.expr(callee, environment)?;
                    let mut arguments = self.slots();
                    arguments
                        .push(value)
                        .map_err(|e| self.environment_error(span, e))?;
                    for argument in supplied {
                        arguments
                            .push(self.expr(argument, environment)?)
                            .map_err(|e| self.environment_error(span, e))?;
                    }
                    value = self.call(function, Cow::Borrowed(&arguments), stage.span)?;
                }
                Ok(value)
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_call(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Call { callee, arguments } => {
                let function = self.expr(callee, environment)?;
                let mut args = self.slots();
                for argument in arguments {
                    args.push(self.expr(argument, environment)?)
                        .map_err(|e| self.environment_error(span, e))?;
                }
                let arguments = args;
                self.call(function, Cow::Borrowed(&arguments), span)
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_unary(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Unary { operator, value } => {
                match (*operator, self.expr(value, environment)?) {
                    ("-", Value::Number(n)) => Ok(Value::Number(-n)),
                    ("+", Value::Number(n)) => Ok(Value::Number(n)),
                    ("-", Value::Vector(values)) => self.vector(values.iter().map(|v| -v), span),
                    ("+", Value::Vector(values)) => Ok(Value::Vector(values)),
                    ("!" | "not", Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => self.error(span, "Invalid unary operand"),
                }
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    #[cfg_attr(debug_assertions, inline(never))]
    fn inner_binary(
        &mut self,
        expression: &Expr<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let left = self.expr(left, environment)?;
                if matches!(*operator, "and" | "&&" | "or" | "||") {
                    let Value::Bool(left) = left else {
                        return self.error(span, "Boolean operand required");
                    };
                    if left == matches!(*operator, "or" | "||") {
                        return Ok(Value::Bool(left));
                    }
                    let right = self.expr(right, environment)?;
                    return if matches!(right, Value::Bool(_)) {
                        Ok(right)
                    } else {
                        self.error(span, "Boolean operand required")
                    };
                }
                let right = self.expr(right, environment)?;
                if matches!(*operator, "==" | "!=") {
                    let equal = self.equal(&left, &right, span)?;
                    return Ok(Value::Bool(if *operator == "==" { equal } else { !equal }));
                }
                if let (Value::String(a), Value::String(b), "+") = (&left, &right, *operator) {
                    self.string_growth(a.len(), b.len(), span)?;
                    self.charge(a.len(), span)?;
                    self.charge(b.len(), span)?;
                    return Ok(Value::String(
                        a.concat(&self.memory, b)
                            .map_err(|e| self.environment_error(span, e))?,
                    ));
                }
                if matches!(left, Value::Angle(_)) || matches!(right, Value::Angle(_)) {
                    let radians = match (&left, &right, *operator) {
                        (Value::Angle(a), Value::Angle(b), "+") => a + b,
                        (Value::Angle(a), Value::Angle(b), "-") => a - b,
                        (Value::Angle(a), Value::Number(b), "*")
                        | (Value::Number(b), Value::Angle(a), "*") => a * b,
                        (Value::Angle(a), Value::Number(b), "/") => a / b,
                        _ => return self.error(span, "Invalid angle operands"),
                    };
                    if !radians.is_finite() {
                        return self.error(span, "Non-finite angle");
                    }
                    return Ok(Value::Angle(radians));
                }
                if let (Value::Quaternion(a), Value::Quaternion(b), "*") =
                    (&left, &right, *operator)
                {
                    return Ok(Value::Quaternion(self.shared(a.compose(b), span)?));
                }
                if let (Value::Matrix(a), Value::Matrix(b), "*") = (&left, &right, *operator) {
                    return a
                        .multiply(b)
                        .map(|matrix| self.shared(matrix, span).map(Value::Matrix))
                        .ok_or_else(|| RuntimeError {
                            stack: Vec::new(),
                            location: None,
                            module: self.module.map(str::to_owned),
                            span,
                            message: "Matrix multiplication overflow".into(),
                        })?;
                }
                if matches!(left, Value::Vector(_)) || matches!(right, Value::Vector(_)) {
                    return match (&left, &right, *operator) {
                        (Value::Vector(a), Value::Vector(b), "+" | "-") if a.len() == b.len() => {
                            self.vector(
                                a.iter()
                                    .zip(b.iter())
                                    .map(|(a, b)| if *operator == "+" { a + b } else { a - b }),
                                span,
                            )
                        }
                        (Value::Vector(a), Value::Number(b), "*" | "/") => self.vector(
                            a.iter()
                                .map(|a| if *operator == "*" { a * b } else { a / b }),
                            span,
                        ),
                        (Value::Number(a), Value::Vector(b), "*") => {
                            self.vector(b.iter().map(|b| a * b), span)
                        }
                        _ => self.error(span, "Invalid vector arithmetic"),
                    };
                }
                let (Value::Number(a), Value::Number(b)) = (left, right) else {
                    return self.error(span, "Numeric operands required");
                };
                let number = match *operator {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    "%" => a % b,
                    "**" => a.powf(b),
                    "==" => return Ok(Value::Bool(a == b)),
                    "!=" => return Ok(Value::Bool(a != b)),
                    "<" => return Ok(Value::Bool(a < b)),
                    ">" => return Ok(Value::Bool(a > b)),
                    "<=" => return Ok(Value::Bool(a <= b)),
                    ">=" => return Ok(Value::Bool(a >= b)),
                    _ => return self.error(span, "Unsupported binary operator"),
                };
                if number.is_finite() {
                    Ok(Value::Number(number))
                } else {
                    self.error(span, "Non-finite arithmetic result")
                }
            }
            _ => unreachable!("expression dispatch"),
        }
    }
    pub(super) fn vector(
        &self,
        values: impl IntoIterator<Item = f64>,
        span: Span,
    ) -> Result<Value<'s>> {
        let values = self.buffer(values, span)?;
        if values.iter().all(|x| x.is_finite()) {
            Ok(Value::Vector(values))
        } else {
            self.error(span, "Non-finite vector result")
        }
    }
    pub(super) fn math(
        &self,
        builtin: Builtin,
        arguments: &[Value<'s>],
        span: Span,
    ) -> Result<Value<'s>> {
        if matches!(builtin, Builtin::Vec2 | Builtin::Vec3 | Builtin::Vec4) {
            let dimension = match builtin {
                Builtin::Vec2 => 2,
                Builtin::Vec3 => 3,
                _ => 4,
            };
            if arguments.len() != dimension {
                return self.error(span, "Incorrect vector dimension");
            }
            let mut values = [0.; 4];
            for (i, argument) in arguments.iter().enumerate() {
                let Value::Number(v) = argument else {
                    return self.error(span, "Vector components must be numbers");
                };
                values[i] = *v;
            }
            return self.vector(values[..dimension].iter().copied(), span);
        }
        match (builtin, arguments) {
            (Builtin::Slerp, [Value::Quaternion(a), Value::Quaternion(b), Value::Number(t)]) => {
                return a
                    .slerp(b, *t)
                    .map(|q| self.shared(q, span).map(Value::Quaternion))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "slerp requires a finite fraction between 0 and 1".into(),
                    })?;
            }
            (
                Builtin::AxisAngle,
                [
                    Value::Vector(axis),
                    Value::Number(angle) | Value::Angle(angle),
                ],
            ) if axis.len() == 3 => {
                return crate::Quaternion::axis_angle([axis[0], axis[1], axis[2]], *angle)
                    .map(|q| self.shared(q, span).map(Value::Quaternion))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Rotation requires a finite nonzero axis and angle".into(),
                    })?;
            }
            (Builtin::RotationMatrix, [Value::Quaternion(q)]) => {
                return Ok(Value::Matrix(self.shared(q.matrix(), span)?));
            }
            (Builtin::Identity, []) => {
                return Ok(Value::Matrix(
                    self.shared(crate::Matrix4::identity(), span)?,
                ));
            }
            (Builtin::Translation | Builtin::Scaling, [Value::Vector(v)]) if v.len() == 3 => {
                let mut matrix = crate::Matrix4::identity();
                for (axis, value) in v.iter().enumerate() {
                    if builtin == Builtin::Translation {
                        matrix.0[axis][3] = *value;
                    } else {
                        matrix.0[axis][axis] = *value;
                    }
                }
                return Ok(Value::Matrix(self.shared(matrix, span)?));
            }
            (
                Builtin::RotationX | Builtin::RotationY | Builtin::RotationZ,
                [Value::Number(angle) | Value::Angle(angle)],
            ) => {
                let mut matrix = crate::Matrix4::identity();
                let (sin, cos) = angle.sin_cos();
                let (a, b) = match builtin {
                    Builtin::RotationX => (1, 2),
                    Builtin::RotationY => (2, 0),
                    _ => (0, 1),
                };
                matrix.0[a][a] = cos;
                matrix.0[a][b] = -sin;
                matrix.0[b][a] = sin;
                matrix.0[b][b] = cos;
                return Ok(Value::Matrix(self.shared(matrix, span)?));
            }
            (
                Builtin::TransformPoint | Builtin::TransformDirection,
                [Value::Matrix(matrix), Value::Vector(v)],
            ) => {
                return matrix
                    .apply_array(v, builtin == Builtin::TransformPoint)
                    .map(|v| self.vector(v, span))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Invalid matrix transformation".into(),
                    })?;
            }
            _ => {}
        }
        if let (Builtin::Degrees | Builtin::Radians, [Value::Number(value)]) = (builtin, arguments)
        {
            let radians = if builtin == Builtin::Degrees {
                value.to_radians()
            } else {
                *value
            };
            if !radians.is_finite() {
                return self.error(span, "Non-finite angle");
            }
            return Ok(Value::Angle(radians));
        }
        match (builtin, arguments) {
            (Builtin::Polygon, [Value::List(points)]) => {
                self.collection_growth(0, points.len(), span)?;
                let mut data = self.slots();
                for point in points {
                    let Value::Vector(v) = point else {
                        return self.error(span, "Polygon points must be vec2 values");
                    };
                    if v.len() != 2 {
                        return self.error(span, "Polygon points must be vec2 values");
                    }
                    data.push([v[0], v[1]])
                        .map_err(|e| self.environment_error(span, e))?;
                }
                return super::engine_value::EnginePolygon::new(&self.memory, data)
                    .map(Value::Polygon)
                    .map_err(|e| RuntimeError {
                        module: self.module.map(str::to_owned),
                        span,
                        message: e.into(),
                        stack: Vec::new(),
                        location: None,
                    });
            }
            (Builtin::Translate, [Value::Polygon(p), Value::Vector(v)]) if v.len() == 2 => {
                self.collection_growth(0, p.points().len(), span)?;
                return p
                    .translated(&self.memory, [v[0], v[1]])
                    .map(Value::Polygon)
                    .map_err(|e| RuntimeError {
                        module: self.module.map(str::to_owned),
                        span,
                        message: e.into(),
                        stack: Vec::new(),
                        location: None,
                    });
            }
            (Builtin::Rotate, [Value::Polygon(p), Value::Number(a) | Value::Angle(a)]) => {
                self.collection_growth(0, p.points().len(), span)?;
                return p
                    .rotated(&self.memory, *a)
                    .map(Value::Polygon)
                    .map_err(|e| RuntimeError {
                        module: self.module.map(str::to_owned),
                        span,
                        message: e.into(),
                        stack: Vec::new(),
                        location: None,
                    });
            }
            _ => {}
        }
        let number = match (builtin, arguments) {
            (Builtin::Random, [Value::Number(seed), Value::Number(index)]) => {
                crate::noise::random(*seed, *index).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "random requires nonnegative exact integer seed and index".into(),
                })?
            }
            (Builtin::Noise, [Value::Number(x), Value::Number(seed)]) => {
                crate::noise::noise(*x, *seed).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message:
                        "noise requires a bounded coordinate and nonnegative exact integer seed"
                            .into(),
                })?
            }

            (Builtin::Cross, [Value::Vector(a), Value::Vector(b)])
                if a.len() == 3 && b.len() == 3 =>
            {
                return self.vector(
                    [
                        a[1] * b[2] - a[2] * b[1],
                        a[2] * b[0] - a[0] * b[2],
                        a[0] * b[1] - a[1] * b[0],
                    ],
                    span,
                );
            }
            (Builtin::Lerp, [Value::Vector(a), Value::Vector(b), Value::Number(t)])
                if a.len() == b.len() =>
            {
                return self.vector(
                    a.iter().zip(b.iter()).map(|(a, b)| a * (1.0 - t) + b * t),
                    span,
                );
            }
            (Builtin::Lerp, [Value::Number(a), Value::Number(b), Value::Number(t)]) => {
                a * (1.0 - t) + b * t
            }
            (Builtin::Clamp, [Value::Number(x), Value::Number(low), Value::Number(high)])
                if low <= high =>
            {
                x.clamp(*low, *high)
            }
            (Builtin::Smoothstep, [Value::Number(low), Value::Number(high), Value::Number(x)])
                if low < high =>
            {
                let t = if x <= low {
                    0.0
                } else if x >= high {
                    1.0
                } else {
                    let width = high - low;
                    if width.is_finite() {
                        (x - low) / width
                    } else {
                        // Halving first keeps opposite finite extremes representable.
                        (x * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)
                    }
                };
                t * t * (3.0 - 2.0 * t)
            }

            (Builtin::Dot, [Value::Vector(a), Value::Vector(b)]) if a.len() == b.len() => {
                a.iter().zip(b.iter()).map(|(a, b)| a * b).sum()
            }
            (Builtin::Length | Builtin::Normalize, [Value::Vector(v)]) => {
                if builtin == Builtin::Normalize {
                    let scale = v.iter().fold(0.0_f64, |scale, x| scale.max(x.abs()));
                    if scale == 0.0 || v.iter().any(|x| !x.is_finite()) {
                        return self.error(span, "Cannot normalize this vector");
                    }
                    let length = v.iter().fold(0.0_f64, |length, x| length.hypot(x / scale));
                    return self.vector(v.iter().map(|x| (x / scale) / length), span);
                }
                v.iter().fold(0.0_f64, |length, x| length.hypot(*x))
            }
            (Builtin::Sin, [Value::Number(x) | Value::Angle(x)]) => x.sin(),
            (Builtin::Cos, [Value::Number(x) | Value::Angle(x)]) => x.cos(),
            (Builtin::Sqrt, [Value::Number(x)]) => x.sqrt(),
            (Builtin::Deg, [Value::Number(x)]) => x.to_radians(),
            _ => return self.error(span, "Invalid mathematical arguments"),
        };
        if number.is_finite() {
            Ok(Value::Number(number))
        } else {
            self.error(span, "Non-finite mathematical result")
        }
    }
    pub(super) fn call(
        &mut self,
        function: Value<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        self.check(span)?;
        if self.remaining == 0 {
            return self.error(span, "Execution limit exceeded");
        }
        self.remaining -= 1;
        let name = match &function {
            Value::Function(f) => f.name.unwrap_or("<lambda>"),
            Value::Host(f) => f.name,
            Value::Builtin(_) => "<builtin>",
            _ => "<non-callable>",
        };
        let builtin = if let Value::Builtin(b) = &function {
            Some(*b)
        } else {
            None
        };
        let module = self.module;
        let source = self.sources.get(&self.module).copied().unwrap_or("");
        let result = match function {
            Value::Function(function) => self.call_user(function, arguments, span),
            other => self.call_inner(other, arguments, span),
        };
        result.map_err(|mut error| {
            let prefix = &source[..span.start.min(source.len())];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
            let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;

            if error.location.is_none()
                && let Some((_, source)) = self
                    .sources
                    .iter()
                    .find(|(module, _)| *module == error.module.as_deref())
            {
                error = error.locate(source);
            }
            error.stack.push(CallFrame {
                function: builtin.map_or_else(|| name.to_owned(), |b| format!("{b:?}")),
                module: module.map(str::to_owned),
                span,
                line,
                column,
            });
            error
        })
    }
    pub(super) fn call_inner(
        &mut self,
        function: Value<'s>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        if let Value::Host(function) = function {
            if arguments.len() != function.parameters.len()
                || !function
                    .parameters
                    .iter()
                    .zip(arguments.iter())
                    .all(|(ty, value)| ty.accepts_engine(value))
            {
                return self.error(span, "Host function arguments do not match its signature");
            }
            let transfer_bytes = arguments
                .iter()
                .fold(
                    arguments
                        .len()
                        .checked_mul(std::mem::size_of::<super::Value<'s>>()),
                    |n, v| {
                        let n = n?;
                        n.checked_add(
                            v.export_bytes(self.memory.available_bytes().checked_sub(n)?)?,
                        )
                    },
                )
                .ok_or_else(|| self.environment_error(span, memory::AllocationError::Capacity))?;
            let _transfer = self
                .memory
                .reservation(transfer_bytes)
                .map_err(|e| self.environment_error(span, e))?;
            let mut host_arguments = Vec::with_capacity(arguments.len());
            for value in arguments.iter() {
                host_arguments.push(value.export());
            }
            let result = match self.contextual.get(&Rc::as_ptr(&function)) {
                Some(callback) => callback(&host_arguments, self.cancellation),
                None => (function.callback)(&host_arguments, self.cancellation),
            }
            .map_err(|message| RuntimeError {
                stack: Vec::new(),
                location: None,
                module: self.module.map(str::to_owned),
                span,
                message,
            })?;
            self.check(span)?;
            let result = self.import(&result, span)?;
            self.host_value_size(&result, span, 0)?;
            if !function.result.accepts_engine(&result) {
                return self.error(span, "Host function returned an invalid value");
            }
            return Ok(result);
        }
        if let Value::Builtin(builtin) = function {
            for &(index, expected) in builtin.callback_arities() {
                let valid = match arguments.get(index) {
                    Some(Value::Function(function)) => Some(function.parameters.len() == expected),
                    Some(Value::Host(function)) => Some(function.parameters.len() == expected),
                    Some(Value::Builtin(function)) => Some(function.arity().contains(&expected)),
                    _ => None,
                };
                if valid == Some(false) {
                    return self.error(span, "Callback argument count does not match operation");
                }
            }
            if builtin == Builtin::Iter {
                let [source] = arguments.as_ref() else {
                    return self.error(span, "iter requires one list or sequence");
                };
                let cursor = self.sequence_cursor(source.clone(), span)?;
                return Ok(Value::Sequence(cursor.sequence));
            }
            if builtin == Builtin::Collect {
                let [source, Value::Number(limit)] = arguments.as_ref() else {
                    return self.error(span, "collect requires a sequence and maximum item count");
                };
                if !limit.is_finite()
                    || *limit < 0.0
                    || limit.fract() != 0.0
                    || *limit >= usize::MAX as f64
                {
                    return self.error(
                        span,
                        "Collection limit must be a nonnegative representable integer",
                    );
                }
                if *limit as usize > self.max_collection_items {
                    return self.error(span, "Collection item limit exceeded");
                }
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                let mut output = self.slots();
                for _ in 0..*limit as usize {
                    let Some(item) = self.sequence_next(&mut cursor, span)? else {
                        break;
                    };
                    self.charge(1, span)?;
                    output
                        .push(item)
                        .map_err(|e| self.environment_error(span, e))?;
                }
                return Ok(Value::List(
                    memory::Buffer::from_slots(&self.memory, output)
                        .map_err(|e| self.environment_error(span, e))?,
                ));
            }
            if matches!(builtin, Builtin::Map | Builtin::Filter)
                && matches!(
                    arguments.first(),
                    Some(Value::Range { .. } | Value::Sequence(_))
                )
            {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "map/filter require a sequence and callback");
                };
                if !matches!(
                    callback,
                    Value::Function(_) | Value::Builtin(_) | Value::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let cursor = self.sequence_cursor(source.clone(), span)?;
                self.charge(cursor.sequence.stages.len() + 1, span)?;
                let mut sequence = (*cursor.sequence).clone();
                sequence
                    .stages
                    .append(
                        &self.memory,
                        SequenceStage {
                            callback: callback.clone(),
                            filter: builtin == Builtin::Filter,
                            span,
                            module: self.module,
                        },
                    )
                    .map_err(|e| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: format!("Runtime sequence allocation failed: {e:?}"),
                    })?;
                return Ok(Value::Sequence(self.sequence(sequence, span)?));
            }
            if matches!(builtin, Builtin::Any | Builtin::All) {
                let [source, callback] = arguments.as_ref() else {
                    return self.error(span, "any/all require a list or sequence and predicate");
                };
                let mut cursor = self.sequence_cursor(source.clone(), span)?;
                if !matches!(
                    callback,
                    Value::Function(_) | Value::Builtin(_) | Value::Host(_)
                ) {
                    return self.error(span, "Predicate must be callable");
                }
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let Value::Bool(result) = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                    else {
                        return self.error(span, "Predicate must return a boolean");
                    };
                    if result == (builtin == Builtin::Any) {
                        return Ok(Value::Bool(result));
                    }
                }
                return Ok(Value::Bool(builtin == Builtin::All));
            }
            if builtin == Builtin::Get {
                if arguments.len() != 2 {
                    return self.error(span, "get requires a collection and key");
                }
                let value = match (&arguments[0], &arguments[1]) {
                    (Value::Record(fields), Value::String(key)) => fields.get(key).cloned(),
                    (Value::List(items) | Value::Tuple(items), Value::Number(index)) => {
                        if !index.is_finite() || index.fract() != 0.0 || *index < 0.0 {
                            return self
                                .error(span, "Collection index must be a non-negative integer");
                        }
                        if *index >= items.len() as f64 {
                            None
                        } else {
                            items.get(*index as usize).cloned()
                        }
                    }
                    _ => {
                        return self.error(
                            span,
                            "get requires a record and string key, or list/tuple and integer index",
                        );
                    }
                };
                return Ok(match value {
                    Some(value) => Value::Variant("Some", self.buffer([value], span)?),
                    None => Value::Variant("None", self.buffer([], span)?),
                });
            }
            if builtin == Builtin::Len {
                if arguments.len() != 1 {
                    return self.error(span, "len requires one argument");
                }
                let count = match &arguments[0] {
                    Value::List(items) | Value::Tuple(items) => items.len(),
                    Value::Record(fields) => fields.len(),
                    Value::String(text) => {
                        self.charge(text.len(), span)?;
                        text.chars().count()
                    }
                    _ => return self.error(span, "len requires a list, tuple, record or string"),
                };
                return Ok(Value::Number(count as f64));
            }
            if builtin == Builtin::Assert {
                if !builtin.arity().contains(&arguments.len()) {
                    return self.error(span, "Incorrect argument count");
                }
                let message = match arguments.get(1) {
                    None => "Assertion failed",
                    Some(Value::String(message)) => message.as_str(),
                    _ => return self.error(span, "Assertion message must be a string"),
                };
                return match arguments[0] {
                    Value::Bool(true) => Ok(Value::Null),
                    Value::Bool(false) => self.error(span, message),
                    _ => self.error(span, "Assertion condition must be a boolean"),
                };
            }
            if !builtin.arity().contains(&arguments.len()) {
                return self.error(span, "Incorrect builtin argument count");
            }

            if matches!(
                builtin,
                Builtin::Some | Builtin::None | Builtin::Ok | Builtin::Err
            ) {
                let (tag, count) = match builtin {
                    Builtin::Some => ("Some", 1),
                    Builtin::None => ("None", 0),
                    Builtin::Ok => ("Ok", 1),
                    _ => ("Err", 1),
                };
                if arguments.len() != count {
                    return self.error(span, "Invalid variant argument count");
                }
                return Ok(Value::Variant(
                    tag,
                    self.buffer(arguments.iter().cloned(), span)?,
                ));
            }

            if builtin == Builtin::Mesh {
                let [Value::List(points), Value::List(faces)] = arguments.as_ref() else {
                    return self.error(span, "mesh requires vertex and triangle lists");
                };
                self.collection_growth(0, points.len(), span)?;
                self.collection_growth(0, faces.len(), span)?;
                let mut vertices = self.slots();
                for point in points {
                    self.charge(1, span)?;
                    let Value::Vector(v) = point else {
                        return self.error(span, "Mesh vertex must be vec3");
                    };
                    if v.len() != 3 {
                        return self.error(span, "Mesh vertex must be vec3");
                    }
                    vertices
                        .push([v[0], v[1], v[2]])
                        .map_err(|e| self.environment_error(span, e))?;
                }
                let mut triangles = self.slots();
                for face in faces {
                    self.charge(1, span)?;
                    let Value::List(indices) = face else {
                        return self.error(span, "Mesh triangle must be a list of three indices");
                    };
                    if indices.len() != 3 {
                        return self.error(span, "Mesh triangle must have three indices");
                    }
                    let mut triangle = [0; 3];
                    for (slot, index) in triangle.iter_mut().zip(indices) {
                        let Value::Number(index) = index else {
                            return self.error(span, "Mesh index must be an integer");
                        };
                        if !index.is_finite()
                            || index.fract() != 0.0
                            || *index < 0.0
                            || *index >= vertices.len() as f64
                        {
                            return self.error(span, "Mesh index out of bounds or non-integer");
                        }
                        *slot = *index as usize;
                    }
                    triangles
                        .push(triangle)
                        .map_err(|e| self.environment_error(span, e))?;
                }
                return super::engine_value::EngineMesh::new(&self.memory, vertices, triangles)
                    .map(Value::Mesh)
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Transform {
                let [Value::Mesh(mesh), Value::Matrix(matrix)] = arguments.as_ref() else {
                    return self.error(span, "transform requires a mesh and mat4");
                };
                self.collection_growth(0, mesh.vertices().len(), span)?;
                self.collection_growth(0, mesh.triangles().len(), span)?;
                let mut vertices = self.slots();
                for vertex in mesh.vertices() {
                    self.charge(1, span)?;
                    let transformed =
                        matrix
                            .apply_array(vertex, true)
                            .ok_or_else(|| RuntimeError {
                                stack: Vec::new(),
                                location: None,
                                module: self.module.map(str::to_owned),
                                span,
                                message: "Mesh transformation overflow".into(),
                            })?;
                    vertices
                        .push([transformed[0], transformed[1], transformed[2]])
                        .map_err(|e| self.environment_error(span, e))?;
                }
                self.charge(mesh.triangles().len(), span)?;
                let mut triangles = self.slots();
                triangles
                    .extend(mesh.triangles().iter().copied())
                    .map_err(|e| self.environment_error(span, e))?;
                return super::engine_value::EngineMesh::new(&self.memory, vertices, triangles)
                    .map(Value::Mesh)
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::GridMesh {
                let [Value::List(xs), Value::List(ys), callback] = arguments.as_ref() else {
                    return self.error(
                        span,
                        "grid_mesh requires x values, y values and a point function",
                    );
                };
                if xs.len() < 2 || ys.len() < 2 {
                    return self.error(span, "Grid requires at least two coordinates per axis");
                }
                let count = xs.len().checked_mul(ys.len()).ok_or_else(|| RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: self.module.map(str::to_owned),
                    span,
                    message: "Grid size overflow".into(),
                })?;
                self.collection_growth(0, count, span)?;
                let faces = (xs.len() - 1)
                    .checked_mul(ys.len() - 1)
                    .and_then(|n| n.checked_mul(2))
                    .ok_or_else(|| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: "Grid size overflow".into(),
                    })?;
                self.collection_growth(0, faces, span)?;
                self.charge(count, span)?;
                let mut vertices = self.slots();
                for y in ys {
                    for x in xs {
                        let point = self.call(
                            callback.clone(),
                            Cow::Borrowed(&[x.clone(), y.clone()]),
                            span,
                        )?;
                        let Value::Vector(point) = point else {
                            return self.error(span, "Grid callback must return vec3");
                        };
                        if point.len() != 3 {
                            return self.error(span, "Grid callback must return vec3");
                        }
                        vertices
                            .push([point[0], point[1], point[2]])
                            .map_err(|e| self.environment_error(span, e))?;
                    }
                }
                let mut triangles = self.slots();
                for row in 0..ys.len() - 1 {
                    for column in 0..xs.len() - 1 {
                        self.charge(2, span)?;
                        let a = row * xs.len() + column;
                        let b = a + 1;
                        let c = a + xs.len();
                        let d = c + 1;
                        triangles
                            .extend([[a, b, d], [a, d, c]])
                            .map_err(|e| self.environment_error(span, e))?;
                    }
                }
                return super::engine_value::EngineMesh::new(&self.memory, vertices, triangles)
                    .map(Value::Mesh)
                    .map_err(|message| RuntimeError {
                        stack: Vec::new(),
                        location: None,
                        module: self.module.map(str::to_owned),
                        span,
                        message: message.into(),
                    });
            }
            if builtin == Builtin::Zip {
                let [Value::List(a), Value::List(b)] = arguments.as_ref() else {
                    return self.error(span, "zip requires two lists");
                };
                self.collection_growth(0, a.len().min(b.len()), span)?;
                if !a.is_empty() && !b.is_empty() {
                    self.collection_growth(0, 2, span)?;
                }
                let mut values = self.slots();
                for (a, b) in a.iter().zip(b.iter()) {
                    self.check(span)?;
                    if self.remaining == 0 {
                        return self.error(span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    values
                        .push(Value::Tuple(self.buffer([a.clone(), b.clone()], span)?))
                        .map_err(|e| self.environment_error(span, e))?;
                }
                return Ok(Value::List(
                    memory::Buffer::from_slots(&self.memory, values)
                        .map_err(|e| self.environment_error(span, e))?,
                ));
            }
            if matches!(builtin, Builtin::Range | Builtin::RangeIter) {
                let (start, end, step) = match arguments.as_ref() {
                    [Value::Number(start), Value::Number(end)] => (*start, *end, 1.0),
                    [
                        Value::Number(start),
                        Value::Number(end),
                        Value::Number(step),
                    ] => (*start, *end, *step),
                    _ => return self.error(span, "range expects start, end and optional step"),
                };
                if step == 0.0 {
                    return self.error(span, "Range step must be nonzero");
                }
                if !start.is_finite() || !end.is_finite() || !step.is_finite() {
                    return self.error(span, "Range arguments must be finite");
                }
                if builtin == Builtin::RangeIter {
                    return Ok(Value::Range { start, end, step });
                }
                let mut values = self.slots();
                let mut current = start;
                while if step > 0.0 {
                    current < end
                } else {
                    current > end
                } {
                    self.check(span)?;
                    if self.remaining == 0 {
                        return self.error(span, "Execution limit exceeded");
                    }
                    self.remaining -= 1;
                    if values.len() >= self.max_collection_items {
                        return self.error(span, "Collection item limit exceeded");
                    }
                    values
                        .push(Value::Number(current))
                        .map_err(|e| self.environment_error(span, e))?;
                    let next = (values.len() as f64).mul_add(step, start);
                    if !next.is_finite() || next == current {
                        return self.error(span, "Range cannot advance finitely");
                    }
                    current = next;
                }
                return Ok(Value::List(
                    memory::Buffer::from_slots(&self.memory, values)
                        .map_err(|e| self.environment_error(span, e))?,
                ));
            }

            if matches!(builtin, Builtin::GroupBy | Builtin::FoldBy) {
                let mut arguments = arguments.iter().cloned();
                let source = arguments.next().unwrap();
                let callback = arguments.next().unwrap();
                if !matches!(
                    callback,
                    Value::Function(_) | Value::Builtin(_) | Value::Host(_)
                ) {
                    return self.error(span, "Callback must be callable");
                }
                let initial = arguments.next().unwrap_or(Value::Null);
                let reducer = arguments.next();
                if reducer.as_ref().is_some_and(|f| {
                    !matches!(f, Value::Function(_) | Value::Builtin(_) | Value::Host(_))
                }) {
                    return self.error(span, "Reducer must be callable");
                }
                let field = if reducer.is_some() { "value" } else { "values" };
                let mut cursor = self.sequence_cursor(source, span)?;
                let mut positions = self.slots::<(memory::Text, usize)>();
                let mut groups = self.slots::<(memory::Text, Value)>();
                while let Some(item) = self.sequence_next(&mut cursor, span)? {
                    let key = self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?;
                    let Value::String(key) = key else {
                        return self.error(span, "Group key must be a string");
                    };
                    self.string_growth(0, key.len(), span)?;
                    let position =
                        if let Ok(index) = positions.binary_search_by(|(k, _)| k.cmp(&key)) {
                            positions[index].1
                        } else {
                            self.collection_growth(groups.len(), 1, span)?;
                            self.collection_growth(0, 2, span)?;
                            self.string_growth(0, field.len(), span)?;
                            let position = groups.len();
                            let index = positions
                                .binary_search_by(|(k, _)| k.cmp(&key))
                                .unwrap_err();
                            positions
                                .push((key.clone(), position))
                                .map_err(|e| self.environment_error(span, e))?;
                            positions[index..].rotate_right(1);
                            groups
                                .push((
                                    key,
                                    if reducer.is_some() {
                                        initial.clone()
                                    } else {
                                        Value::List(self.buffer([], span)?)
                                    },
                                ))
                                .map_err(|e| self.environment_error(span, e))?;
                            position
                        };
                    if let Some(reducer) = &reducer {
                        let accumulator = std::mem::replace(&mut groups[position].1, Value::Null);
                        groups[position].1 =
                            self.call(reducer.clone(), Cow::Borrowed(&[accumulator, item]), span)?;
                    } else if let Value::List(values) = &mut groups[position].1 {
                        self.collection_growth(values.len(), 1, span)?;
                        values
                            .push(item)
                            .map_err(|e| self.environment_error(span, e))?;
                    }
                }
                let mut output = self.slots();
                for (key, values) in groups.iter() {
                    self.charge(1, span)?;
                    let mut record = self.slots();
                    record
                        .push((
                            memory::Text::from_str(&self.memory, "key")
                                .map_err(|e| self.environment_error(span, e))?,
                            Value::String(key.clone()),
                        ))
                        .map_err(|e| self.environment_error(span, e))?;
                    record
                        .push((
                            memory::Text::from_str(&self.memory, field)
                                .map_err(|e| self.environment_error(span, e))?,
                            values.clone(),
                        ))
                        .map_err(|e| self.environment_error(span, e))?;
                    output
                        .push(Value::Record(
                            memory::Record::from_slots(&self.memory, record)
                                .map_err(|e| self.environment_error(span, e))?,
                        ))
                        .map_err(|e| self.environment_error(span, e))?;
                }
                return Ok(Value::List(
                    memory::Buffer::from_slots(&self.memory, output)
                        .map_err(|e| self.environment_error(span, e))?,
                ));
            }
            if !matches!(
                builtin,
                Builtin::Map | Builtin::FlatMap | Builtin::Filter | Builtin::Fold
            ) {
                return self.math(builtin, &arguments, span);
            }
            let expected = if builtin == Builtin::Fold { 3 } else { 2 };
            if arguments.len() != expected {
                return self.error(span, "Incorrect argument count");
            }
            let mut arguments = arguments.iter().cloned();
            let source = arguments.next().unwrap();
            let mut cursor = self.sequence_cursor(source, span)?;
            let mut accumulator = if builtin == Builtin::Fold {
                arguments.next().unwrap()
            } else {
                Value::Null
            };
            let callback = arguments.next().unwrap();
            if !matches!(
                callback,
                Value::Function(_) | Value::Builtin(_) | Value::Host(_)
            ) {
                return self.error(span, "Callback must be callable");
            }
            let mut output = self.slots();
            while let Some(item) = self.sequence_next(&mut cursor, span)? {
                let result = if builtin == Builtin::Fold {
                    self.call(
                        callback.clone(),
                        Cow::Borrowed(&[accumulator, item.clone()]),
                        span,
                    )?
                } else {
                    self.call(
                        callback.clone(),
                        Cow::Borrowed(std::slice::from_ref(&item)),
                        span,
                    )?
                };
                accumulator = Value::Null;
                match builtin {
                    Builtin::Map => {
                        self.collection_growth(output.len(), 1, span)?;
                        output
                            .push(result)
                            .map_err(|e| self.environment_error(span, e))?;
                    }
                    Builtin::FlatMap => match result {
                        Value::List(values) => {
                            self.charge(values.len(), span)?;
                            self.collection_growth(output.len(), values.len(), span)?;
                            output
                                .extend(values)
                                .map_err(|e| self.environment_error(span, e))?;
                        }
                        _ => return self.error(span, "Flat map callback must return a list"),
                    },
                    Builtin::Filter => match result {
                        Value::Bool(true) => {
                            self.collection_growth(output.len(), 1, span)?;
                            output
                                .push(item)
                                .map_err(|e| self.environment_error(span, e))?;
                        }
                        Value::Bool(false) => {}
                        _ => return self.error(span, "Filter callback must return a boolean"),
                    },
                    Builtin::Fold => accumulator = result,
                    _ => unreachable!(),
                }
            }
            return if builtin == Builtin::Fold {
                Ok(accumulator)
            } else {
                Ok(Value::List(
                    memory::Buffer::from_slots(&self.memory, output)
                        .map_err(|e| self.environment_error(span, e))?,
                ))
            };
        }
        self.error(span, "Value is not callable")
    }

    pub(super) fn call_user(
        &mut self,
        function: Rc<Closure<'s>>,
        arguments: Cow<'_, [Value<'s>]>,
        span: Span,
    ) -> Result<Value<'s>> {
        if arguments.len() != function.parameters.len() {
            return self.error(span, "Incorrect argument count");
        }
        if function
            .parameter_types
            .iter()
            .zip(arguments.iter())
            .any(|(ty, value)| ty.as_ref().is_some_and(|ty| !ty.accepts_engine(value)))
        {
            return self.error(span, "Argument does not match its type annotation");
        }
        let mut environment = Environment::child(function.environment.clone());
        if let Some(name) = function.name {
            environment
                .insert(name, Value::Function(function.clone()))
                .map_err(|e| self.environment_error(span, e))?;
        }
        let previous_module = self.module;
        let previous_references =
            std::mem::replace(&mut self.current_references, function.references.clone());
        self.module = function.module;
        for (parameter, argument) in function.parameters.iter().zip(arguments.iter()) {
            if let Err(error) = self.bind_pattern(parameter, argument, &mut environment) {
                self.module = previous_module;
                self.current_references = previous_references;
                return Err(error);
            }
        }
        let result = match &function.body {
            FunctionBody::Expression(body) => self.expr(body, &environment),
            FunctionBody::Block(body) => {
                if self.depth >= self.max_depth {
                    self.module = previous_module;
                    self.current_references = previous_references;
                    return self.error(span, "Execution limit exceeded");
                }
                self.depth += 1;
                let result =
                    self.statements(&body.stmts, &mut environment)
                        .map(|(value, returned)| {
                            if returned == Flow::Return {
                                value
                            } else {
                                Value::Null
                            }
                        });
                self.depth -= 1;
                result
            }
        };
        self.module = previous_module;
        self.current_references = previous_references;
        drop(environment);
        self.reclaim_cells();
        let value = result?;
        if function
            .result_type
            .as_ref()
            .is_some_and(|ty| !ty.accepts_engine(&value))
        {
            return self.error(span, "Return value does not match its type annotation");
        }
        Ok(value)
    }
}
