//! Trial reference counting for runtime-owned cells. References from outside the
//! inspected graph (including active stack frames and host values) are roots.
use super::*;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Key {
    Cell(usize),
    Function(usize),
    Environment(usize),
    Sequence(usize),
    List(usize),
    Stages(usize),
}
struct Node {
    strong: usize,
    incoming: usize,
    edges: memory::Slots<usize>,
}
enum Work<'a, 's> {
    Cell(usize),
    Value(&'a Value<'s>),
    Function(&'a Closure<'s>),
    Environment(&'a Environment<'s>),
    Sequence(&'a Sequence<'s>),
    List(&'a [Value<'s>]),
    Stages(&'a [SequenceStage<'s>]),
}
struct Graph<'a, 's> {
    runtime: &'a Runtime<'a, 's>,
    ids: memory::Slots<Option<(Key, usize)>>,
    nodes: memory::Slots<Node>,
    work: memory::Slots<(usize, Work<'a, 's>)>,
    remaining: usize,
    span: Span,
}
impl<'a, 's> Graph<'a, 's> {
    fn charge(&mut self, amount: usize) -> Result<()> {
        self.runtime.check(self.span)?;
        let Some(remaining) = self.remaining.checked_sub(amount) else {
            self.remaining = 0;
            return self.runtime.error(self.span, "Execution limit exceeded");
        };
        self.remaining = remaining;
        Ok(())
    }
    fn hash(key: Key) -> usize {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hash);
        hash.finish() as usize
    }
    fn lookup(&mut self, key: Key) -> Result<Option<usize>> {
        if self.ids.is_empty() {
            return Ok(None);
        }
        let mut slot = Self::hash(key) & (self.ids.len() - 1);
        loop {
            self.charge(1)?;
            match self.ids[slot] {
                None => return Ok(None),
                Some((candidate, index)) if candidate == key => return Ok(Some(index)),
                _ => slot = (slot + 1) & (self.ids.len() - 1),
            }
        }
    }
    fn insert_id(&mut self, key: Key, index: usize) -> Result<()> {
        // Keep at least half the buckets empty, guaranteeing probe termination.
        if self.nodes.len() >= self.ids.len() / 2 {
            let capacity = self
                .ids
                .len()
                .checked_mul(2)
                .filter(|n| *n > 0)
                .unwrap_or(16);
            if capacity <= self.ids.len() {
                return Err(self
                    .runtime
                    .gc_allocation_error(self.span, memory::AllocationError::Capacity));
            }
            self.charge(capacity)?;
            let mut replacement = self.runtime.gc_slots();
            replacement
                .extend(std::iter::repeat_n(None, capacity))
                .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
            for old in 0..self.ids.len() {
                if let Some((key, index)) = self.ids[old] {
                    let mut slot = Self::hash(key) & (capacity - 1);
                    while replacement[slot].is_some() {
                        self.charge(1)?;
                        slot = (slot + 1) & (capacity - 1);
                    }
                    replacement[slot] = Some((key, index));
                }
            }
            self.ids = replacement;
        }
        let mut slot = Self::hash(key) & (self.ids.len() - 1);
        while self.ids[slot].is_some() {
            self.charge(1)?;
            slot = (slot + 1) & (self.ids.len() - 1);
        }
        self.ids[slot] = Some((key, index));
        Ok(())
    }
    fn node(&mut self, key: Key, strong: usize, work: Work<'a, 's>) -> Result<usize> {
        if let Some(index) = self.lookup(key)? {
            return Ok(index);
        }
        let index = self.nodes.len();
        self.insert_id(key, index)?;
        self.nodes
            .push(Node {
                strong,
                incoming: 0,
                edges: self.runtime.gc_slots(),
            })
            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
        self.work
            .push((index, work))
            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
        Ok(index)
    }
    fn edge(&mut self, from: usize, key: Key, strong: usize, work: Work<'a, 's>) -> Result<()> {
        let to = self.node(key, strong, work)?;
        self.nodes[to].incoming += 1;
        self.nodes[from]
            .edges
            .push(to)
            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
        Ok(())
    }
    fn environment(&mut self, from: usize, env: &'a memory::Shared<Environment<'s>>) -> Result<()> {
        self.edge(
            from,
            Key::Environment(memory::Shared::as_ptr(env) as usize),
            memory::Shared::strong_count(env),
            Work::Environment(env),
        )
    }
    fn build(&mut self) -> Result<()> {
        self.charge(self.runtime.cell_ids.len())?;
        for (index, id) in self.runtime.cell_ids.iter().enumerate() {
            if id.strong_count() != 0 {
                self.node(Key::Cell(index), id.strong_count(), Work::Cell(index))?;
            }
        }
        while let Some((from, work)) = self.work.pop() {
            self.charge(1)?;
            match work {
                Work::Cell(index) => self
                    .work
                    .push((from, Work::Value(&self.runtime.cells[index].0)))
                    .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?,
                Work::Value(value) => match value {
                    Value::Function(function) => self.edge(
                        from,
                        Key::Function(Rc::as_ptr(function) as usize),
                        Rc::strong_count(function),
                        Work::Function(function),
                    )?,
                    Value::Sequence(sequence) => self.edge(
                        from,
                        Key::Sequence(Rc::as_ptr(sequence) as usize),
                        Rc::strong_count(sequence),
                        Work::Sequence(sequence),
                    )?,
                    Value::List(values) | Value::Tuple(values) | Value::Variant(_, values) => {
                        self.charge(values.len())?;
                        self.work
                            .extend(values.iter().map(|value| (from, Work::Value(value))))
                            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
                    }
                    Value::Record(fields) => {
                        self.charge(fields.len())?;
                        self.work
                            .extend(fields.values().map(|value| (from, Work::Value(value))))
                            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
                    }
                    _ => {}
                },
                Work::Function(function) => self.environment(from, &function.environment)?,
                Work::Environment(environment) => {
                    if let Some(parent) = &environment.parent {
                        self.environment(from, parent)?;
                    }
                    for (_, binding) in environment.bindings.iter() {
                        self.charge(1)?;
                        match binding {
                            Binding::Value(value) => self
                                .work
                                .push((from, Work::Value(value)))
                                .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?,
                            Binding::Cell(id) => {
                                // An opaque host can retain a value from another run. It
                                // must not be interpreted as a slot in this runtime.
                                if id.released.as_ptr() == Rc::as_ptr(&self.runtime.released_cells)
                                {
                                    self.edge(
                                        from,
                                        Key::Cell(id.index),
                                        Rc::strong_count(id),
                                        Work::Cell(id.index),
                                    )?;
                                }
                            }
                        }
                    }
                }
                Work::Sequence(sequence) => {
                    if let SequenceSource::List(values) = &sequence.source {
                        self.edge(
                            from,
                            Key::List(memory::Buffer::as_ptr(values) as usize),
                            memory::Buffer::strong_count(values),
                            Work::List(values),
                        )?;
                    }
                    if let Some(stages) = &sequence.stages.0 {
                        self.edge(
                            from,
                            Key::Stages(memory::Shared::as_ptr(stages) as usize),
                            memory::Shared::strong_count(stages),
                            Work::Stages(stages),
                        )?;
                    }
                }
                Work::Stages(stages) => {
                    self.charge(stages.len())?;
                    self.work
                        .extend(
                            stages
                                .iter()
                                .map(|stage| (from, Work::Value(&stage.callback))),
                        )
                        .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
                }
                Work::List(values) => {
                    self.charge(values.len())?;
                    self.work
                        .extend(values.iter().map(|value| (from, Work::Value(value))))
                        .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
                }
            }
        }
        Ok(())
    }
    fn unreachable_cells(&mut self) -> Result<memory::Slots<usize>> {
        self.charge(self.nodes.len())?;
        let mut reachable = self.runtime.gc_slots();
        reachable
            .extend(std::iter::repeat_n(false, self.nodes.len()))
            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
        let mut pending = self.runtime.gc_slots();
        for (index, node) in self.nodes.iter().enumerate() {
            // Do not collect anything if an accounting invariant ever fails.
            if node.incoming > node.strong {
                return Ok(self.runtime.gc_slots());
            }
            if node.strong > node.incoming {
                pending
                    .push(index)
                    .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
            }
        }
        while let Some(index) = pending.pop() {
            self.charge(1)?;
            if std::mem::replace(&mut reachable[index], true) {
                continue;
            }
            self.charge(self.nodes[index].edges.len())?;
            pending
                .extend(self.nodes[index].edges.iter().copied())
                .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
        }
        let mut result = self.runtime.gc_slots();
        result
            .extend(
                self.ids
                    .iter()
                    .flatten()
                    .filter_map(|(key, index)| match key {
                        Key::Cell(cell) if !reachable[*index] => Some(*cell),
                        _ => None,
                    }),
            )
            .map_err(|e| self.runtime.gc_allocation_error(self.span, e))?;
        Ok(result)
    }
}

impl Runtime<'_, '_> {
    fn gc_slots<T>(&self) -> memory::Slots<T> {
        memory::Slots::new(&self.memory, 0).expect("empty GC storage requires no allocation")
    }
    fn gc_allocation_error(&self, span: Span, error: memory::AllocationError) -> RuntimeError {
        RuntimeError {
            stack: Vec::new(),
            location: None,
            module: self.module.map(str::to_owned),
            span,
            message: format!("Runtime collection allocation failed: {error:?}"),
        }
    }

    pub(super) fn collect_cell_cycles(&mut self, span: Span) -> Result<()> {
        let (unreachable, remaining) = {
            let mut graph = Graph {
                runtime: self,
                ids: self.gc_slots(),
                nodes: self.gc_slots(),
                work: self.gc_slots(),
                remaining: self.remaining,
                span,
            };
            let result = graph.build().and_then(|()| graph.unreachable_cells());
            (result, graph.remaining)
        };
        self.remaining = remaining;
        // No graph references remain while values and their captured IDs drop.
        for &index in unreachable?.iter() {
            self.cells[index] = (Value::Null, None);
        }
        self.reclaim_cells();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colliding_graph_keys_survive_growth_and_search_obeys_step_budget() {
        let program = Program::compile("0").unwrap();
        let token = CancellationToken::default();
        let limits = ExecutionLimits::new(100_000);
        let instance = program
            .instantiate(limits, &token, &[], &[], &[], &[])
            .unwrap();
        let runtime = &instance.runtime;
        let mut graph = Graph {
            runtime,
            ids: runtime.gc_slots(),
            nodes: runtime.gc_slots(),
            work: runtime.gc_slots(),
            remaining: limits.steps,
            span: instance.span,
        };
        // All keys collide through the 16 -> 32 -> 64 bucket growth sequence.
        let keys: Vec<_> = (0..)
            .map(Key::Cell)
            .filter(|key| Graph::hash(*key) & 63 == 0)
            .take(21)
            .collect();
        for (index, key) in keys[..20].iter().copied().enumerate() {
            assert_eq!(graph.node(key, 1, Work::Cell(index)).unwrap(), index);
        }
        assert_eq!(graph.ids.len(), 64);
        for (index, key) in keys[..20].iter().copied().enumerate() {
            assert_eq!(graph.lookup(key).unwrap(), Some(index));
            assert_eq!(graph.node(key, 1, Work::Cell(index)).unwrap(), index);
        }
        assert_eq!(graph.nodes.len(), 20);
        graph.remaining = 3;
        assert_eq!(
            graph.lookup(keys[20]).unwrap_err().message,
            "Execution limit exceeded"
        );
        assert_eq!(graph.remaining, 0);
        graph.remaining = 100;
        assert_eq!(graph.lookup(keys[20]).unwrap(), None);
        assert_eq!(graph.remaining, 79);
        token.cancel();
        assert_eq!(
            graph.lookup(keys[0]).unwrap_err().message,
            "Execution cancelled"
        );
    }
}
