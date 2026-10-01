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
}
struct Node {
    strong: usize,
    incoming: usize,
    edges: Vec<usize>,
}
enum Work<'a, 's> {
    Cell(usize),
    Value(&'a Value<'s>),
    Function(&'a Closure<'s>),
    Environment(&'a Environment<'s>),
    Sequence(&'a Sequence<'s>),
    List(&'a [Value<'s>]),
}
struct Graph<'a, 's> {
    runtime: &'a Runtime<'a, 's>,
    ids: HashMap<Key, usize>,
    nodes: Vec<Node>,
    work: Vec<(usize, Work<'a, 's>)>,
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
    fn node(&mut self, key: Key, strong: usize, work: Work<'a, 's>) -> usize {
        if let Some(&index) = self.ids.get(&key) {
            return index;
        }
        let index = self.nodes.len();
        self.ids.insert(key, index);
        self.nodes.push(Node {
            strong,
            incoming: 0,
            edges: Vec::new(),
        });
        self.work.push((index, work));
        index
    }
    fn edge(&mut self, from: usize, key: Key, strong: usize, work: Work<'a, 's>) {
        let to = self.node(key, strong, work);
        self.nodes[to].incoming += 1;
        self.nodes[from].edges.push(to);
    }
    fn environment(&mut self, from: usize, env: &'a Rc<Environment<'s>>) {
        self.edge(
            from,
            Key::Environment(Rc::as_ptr(env) as usize),
            Rc::strong_count(env),
            Work::Environment(env),
        );
    }
    fn build(&mut self) -> Result<()> {
        self.charge(self.runtime.cell_ids.len())?;
        for (index, id) in self.runtime.cell_ids.iter().enumerate() {
            if id.strong_count() != 0 {
                self.node(Key::Cell(index), id.strong_count(), Work::Cell(index));
            }
        }
        while let Some((from, work)) = self.work.pop() {
            self.charge(1)?;
            match work {
                Work::Cell(index) => self
                    .work
                    .push((from, Work::Value(&self.runtime.cells[index].0))),
                Work::Value(value) => match value {
                    Value::Function(function) => self.edge(
                        from,
                        Key::Function(Rc::as_ptr(function) as usize),
                        Rc::strong_count(function),
                        Work::Function(function),
                    ),
                    Value::Sequence(sequence) => self.edge(
                        from,
                        Key::Sequence(Rc::as_ptr(sequence) as usize),
                        Rc::strong_count(sequence),
                        Work::Sequence(sequence),
                    ),
                    Value::List(values) | Value::Tuple(values) | Value::Variant(_, values) => {
                        self.charge(values.len())?;
                        self.work
                            .extend(values.iter().map(|value| (from, Work::Value(value))));
                    }
                    Value::Record(fields) => {
                        self.charge(fields.len())?;
                        self.work
                            .extend(fields.values().map(|value| (from, Work::Value(value))));
                    }
                    _ => {}
                },
                Work::Function(function) => self.environment(from, &function.environment),
                Work::Environment(environment) => {
                    if let Some(parent) = &environment.parent {
                        self.environment(from, parent);
                    }
                    for binding in environment.bindings.values() {
                        self.charge(1)?;
                        match binding {
                            Binding::Value(value) => self.work.push((from, Work::Value(value))),
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
                                    );
                                }
                            }
                        }
                    }
                }
                Work::Sequence(sequence) => {
                    self.charge(sequence.stages.len())?;
                    if let SequenceSource::List(values) = &sequence.source {
                        self.edge(
                            from,
                            Key::List(Rc::as_ptr(values) as usize),
                            Rc::strong_count(values),
                            Work::List(values),
                        );
                    }
                    self.work.extend(
                        sequence
                            .stages
                            .iter()
                            .map(|stage| (from, Work::Value(&stage.callback))),
                    );
                }
                Work::List(values) => {
                    self.charge(values.len())?;
                    self.work
                        .extend(values.iter().map(|value| (from, Work::Value(value))));
                }
            }
        }
        Ok(())
    }
    fn unreachable_cells(&mut self) -> Result<Vec<usize>> {
        self.charge(self.nodes.len())?;
        let mut reachable = vec![false; self.nodes.len()];
        let mut pending = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            // Do not collect anything if an accounting invariant ever fails.
            if node.incoming > node.strong {
                return Ok(Vec::new());
            }
            if node.strong > node.incoming {
                pending.push(index);
            }
        }
        while let Some(index) = pending.pop() {
            self.charge(1)?;
            if std::mem::replace(&mut reachable[index], true) {
                continue;
            }
            self.charge(self.nodes[index].edges.len())?;
            pending.extend(self.nodes[index].edges.iter().copied());
        }
        Ok(self
            .ids
            .iter()
            .filter_map(|(key, &index)| match key {
                Key::Cell(cell) if !reachable[index] => Some(*cell),
                _ => None,
            })
            .collect())
    }
}

impl Runtime<'_, '_> {
    pub(super) fn collect_cell_cycles(&mut self, span: Span) -> Result<()> {
        let (unreachable, remaining) = {
            let mut graph = Graph {
                runtime: self,
                ids: HashMap::new(),
                nodes: Vec::new(),
                work: Vec::new(),
                remaining: self.remaining,
                span,
            };
            let result = graph.build().and_then(|()| graph.unreachable_cells());
            (result, graph.remaining)
        };
        self.remaining = remaining;
        // No graph references remain while values and their captured IDs drop.
        for index in unreachable? {
            self.cells[index] = (Value::Null, None);
        }
        self.reclaim_cells();
        Ok(())
    }
}
