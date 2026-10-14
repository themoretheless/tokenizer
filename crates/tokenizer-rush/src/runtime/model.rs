//! Declarative dataflow uses the ordinary Rush AST, checker and evaluator.
use super::*;
use crate::DeclarationRole;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelNode<'s> {
    pub name: &'s str,
    pub span: Span,
    pub dependencies: Vec<&'s str>,
    pub parameter: bool,
}
/// A checked, acyclic model. Forward node references are evaluated in dependency order.
pub struct ModelProgram<'s> {
    program: Program<'s>,
    nodes: Vec<ModelNode<'s>>,
    statements: Vec<usize>,
    dependencies: Vec<Vec<usize>>,
}
fn error(source: &str, span: Span, message: impl Into<String>) -> RuntimeError {
    RuntimeError {
        module: None,
        span,
        message: message.into(),
        stack: vec![],
        location: None,
    }
    .locate(source)
}
impl<'s> ModelProgram<'s> {
    pub fn compile(source: &'s str) -> Result<Self> {
        let details = crate::analyze_editor_details(source);
        let mut parsed = crate::parse(source);
        if !parsed.is_valid() {
            return Program::compile(source).map(|_| unreachable!());
        }
        let mut nodes = Vec::new();
        let mut statements = Vec::new();
        let mut named = HashMap::new();
        let mut prelude = Vec::new();
        for (index, statement) in parsed.module.items.iter().enumerate() {
            let (name, parameter) = match &statement.kind {
                StmtKind::Declaration {
                    name,
                    constant: true,
                    role,
                    ..
                } => (Some(name), *role == DeclarationRole::Parameter),
                StmtKind::Function { name, .. } => (Some(name), false),
                StmtKind::Show(_) => (None, false),
                StmtKind::Struct { .. } | StmtKind::Enum { .. } | StmtKind::Import(_) => {
                    prelude.push(index);
                    continue;
                }
                _ => {
                    return Err(error(
                        source,
                        statement.span,
                        "Models allow immutable declarations, functions, types, imports and show",
                    ));
                }
            };
            let name_text = name.map_or("", |n| n.text);
            if let Some(name) = name
                && named.insert(name.text, nodes.len()).is_some()
            {
                return Err(error(source, name.span, "Duplicate model binding"));
            }
            nodes.push(ModelNode {
                name: name_text,
                span: statement.span,
                dependencies: Vec::new(),
                parameter,
            });
            statements.push(index);
        }
        let mut dependencies = vec![Vec::new(); nodes.len()];
        for (index, node) in nodes.iter().enumerate() {
            let statement = &parsed.module.items[statements[index]];
            for reference in &details.references {
                if reference.usage.start < node.span.start || reference.usage.end > node.span.end {
                    continue;
                }
                let Some(name) = source.get(reference.usage.start..reference.usage.end) else {
                    continue;
                };
                let Some(&dependency) = named.get(name) else {
                    continue;
                };
                let declaration = &parsed.module.items[statements[dependency]];
                let definition = match &declaration.kind {
                    StmtKind::Declaration { name, .. } | StmtKind::Function { name, .. } => {
                        name.span
                    }
                    _ => continue,
                };
                if reference.definition.is_some_and(|d| d != definition) {
                    continue;
                }
                if index == dependency && matches!(statement.kind, StmtKind::Function { .. }) {
                    continue;
                }
                if !dependencies[index].contains(&dependency) {
                    dependencies[index].push(dependency);
                }
            }
            if node.parameter && !dependencies[index].is_empty() {
                return Err(error(
                    source,
                    node.span,
                    "Parameter defaults must not depend on model bindings",
                ));
            }
        }
        let shows: Vec<_> = statements
            .iter()
            .enumerate()
            .filter(|(_, i)| matches!(parsed.module.items[**i].kind, StmtKind::Show(_)))
            .map(|(i, _)| i)
            .collect();
        if shows.len() != 1 {
            return Err(error(
                source,
                parsed
                    .module
                    .items
                    .first()
                    .map_or(Span { start: 0, end: 0 }, |s| s.span),
                "A model requires exactly one show expression",
            ));
        }
        // Kahn's algorithm avoids recursive traversal of long dependency chains.
        let mut remaining: Vec<_> = dependencies.iter().map(Vec::len).collect();
        let mut dependents = vec![Vec::new(); nodes.len()];
        for (i, edges) in dependencies.iter().enumerate() {
            for &d in edges {
                dependents[d].push(i);
            }
        }
        let mut ready = std::collections::VecDeque::new();
        for (i, &count) in remaining.iter().enumerate() {
            if count == 0 {
                ready.push_back(i);
            }
        }
        let mut order = Vec::new();
        while let Some(i) = ready.pop_front() {
            order.push(i);
            for &next in &dependents[i] {
                remaining[next] -= 1;
                if remaining[next] == 0 {
                    ready.push_back(next);
                }
            }
        }
        if order.len() != nodes.len() {
            let i = remaining.iter().position(|&n| n != 0).unwrap();
            return Err(error(source, nodes[i].span, "Cyclic model dependency"));
        }
        // The output has no binding and cannot be a dependency of another node.
        order.retain(|&i| i != shows[0]);
        order.push(shows[0]);
        for index in 0..nodes.len() {
            nodes[index].dependencies =
                dependencies[index].iter().map(|&d| nodes[d].name).collect();
        }
        let original = parsed.module.items;
        parsed.module.items = prelude
            .into_iter()
            .chain(order.iter().map(|&i| statements[i]))
            .map(|i| original[i].clone())
            .collect();
        let program = Program::compile_parsed(parsed)?;
        for reference in program.references.iter().filter(|r| r.definition.is_none()) {
            if !builtin_catalog()
                .iter()
                .any(|(name, _)| *name == reference.name)
                && !program
                    .imports
                    .iter()
                    .any(|name| name.text == reference.name)
            {
                return Err(error(
                    source,
                    reference.usage,
                    format!("Unknown model name: {}", reference.name),
                ));
            }
        }
        let mut strict = program.parsed.clone();
        if program.imports.is_empty() {
            crate::analysis::check_strict(&mut strict);
        }
        if let Some(d) = strict.diagnostics.first() {
            return Err(error(source, d.span, format!("{}: {}", d.code, d.message)));
        }
        let statements = order
            .iter()
            .enumerate()
            .map(|(i, _)| program.parsed.module.items.len() - order.len() + i)
            .collect();
        let old_nodes = nodes;
        let old_dependencies = dependencies;
        let positions: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(new, &old)| (old, new))
            .collect();
        let nodes = order.iter().map(|&i| old_nodes[i].clone()).collect();
        let dependencies = order
            .iter()
            .map(|&i| old_dependencies[i].iter().map(|d| positions[d]).collect())
            .collect();
        Ok(Self {
            program,
            nodes,
            statements,
            dependencies,
        })
    }
    pub fn nodes(&self) -> &[ModelNode<'s>] {
        &self.nodes
    }
    pub fn instantiate<'a>(
        &'a self,
        limits: ExecutionLimits,
        cancellation: &'a CancellationToken,
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<ModelInstance<'a, 's>> {
        self.program.validate_strict_modules(modules)?;
        let script = self
            .program
            .instantiate(limits, cancellation, &[], &[], &[], modules)?;
        let mut contracts = HashMap::new();
        for node in self.nodes.iter().filter(|n| n.parameter) {
            let statement = self
                .program
                .parsed
                .module
                .items
                .iter()
                .find(|s| s.span == node.span)
                .unwrap();
            let StmtKind::Declaration { ty, .. } = &statement.kind else {
                unreachable!()
            };
            let value = script.get(node.name).unwrap();
            let ty = match ty {
                Some(ty) => script.runtime.annotation(ty)?,
                None => ValueType::inferred(&value).ok_or_else(|| {
                    error(
                        self.program.parsed.source,
                        node.span,
                        "Ambiguous parameter default requires a type annotation",
                    )
                })?,
            };
            contracts.insert(node.name, ty);
        }
        Ok(ModelInstance {
            model: self,
            script,
            contracts,
        })
    }
}
/// Persistent evaluated model; updates are atomic for model bindings.
pub struct ModelInstance<'a, 's> {
    model: &'a ModelProgram<'s>,
    script: ScriptInstance<'a, 's>,
    contracts: HashMap<&'s str, ValueType>,
}
impl<'a, 's> ModelInstance<'a, 's> {
    pub fn get(&self, name: &str) -> Option<Value<'s>> {
        self.script.get(name)
    }
    pub fn output(&self) -> &Value<'s> {
        self.script.initial_value()
    }
    /// Recompute only the transitive dependents; return their names in evaluation order.
    pub fn set_parameter(
        &mut self,
        name: &str,
        value: Value<'s>,
        limits: ExecutionLimits,
    ) -> Result<Vec<&'s str>> {
        let (name, contract) = self.contracts.get_key_value(name).ok_or_else(|| {
            error(
                self.model.program.parsed.source,
                self.script.span,
                "Unknown model parameter",
            )
        })?;
        let name = *name;
        if !contract.accepts(&value) {
            return Err(error(
                self.model.program.parsed.source,
                self.script.span,
                "Parameter value does not match its contract",
            ));
        }
        self.script.coroutine_limits(limits)?;
        let unchanged = match (self.script.get(name), &value) {
            (Some(Value::Number(a)), Value::Number(b)) => a == *b,
            (Some(Value::Bool(a)), Value::Bool(b)) => a == *b,
            (Some(Value::String(a)), Value::String(b)) => a == *b,
            _ => false,
        };
        if unchanged {
            return Ok(Vec::new());
        }
        self.script
            .runtime
            .host_value_size(&value, self.script.span, 0)?;
        let snapshot = self
            .script
            .environment
            .try_clone()
            .map_err(|e| self.script.runtime.environment_error(self.script.span, e))?;
        let old_output = self.script.initial_value.clone();
        // Pin live mutable captures so collection cannot recycle their slots during
        // an update, and save their values for rollback (including imported state).
        let mut cells = memory::Slots::new(&self.script.runtime.memory, 0)
            .map_err(|e| self.script.runtime.environment_error(self.script.span, e))?;
        for (index, id) in self.script.runtime.cell_ids.iter().enumerate() {
            if let Some(id) = id.upgrade() {
                cells
                    .push((index, self.script.runtime.cells[index].clone(), id))
                    .map_err(|e| self.script.runtime.environment_error(self.script.span, e))?;
            }
        }
        let mut dirty = vec![false; self.model.nodes.len()];
        let parameter = self
            .model
            .nodes
            .iter()
            .position(|n| n.name == name)
            .unwrap();
        dirty[parameter] = true;
        let result: Result<Vec<&'s str>> = (|| {
            self.script
                .environment
                .insert(name, value)
                .map_err(|e| self.script.runtime.environment_error(self.script.span, e))?;
            let mut changed = Vec::new();
            for (index, node) in self.model.nodes.iter().enumerate() {
                if index == parameter {
                    continue;
                }
                if !self.model.dependencies[index].iter().any(|&d| dirty[d]) {
                    continue;
                }
                dirty[index] = true;
                let statement =
                    &self.model.program.parsed.module.items[self.model.statements[index]];
                let (value, _) = self.script.runtime.statements(
                    std::slice::from_ref(statement),
                    &mut self.script.environment,
                )?;
                if matches!(statement.kind, StmtKind::Show(_)) {
                    self.script.initial_value = value;
                }
                if !node.name.is_empty() {
                    changed.push(node.name);
                }
            }
            self.script.enforce_memory_limit()?;
            let roots = self
                .script
                .environment
                .try_clone()
                .map_err(|e| self.script.runtime.environment_error(self.script.span, e))?;
            self.script.runtime.instance_roots = Some(roots);
            Ok(changed)
        })();
        if result.is_err() {
            self.script.environment = snapshot;
            self.script.initial_value = old_output;
            for (index, value, _) in cells.iter() {
                self.script.runtime.cells[*index] = value.clone();
            }
        }
        drop(cells);
        self.script.runtime.reclaim_cells();
        result.map_err(|e| {
            if e.location.is_some() {
                return e;
            }
            let source = self
                .script
                .runtime
                .sources
                .iter()
                .find(|(module, _)| module.map(str::to_owned) == e.module)
                .map_or(self.model.program.parsed.source, |(_, source)| *source);
            e.locate(source)
        })
    }
}
