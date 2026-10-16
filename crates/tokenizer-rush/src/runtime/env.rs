use super::{FunctionBody, Result, Runtime, RuntimeError, Value, ValueType, memory};
use crate::Name;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use themoretheless_tokenizer_core::Span;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Binding<'s> {
    Value(Value<'s>),
    Cell(Rc<CellId>),
}

/// Bindings and captured environments keep a slot alive. The runtime owns its
/// value separately so assigning a closure that captures itself cannot create
/// an Rc cycle owning the runtime. Dead slots are cleared at scope exit and
/// before allocation.
#[derive(Debug)]
pub(super) struct CellId {
    pub(super) index: usize,
    pub(super) released: Weak<RefCell<memory::Slots<usize>>>,
}

impl PartialEq for CellId {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && Weak::ptr_eq(&self.released, &other.released)
    }
}

impl Drop for CellId {
    fn drop(&mut self) {
        if let Some(released) = self.released.upgrade() {
            released
                .borrow_mut()
                .push(self.index)
                .expect("release queue reserved with cell");
        }
    }
}

#[derive(Debug)]
pub(super) struct Environment<'s> {
    pub(super) bindings: memory::Slots<(&'s str, Binding<'s>)>,
    pub(super) budget: memory::Budget,
    pub(super) parent: Option<memory::Shared<Environment<'s>>>,
}

impl PartialEq for Environment<'_> {
    fn eq(&self, other: &Self) -> bool {
        *self.bindings == *other.bindings && self.parent == other.parent
    }
}

impl<'s> Environment<'s> {
    pub(super) fn new(budget: &memory::Budget) -> Self {
        Self {
            bindings: memory::Slots::new(budget, 0).expect("empty bindings require no allocation"),
            budget: budget.clone(),
            parent: None,
        }
    }

    pub(super) fn child(parent: memory::Shared<Self>) -> Self {
        let mut child = Self::new(&parent.budget);
        child.parent = Some(parent);
        child
    }

    pub(super) fn try_clone(&self) -> std::result::Result<Self, memory::AllocationError> {
        let mut copy = Self::new(&self.budget);
        copy.bindings.reserve(self.bindings.len())?;
        for (name, binding) in self.bindings.iter() {
            copy.bindings
                .push((*name, binding.clone()))
                .expect("copy capacity reserved");
        }
        copy.parent = self.parent.clone();
        Ok(copy)
    }

    /// Drop every binding so cell references release exactly as they would
    /// when a freshly cloned scope goes out of scope.
    pub(super) fn release_bindings(&mut self) {
        while self.bindings.pop().is_some() {}
    }

    /// Rebind this scope as a child of `parent`, reusing the allocated
    /// binding storage. Used to recycle per-call environments.
    pub(super) fn reset_child(&mut self, parent: memory::Shared<Self>) {
        self.release_bindings();
        self.parent = Some(parent);
    }

    pub(super) fn get(&self, name: &str) -> Option<&Binding<'s>> {
        let mut current = self;
        loop {
            if let Ok(index) = current
                .bindings
                .binary_search_by_key(&name, |(key, _)| *key)
            {
                return Some(&current.bindings[index].1);
            }
            current = current.parent.as_deref()?;
        }
    }

    pub(super) fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub(super) fn insert_binding(
        &mut self,
        name: &'s str,
        binding: Binding<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        match self.bindings.binary_search_by_key(&name, |(key, _)| *key) {
            Ok(index) => self.bindings[index].1 = binding,
            Err(index) => {
                self.bindings.push((name, binding))?;
                self.bindings[index..].rotate_right(1);
            }
        }
        Ok(())
    }

    pub(super) fn insert(
        &mut self,
        name: &'s str,
        value: Value<'s>,
    ) -> std::result::Result<(), memory::AllocationError> {
        self.insert_binding(name, Binding::Value(value))
    }

    pub(super) fn extend(&mut self, mut other: Self) -> std::result::Result<(), memory::AllocationError> {
        debug_assert!(other.parent.is_none());
        let additional = other
            .bindings
            .iter()
            .filter(|(name, _)| {
                self.bindings
                    .binary_search_by_key(name, |(key, _)| *key)
                    .is_err()
            })
            .count();
        self.bindings.reserve(additional)?;
        while let Some((name, binding)) = other.bindings.pop() {
            self.insert_binding(name, binding)
                .expect("extension capacity reserved");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CaptureReference<'s> {
    pub(super) name: &'s str,
    pub(super) usage: Span,
    pub(super) definition: Option<Span>,
}

impl<'s> Runtime<'_, 's> {
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
        self.capture_excluding(span, environment, &[])
    }

    pub(super) fn capture_excluding(
        &self,
        span: Span,
        environment: &Environment<'s>,
        excluded_defs: &[Span],
    ) -> Result<memory::Shared<Environment<'s>>> {
        let references = &self.current_references;
        let start = references.partition_point(|reference| reference.usage.start < span.start);
        let mut captured = Environment::new(&self.memory);
        for reference in &references[start..] {
            if reference.usage.start >= span.end {
                break;
            }
            if reference.definition.is_some_and(|definition| {
                (definition.start >= span.start && definition.end <= span.end)
                    || excluded_defs.contains(&definition)
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
            let region = self.cell_regions[index];
            if region != 0
                && let Some(frame) = self
                    .region_stack
                    .iter_mut()
                    .rev()
                    .find(|frame| frame.id == region)
            {
                frame.live = frame.live.saturating_sub(1);
            }
            self.cells[index] = (Value::Null, None);
            self.free_cells
                .push(index)
                .expect("free list reserved with cell");
        }
    }

    pub(super) fn shared_body(
        &mut self,
        span: Span,
        build: impl FnOnce() -> FunctionBody<'s>,
    ) -> Rc<FunctionBody<'s>> {
        self.function_bodies
            .entry((self.module, span.start))
            .or_insert_with(|| Rc::new(build()))
            .clone()
    }

    /// Take a call environment from the pool or allocate a fresh child scope.
    pub(super) fn take_call_environment(
        &mut self,
        parent: &memory::Shared<Environment<'s>>,
    ) -> Environment<'s> {
        match self.environment_pool.pop() {
            Some(mut environment) => {
                environment.reset_child(parent.clone());
                environment
            }
            None => Environment::child(parent.clone()),
        }
    }

    /// Return a call environment to the pool after releasing its bindings, so
    /// repeated closure calls reuse the allocated binding storage.
    pub(super) fn recycle_call_environment(&mut self, mut environment: Environment<'s>) {
        const POOL_LIMIT: usize = 64;
        if self.environment_pool.len() < POOL_LIMIT {
            environment.release_bindings();
            environment.parent = None;
            self.environment_pool.push(environment);
        }
    }

    pub(super) fn allocate_cell(
        &mut self,
        value: Value<'s>,
        contract: Option<ValueType>,
        span: Span,
    ) -> Result<Rc<CellId>> {
        self.reclaim_cells();
        self.enforce_cell_limit(&value, span)?;
        self.cell_allocations += 1;
        if self.cell_allocations >= self.cell_collection_interval {
            self.collect_cell_cycles(span)?;
            self.cell_allocations = 0;
            self.cell_collection_interval = (self.cells.len() - self.free_cells.len())
                .saturating_mul(2)
                .max(64);
        }
        let mut reused = true;
        let index = if let Some(index) = self.free_cells.pop() {
            self.cells[index] = (value, contract);
            index
        } else {
            reused = false;
            let index = self.cells.len();
            self.cells
                .reserve(1)
                .and_then(|()| self.cell_ids.reserve(1))
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
            index
        };
        let id = Rc::new(CellId {
            index,
            released: Rc::downgrade(&self.released_cells),
        });
        self.cell_ids[index] = Rc::downgrade(&id);
        let region = self.region_stack.last().map_or(0, |frame| frame.id);
        if index == self.cell_regions.len() {
            self.cell_regions.push(region);
        } else {
            self.cell_regions[index] = region;
        }
        if let Some(frame) = self.region_stack.last_mut() {
            frame.live += 1;
            frame.cells.push(index);
            let stats = &mut self.region_stats[frame.stats];
            stats.allocated += 1;
            stats.reused_slots += reused as u64;
            stats.peak_live = stats.peak_live.max(frame.live);
            if frame.budget.is_some_and(|budget| frame.live > budget) {
                return self.error(span, "Region cell budget exceeded");
            }
        }
        Ok(id)
    }

    /// Take the value out of a mutable binding's cell, leaving `Null` behind.
    pub(super) fn move_cell(
        &mut self,
        name: &Name<'s>,
        environment: &Environment<'s>,
    ) -> Result<Value<'s>> {
        let Some(binding) = environment.get(name.text) else {
            return self.error(name.span, &format!("Unknown name: {}", name.text));
        };
        match binding {
            Binding::Value(_) => self.error(name.span, "move requires a mutable binding"),
            Binding::Cell(id) => Ok(std::mem::replace(&mut self.cells[id.index].0, Value::Null)),
        }
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
            message: format!("Runtime environment allocation failed: {error:?}"),
        }
    }
}
