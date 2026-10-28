//! Bounded expression evaluator. Closures capture immutable snapshots.
#[allow(unused_imports)]
use crate::{Block, Expr, ExprKind, Name, Stmt, StmtKind};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};
#[allow(unused_imports)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use themoretheless_tokenizer_core::Span;

pub(crate) mod builtins;
mod calls;
mod cell_gc;
mod coroutine;
mod env;
mod expr;
mod instance;
pub mod json;
mod memory;
mod model;
mod mutation;
pub(crate) mod ops;
mod regions;
mod scheduler;
mod sequence;
mod stmt;
mod types;

pub use builtins::{Builtin, builtin_catalog};
pub use coroutine::{CoroutineId, CoroutineState};
pub use instance::{OwnedScriptInstance, ScriptState, StateValue};
pub use json::{parse as json_parse, stringify as json_stringify};
pub use model::{ModelInstance, ModelNode, ModelProgram};
pub use regions::RegionStats;
pub use scheduler::{
    CoroutineScheduler, OwnedCoroutineScheduler, ScheduledState, ScheduledStep, WakeRequest,
};
pub use sequence::{HostSequence, HostSequenceIterator, Sequence};
pub use types::{
    CallFrame, CancellationToken, Closure, ExecutionLimits, HostCallback, HostFunction,
    HostRegistration, InlineVec, RecordMap, RecordShape, RuntimeError, SourceLocation, UserData,
    Value, ValueType,
};

type Result<T> = std::result::Result<T, RuntimeError>;

use env::{Binding, CaptureReference, CellId, Environment};
pub(crate) use ops::{exact_remainder, format_value_for_display};
use regions::RegionFrame;
#[allow(unused_imports)]
use sequence::{
    HostSource, SequenceCursor, SequenceSource, SequenceStage, SequenceStages, StageKind,
};
use stmt::Flow;
use types::{ContextualHostCallback, FunctionBody};
pub(crate) use types::{
    int_f64_cmp, int_f64_eq, is_int_literal, parse_f64_literal, parse_int_literal,
    parse_neg_int_literal, parse_signed_int_str,
};

/// One initialized script. Mutable captures and module caches survive calls.
/// Source and cancellation token must outlive the instance. Errors do not roll back mutations.
pub struct ScriptInstance<'a, 's> {
    runtime: Runtime<'a, 's>,
    environment: Environment<'s>,
    span: Span,
    initial_value: Value<'s>,
}
impl<'a, 's> ScriptInstance<'a, 's> {
    pub fn initial_value(&self) -> &Value<'s> {
        &self.initial_value
    }
    pub fn get(&self, name: &str) -> Option<Value<'s>> {
        match self.environment.get(name)? {
            Binding::Value(value) => Some(value.clone()),
            Binding::Cell(cell) => Some(self.runtime.cells[cell.index].0.clone()),
        }
    }
    /// Invoke a named callable with a fresh execution budget and the instance's token.
    pub fn call(
        &mut self,
        name: &str,
        arguments: &[Value<'s>],
        limits: ExecutionLimits,
    ) -> Result<Value<'s>> {
        self.runtime.reclaim_cells();
        if limits.max_depth > 64 {
            return self
                .runtime
                .error(self.span, "Maximum evaluation depth cannot exceed 64");
        }
        self.runtime.remaining = limits.steps;
        self.runtime.max_depth = limits.max_depth;
        self.runtime.max_collection_items = limits.max_collection_items;
        self.runtime.max_string_bytes = limits.max_string_bytes;
        self.runtime.check(self.span)?;
        let function = self.get(name).ok_or_else(|| RuntimeError {
            stack: Vec::new(),
            location: None,
            module: None,
            span: self.span,
            message: format!("Unknown function: {name}"),
        })?;
        for value in arguments {
            self.runtime.host_value_size(value, self.span, 0)?;
        }
        self.enforce_memory_limit()?;
        let value = self
            .runtime
            .call(function, Cow::Borrowed(arguments), self.span)?;
        self.enforce_memory_limit()?;
        Ok(value)
    }

    /// Incrementally execute top-level statements in this instance's environment.
    /// Useful for REPL and interactive notebook sessions.
    pub fn eval_chunk(&mut self, source: &'s str) -> Result<Option<Value<'s>>> {
        let mut parsed = crate::parse(source);
        let (references, contracts) = crate::analysis::compile_analysis(&mut parsed);
        if !parsed.is_valid() {
            let diagnostic = parsed.diagnostics.first();
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: diagnostic.map_or(parsed.module.span, |d| d.span),
                message: diagnostic.map_or_else(
                    || "Invalid Rush program".into(),
                    |d| format!("{}: {}", d.code, d.message),
                ),
            }
            .locate(source));
        }
        if parsed.module.items.is_empty() {
            return Ok(None);
        }
        let mut capture_refs: Vec<_> = references
            .into_iter()
            .map(|reference| CaptureReference {
                name: &source[reference.usage.start..reference.usage.end],
                usage: reference.usage,
                definition: reference.definition,
            })
            .collect();
        capture_refs.sort_by_key(|reference| reference.usage.start);
        self.runtime.current_references = Rc::new(capture_refs);
        if let Some(contracts_map) = self.runtime.contracts.get_mut(&None) {
            if let Some(cm) = Rc::get_mut(contracts_map) {
                cm.extend(contracts);
            } else {
                let mut new_contracts = (**contracts_map).clone();
                new_contracts.extend(contracts);
                *contracts_map = Rc::new(new_contracts);
            }
        }
        self.runtime.sources.insert(None, source);
        self.runtime.remaining = 1_000_000;
        self.runtime.check(parsed.module.span)?;

        let is_expr_or_show = matches!(
            parsed.module.items.last().map(|s| &s.kind),
            Some(StmtKind::Expr(_)) | Some(StmtKind::Show(_))
        );

        let (val, _) = self
            .runtime
            .statements(&parsed.module.items, &mut self.environment)
            .map_err(|e| e.locate(source))?;

        if is_expr_or_show {
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    /// List active user-defined variables and their values.
    pub fn list_variables(&self) -> Vec<(&str, Value<'s>)> {
        let mut list = Vec::new();
        for (name, binding) in self.environment.bindings.iter() {
            if builtin_catalog().iter().any(|(b, _)| b == name) {
                continue;
            }
            let val = match binding {
                Binding::Value(v) => v.clone(),
                Binding::Cell(cell) => {
                    if let Ok(idx) = self.runtime.cell_index(cell, Span::new(0, 0)) {
                        self.runtime.cells[idx].0.clone()
                    } else {
                        continue;
                    }
                }
            };
            list.push((*name, val));
        }
        list
    }
}

/// An analyzed program that can be evaluated repeatedly without reparsing.
pub struct Program<'s> {
    imports: Vec<Name<'s>>,
    contracts: Rc<HashMap<usize, crate::analysis::FunctionContract>>,
    parsed: crate::Parse<'s>,
    references: Rc<Vec<CaptureReference<'s>>>,
}
struct PreparedModules {
    interfaces: crate::analysis::ModuleInterfaces,
    root_contracts: Rc<HashMap<usize, crate::analysis::FunctionContract>>,
}
impl<'s> Program<'s> {
    /// Explicit public interface. None preserves the legacy returned-record interface.
    pub fn exports(&self) -> Option<Vec<Name<'s>>> {
        let mut found = false;
        let mut names = Vec::new();
        for statement in &self.parsed.module.items {
            if let StmtKind::Export(exports) = &statement.kind {
                found = true;
                names.extend(exports.iter().cloned());
            }
        }
        found.then_some(names)
    }
    /// All syntactic imports, including imports inside function and branch bodies.
    pub fn imports(&self) -> Vec<Name<'s>> {
        self.imports.clone()
    }
    pub(crate) fn import_refs(&self) -> &[Name<'s>] {
        &self.imports
    }
    fn collect_imports(statements: &[Stmt<'s>]) -> Vec<Name<'s>> {
        let mut imports = Vec::new();
        let mut pending = vec![statements];
        while let Some(statements) = pending.pop() {
            for statement in statements {
                match &statement.kind {
                    StmtKind::Import(name) => imports.push(name.clone()),
                    StmtKind::Function { body, .. }
                    | StmtKind::For { body, .. }
                    | StmtKind::While { body, .. } => pending.push(&body.stmts),
                    StmtKind::If {
                        then_block,
                        else_block,
                        ..
                    } => {
                        pending.push(&then_block.stmts);
                        if let Some(block) = else_block {
                            pending.push(&block.stmts);
                        }
                    }
                    _ => {}
                }
            }
        }
        imports
    }
    /// Validate the reachable module graph without running any module code.
    pub fn validate_modules(&self, modules: &[(&str, &Program<'_>)]) -> Result<()> {
        self.prepare_modules(modules).map(|_| ())
    }
    fn prepare_modules(&self, modules: &[(&str, &Program<'_>)]) -> Result<PreparedModules> {
        if modules.is_empty() && self.imports.is_empty() {
            return Ok(PreparedModules {
                interfaces: HashMap::new(),
                root_contracts: self.contracts.clone(),
            });
        }
        let mut registry = HashMap::new();
        for (name, program) in modules {
            if registry.insert(*name, *program).is_some() {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: None,
                    span: self.parsed.module.span,
                    message: format!("Duplicate module: {name}"),
                });
            }
        }
        let mut done = std::collections::HashSet::new();
        let mut active = std::collections::HashSet::new();
        let mut pending: Vec<(Option<&str>, &[Name<'_>], usize)> =
            vec![(None, self.import_refs(), 0)];
        while let Some((module, imports, index)) = pending.last_mut() {
            if *index == imports.len() {
                if let Some(name) = module {
                    active.remove(name);
                    done.insert(*name);
                }
                pending.pop();
                continue;
            }
            let import = &imports[*index];
            *index += 1;
            let Some(program) = registry.get(import.text) else {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: module.map(str::to_owned),
                    span: import.span,
                    message: format!("Unknown module: {}", import.text),
                });
            };
            if active.contains(import.text) {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: module.map(str::to_owned),
                    span: import.span,
                    message: format!("Cyclic module import: {}", import.text),
                });
            }
            if done.contains(import.text) {
                continue;
            }
            active.insert(import.text);
            let name = import.text;
            pending.push((Some(name), program.import_refs(), 0));
        }
        let reachable: Vec<_> = modules
            .iter()
            .copied()
            .filter(|(name, _)| done.contains(name))
            .collect();
        let interfaces = crate::analysis::module_interfaces(&reachable);
        let root = if self.imports.is_empty() {
            None
        } else {
            Some(crate::analysis::resolved_interface(
                &mut self.parsed.clone(),
                interfaces.clone(),
                None,
            ))
        };
        for (name, source, diagnostics) in std::iter::once((
            None,
            self.parsed.source,
            root.as_ref()
                .map(|i| i.diagnostics.as_slice())
                .unwrap_or(&[]),
        ))
        .chain(reachable.iter().map(|(name, program)| {
            (
                Some(*name),
                program.parsed.source,
                interfaces[*name].diagnostics.as_slice(),
            )
        })) {
            if let Some(d) = diagnostics.first() {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: name.map(str::to_owned),
                    span: d.span,
                    message: format!("{}: {}", d.code, d.message),
                }
                .locate(source));
            }
        }
        Ok(PreparedModules {
            root_contracts: root.map_or_else(|| self.contracts.clone(), |i| Rc::new(i.contracts())),
            interfaces,
        })
    }

    /// Require complete named-function contracts with the registered module interfaces available.
    pub fn validate_strict_modules(&self, modules: &[(&str, &Program<'_>)]) -> Result<()> {
        self.validate_modules(modules)?;
        let interfaces = crate::analysis::module_interfaces(modules);
        for (name, program) in
            std::iter::once((None, self)).chain(modules.iter().map(|(n, p)| (Some(*n), *p)))
        {
            let mut parsed = program.parsed.clone();
            crate::analysis::check_strict_modules(&mut parsed, interfaces.clone(), name);
            if let Some(d) = parsed.diagnostics.first() {
                return Err(RuntimeError {
                    stack: Vec::new(),
                    location: None,
                    module: name.map(str::to_owned),
                    span: d.span,
                    message: format!("{}: {}", d.code, d.message),
                }
                .locate(parsed.source));
            }
        }
        Ok(())
    }
    /// Compile with complete contracts for every named function. Ambiguous dynamic boundaries require annotations.
    pub fn compile_strict(source: &'s str) -> Result<Self> {
        let program = Self::compile(source)?;
        let mut parsed = program.parsed.clone();
        crate::analysis::check_strict(&mut parsed);
        if let Some(d) = parsed.diagnostics.first() {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: d.span,
                message: format!("{}: {}", d.code, d.message),
            }
            .locate(source));
        }
        Ok(program)
    }
    pub(crate) fn parsed_clone(&self) -> crate::Parse<'s> {
        self.parsed.clone()
    }
    pub fn compile(source: &'s str) -> Result<Self> {
        Self::compile_parsed(crate::parse(source))
    }
    fn compile_parsed(mut parsed: crate::Parse<'s>) -> Result<Self> {
        let source = parsed.source;
        let (references, contracts) = crate::analysis::compile_analysis(&mut parsed);
        if !parsed.is_valid() {
            let diagnostic = parsed.diagnostics.first();
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: diagnostic.map_or(parsed.module.span, |d| d.span),
                message: diagnostic.map_or_else(
                    || "Invalid Rush program".into(),
                    |d| format!("{}: {}", d.code, d.message),
                ),
            }
            .locate(source));
        }
        let mut references: Vec<_> = references
            .into_iter()
            .map(|reference| CaptureReference {
                name: &source[reference.usage.start..reference.usage.end],
                usage: reference.usage,
                definition: reference.definition,
            })
            .collect();
        references.sort_by_key(|reference| reference.usage.start);
        Ok(Self {
            imports: Self::collect_imports(&parsed.module.items),
            parsed,
            references: Rc::new(references),
            contracts: Rc::new(contracts),
        })
    }
    /// Numeric inputs permit parameter/time updates without changing the source.
    pub fn run(
        &self,
        budget: usize,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
    ) -> Result<Value<'s>> {
        self.run_with_host(budget, cancellation, inputs, &[])
    }
    /// Register host functions using the same callable values as Rush functions.
    pub fn run_with_host(
        &self,
        budget: usize,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
        functions: &[Rc<HostFunction>],
    ) -> Result<Value<'s>> {
        self.run_with_modules(budget, cancellation, inputs, functions, &[])
    }
    /// Modules are supplied explicitly; each module's final value must be a record.
    pub fn run_with_modules(
        &self,
        budget: usize,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
        functions: &[Rc<HostFunction>],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<Value<'s>> {
        self.run_with_limits(
            ExecutionLimits::new(budget),
            cancellation,
            inputs,
            functions,
            modules,
        )
    }
    /// Execute with explicit step, evaluation-depth and data-size limits, shared by all modules
    /// and Rush function calls. Existing run methods use a depth ceiling of 64.
    pub fn run_with_limits(
        &self,
        limits: ExecutionLimits,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, f64)],
        functions: &[Rc<HostFunction>],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<Value<'s>> {
        if inputs.iter().any(|(_, value)| !value.is_finite()) {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: self.parsed.module.span,
                message: "Invalid or duplicate input parameter".into(),
            });
        }
        let values: Vec<_> = inputs
            .iter()
            .map(|(name, value)| (*name, Value::Number(*value)))
            .collect();
        self.run_with_values(limits, cancellation, &values, functions, modules)
    }
    /// Execute once with arbitrary values, including host objects and event records.
    pub fn run_with_values(
        &self,
        limits: ExecutionLimits,
        cancellation: &CancellationToken,
        inputs: &[(&'s str, Value<'s>)],
        functions: &[Rc<HostFunction>],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<Value<'s>> {
        Ok(self
            .instantiate(limits, cancellation, inputs, functions, &[], modules)?
            .initial_value)
    }
    /// Initialize top-level code once. Context registrations share the ordinary host signatures.
    pub fn instantiate<'a>(
        &self,
        limits: ExecutionLimits,
        cancellation: &'a CancellationToken,
        inputs: &[(&'s str, Value<'s>)],
        functions: &[Rc<HostFunction>],
        contextual: &[HostRegistration],
        modules: &[(&'s str, &Program<'s>)],
    ) -> Result<ScriptInstance<'a, 's>> {
        let prepared = self.prepare_modules(modules)?;
        if !functions.is_empty() || !contextual.is_empty() {
            let hosts: HashMap<_, _> = functions
                .iter()
                .cloned()
                .chain(
                    contextual
                        .iter()
                        .map(|registration| registration.function.clone()),
                )
                .map(|function| (function.name.to_owned(), function))
                .collect();
            for (name, program) in std::iter::once((None, self)).chain(
                modules
                    .iter()
                    .map(|(name, program)| (Some(*name), *program)),
            ) {
                let mut parsed = program.parsed.clone();
                crate::analysis::check_host_calls(
                    &mut parsed,
                    crate::InputLimits::conservative().max_diagnostics,
                    true,
                    hosts.clone(),
                );
                // Errors are fatal; non-fatal lints (e.g. region-escape
                // warnings) must not block a valid program.
                if !parsed.is_valid() {
                    let diagnostic = parsed.diagnostics.first();
                    return Err(RuntimeError {
                        module: name.map(str::to_owned),
                        span: diagnostic.map_or(parsed.module.span, |d| d.span),
                        message: diagnostic.map_or_else(
                            || "Invalid Rush program".into(),
                            |d| format!("{}: {}", d.code, d.message),
                        ),
                        stack: Vec::new(),
                        location: None,
                    }
                    .locate(parsed.source));
                }
            }
        }
        if limits.max_depth > 64 {
            return Err(RuntimeError {
                stack: Vec::new(),
                location: None,
                module: None,
                span: self.parsed.module.span,
                message: "Maximum evaluation depth cannot exceed 64".into(),
            });
        }
        let root_contracts = prepared.root_contracts;
        let memory = memory::Budget::new(usize::MAX);
        let mut runtime = Runtime {
            module: None,
            early_return: None,
            coroutines: HashMap::new(),
            user_types: HashMap::new(),
            struct_fields: HashMap::new(),
            modules: HashMap::new(),
            module_cache: memory::Slots::new(&memory, 0)
                .expect("empty module cache requires no allocation"),
            loading: memory::Slots::new(&memory, 0)
                .expect("empty module stack requires no allocation"),
            module_globals: Environment::new(&memory),
            memory: memory.clone(),
            contracts: HashMap::from([(None, root_contracts)]),
            references: HashMap::from([(None, self.references.clone())]),
            current_references: self.references.clone(),
            cells: memory::Slots::new(&memory, 0)
                .expect("empty cell storage requires no allocation"),
            released_cells: Rc::new(RefCell::new(
                memory::Slots::new(&memory, 0).expect("empty release queue requires no allocation"),
            )),
            free_cells: memory::Slots::new(&memory, 0)
                .expect("empty free cell storage requires no allocation"),
            cell_ids: memory::Slots::new(&memory, 0)
                .expect("empty cell ID storage requires no allocation"),
            cell_allocations: 0,
            cell_collection_interval: 64,
            remaining: limits.steps,
            max_depth: limits.max_depth,
            max_collection_items: limits.max_collection_items,
            max_string_bytes: limits.max_string_bytes,
            depth: 0,
            sources: HashMap::from([(None, self.parsed.source)]),
            memory_limit: usize::MAX,
            initial_data_bytes: 0,
            instance_roots: None,
            environment_pool: Vec::new(),
            function_bodies: HashMap::new(),
            region_stack: Vec::new(),
            cell_regions: Vec::new(),
            next_region: 0,
            skip_call_region: false,
            region_stats: Vec::new(),
            cancellation,
            contextual: contextual
                .iter()
                .map(|entry| (Rc::as_ptr(&entry.function), entry.callback.clone()))
                .collect(),
        };
        runtime.check(self.parsed.module.span)?;
        let mut environment = Environment::new(&memory);
        for &(name, builtin) in builtin_catalog() {
            environment
                .insert(name, Value::Builtin(builtin))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        for function in functions
            .iter()
            .chain(contextual.iter().map(|entry| &entry.function))
        {
            if environment.contains_key(function.name) {
                return runtime.error(self.parsed.module.span, "Duplicate host function name");
            }
            environment
                .insert(function.name, Value::Host(function.clone()))
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        for (name, value) in inputs {
            runtime.host_value_size(value, self.parsed.module.span, 0)?;
            if environment.contains_key(name) {
                return runtime.error(
                    self.parsed.module.span,
                    "Invalid or duplicate input parameter",
                );
            }
            environment
                .insert(name, value.clone())
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        }
        runtime.module_globals = environment
            .try_clone()
            .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?;
        for (name, program) in modules {
            runtime.sources.insert(Some(*name), program.parsed.source);
            if runtime
                .modules
                .insert(*name, program.parsed.module.clone())
                .is_some()
            {
                return runtime.error(self.parsed.module.span, "Duplicate module registration");
            }
            runtime.contracts.insert(
                Some(*name),
                prepared
                    .interfaces
                    .get(*name)
                    .map_or_else(|| program.contracts.clone(), |i| Rc::new(i.contracts())),
            );
            runtime
                .references
                .insert(Some(*name), program.references.clone());
        }
        let (initial_value, _) =
            runtime.module_statements(&self.parsed.module.items, &mut environment)?;
        runtime.instance_roots = Some(
            environment
                .try_clone()
                .map_err(|e| runtime.environment_error(self.parsed.module.span, e))?,
        );
        let mut instance = ScriptInstance {
            runtime,
            environment,
            span: self.parsed.module.span,
            initial_value,
        };
        instance.initialize_usage();
        Ok(instance)
    }
}

/// Evaluate a module's final expression with a shared expression/call budget.
pub fn evaluate(source: &str, budget: usize) -> Result<Value<'_>> {
    Program::compile(source)?.run(budget, &CancellationToken::default(), &[])
}

struct Runtime<'a, 's> {
    module: Option<&'s str>,
    early_return: Option<Value<'s>>,
    coroutines: HashMap<CoroutineId, coroutine::Coroutine<'s>>,
    user_types: HashMap<(Option<&'s str>, &'s str), String>,
    struct_fields: HashMap<String, BTreeMap<String, ValueType>>,
    modules: HashMap<&'s str, crate::Module<'s>>,
    module_cache: memory::Slots<(&'s str, Value<'s>)>,
    loading: memory::Slots<&'s str>,
    module_globals: Environment<'s>,
    memory: memory::Budget,
    references: HashMap<Option<&'s str>, Rc<Vec<CaptureReference<'s>>>>,
    contracts: HashMap<Option<&'s str>, Rc<HashMap<usize, crate::analysis::FunctionContract>>>,
    current_references: Rc<Vec<CaptureReference<'s>>>,
    cells: memory::Slots<(Value<'s>, Option<ValueType>)>,
    released_cells: Rc<RefCell<memory::Slots<usize>>>,
    free_cells: memory::Slots<usize>,
    cell_ids: memory::Slots<Weak<CellId>>,
    cell_allocations: usize,
    cell_collection_interval: usize,
    remaining: usize,
    depth: usize,
    max_depth: usize,
    max_collection_items: usize,
    max_string_bytes: usize,
    sources: HashMap<Option<&'s str>, &'s str>,
    memory_limit: usize,
    initial_data_bytes: usize,
    instance_roots: Option<Environment<'s>>,
    environment_pool: Vec<Environment<'s>>,
    /// Shared function bodies keyed by (module, expression start), built once
    /// per program point instead of deep-cloning AST on every evaluation.
    function_bodies: HashMap<(Option<&'s str>, usize), Rc<FunctionBody<'s>>>,
    /// Active region (arena) frames, innermost last. Region id 0 is the
    /// global arena and has no frame.
    region_stack: Vec<RegionFrame>,
    /// Region id per cell slot, parallel to `cells`.
    cell_regions: Vec<u32>,
    next_region: u32,
    /// One-shot: the next `call_user` runs without an implicit call region
    /// because `arena_run` already supplied the region for it.
    skip_call_region: bool,
    /// Aggregated per-name metrics; frames reference entries by index.
    /// Lookups are linear — the number of distinct region names is tiny.
    region_stats: Vec<RegionStats>,
    cancellation: &'a CancellationToken,
    contextual: HashMap<*const HostFunction, ContextualHostCallback>,
}

impl<'s> Runtime<'_, 's> {
    fn host_value_size(&mut self, value: &Value<'s>, span: Span, depth: usize) -> Result<()> {
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
            Value::UserData(data) => {
                for item in &data.values {
                    self.host_value_size(item, span, depth + 1)?;
                }
            }
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
    fn string_growth(&self, current: usize, added: usize, span: Span) -> Result<()> {
        if current
            .checked_add(added)
            .is_none_or(|size| size > self.max_string_bytes)
        {
            return self.error(span, "String byte limit exceeded");
        }
        Ok(())
    }
    fn collection_growth(&self, current: usize, added: usize, span: Span) -> Result<()> {
        if current
            .checked_add(added)
            .is_none_or(|size| size > self.max_collection_items)
        {
            return self.error(span, "Collection item limit exceeded");
        }
        Ok(())
    }
    fn check(&self, span: Span) -> Result<()> {
        if self.cancellation.is_cancelled() {
            self.error(span, "Execution cancelled")
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, amount: usize, span: Span) -> Result<()> {
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
    fn error<T>(&self, span: Span, message: &str) -> Result<T> {
        Err(RuntimeError {
            stack: Vec::new(),
            location: None,
            module: self.module.map(str::to_owned),
            span,
            message: message.into(),
        })
    }
}
