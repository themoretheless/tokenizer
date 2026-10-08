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
    /// None denotes the current document; imported symbols identify their module.
    pub definition_module: Option<String>,
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
    inspect_inner(
        parsed,
        limit,
        calls,
        hosts,
        trailing_member,
        true,
        ModuleContext {
            exports: HashMap::new(),
            name: None,
            interface: None,
        },
    )
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
    let (references, bindings, _) = inspect_inner(
        parsed,
        limit,
        calls,
        host_functions,
        false,
        false,
        ModuleContext {
            exports: HashMap::new(),
            name: None,
            interface: None,
        },
    );
    (references, bindings)
}
pub(crate) fn editor_modules(
    parsed: &mut Parse<'_>,
    exports: ModuleInterfaces,
) -> (
    Vec<NameReference>,
    Vec<LexicalBinding>,
    Vec<MemberCompletion>,
) {
    let recover = parsed.source.trim_end().ends_with('.')
        && parsed.diagnostics.len() == 1
        && parsed.diagnostics[0].code == "expected-name";
    inspect_inner(
        parsed,
        crate::InputLimits::conservative().max_diagnostics,
        true,
        HashMap::new(),
        recover,
        true,
        ModuleContext {
            exports,
            name: None,
            interface: None,
        },
    )
}
pub(crate) fn editor_module(
    parsed: &mut Parse<'_>,
    exports: ModuleInterfaces,
    module: &str,
) -> (
    Vec<NameReference>,
    Vec<LexicalBinding>,
    Vec<MemberCompletion>,
) {
    let recover = parsed.source.trim_end().ends_with('.')
        && parsed.diagnostics.len() == 1
        && parsed.diagnostics[0].code == "expected-name";
    inspect_inner(
        parsed,
        crate::InputLimits::conservative().max_diagnostics,
        true,
        HashMap::new(),
        recover,
        true,
        ModuleContext {
            exports,
            name: Some(module),
            interface: None,
        },
    )
}
pub(crate) type FunctionContract = (Vec<Option<ValueType>>, Option<ValueType>);
pub(crate) fn compile_analysis(
    parsed: &mut Parse<'_>,
) -> (Vec<NameReference>, HashMap<usize, FunctionContract>) {
    let mut interface = ModuleInterface::default();
    let references = inspect_inner(
        parsed,
        crate::InputLimits::conservative().max_diagnostics,
        true,
        HashMap::new(),
        false,
        false,
        ModuleContext {
            exports: HashMap::new(),
            name: None,
            interface: Some(&mut interface),
        },
    )
    .0;
    let contracts = interface
        .functions
        .into_iter()
        .filter_map(|(id, shape)| {
            if let Shape::Function(parameters, result, _) = shape {
                let result = if let Shape::Typed(ty) = *result {
                    Some(ty)
                } else {
                    None
                };
                Some((id, (parameters, result)))
            } else {
                None
            }
        })
        .collect();
    (references, contracts)
}
pub(crate) fn resolved_interface(
    parsed: &mut Parse<'_>,
    exports: ModuleInterfaces,
    module: Option<&str>,
) -> ModuleInterface {
    let mut interface = ModuleInterface::default();
    inspect_inner(
        parsed,
        crate::InputLimits::conservative().max_diagnostics,
        true,
        HashMap::new(),
        false,
        false,
        ModuleContext {
            exports,
            name: module,
            interface: Some(&mut interface),
        },
    );
    interface.diagnostics = parsed.diagnostics.clone();
    interface
}
pub(crate) fn check_strict(parsed: &mut Parse<'_>) {
    check_strict_modules(parsed, HashMap::new(), None);
}
pub(crate) fn check_strict_modules(
    parsed: &mut Parse<'_>,
    exports: ModuleInterfaces,
    module: Option<&str>,
) {
    let mut interface = ModuleInterface::default();
    inspect_inner(
        parsed,
        crate::InputLimits::conservative().max_diagnostics,
        true,
        HashMap::new(),
        false,
        false,
        ModuleContext {
            exports,
            name: module,
            interface: Some(&mut interface),
        },
    );
    for (id, shape) in interface.functions {
        if let Shape::Function(parameters, result, _) = shape
            && (parameters.iter().any(Option::is_none) || !result.has_contract())
        {
            parsed.valid = false;
            parsed.diagnostics.push(Diagnostic::new(Span::new(id,id+1),"dynamic-contract","Function contract cannot be inferred completely; annotate dynamic parameters and return type"));
        }
    }
}
#[derive(Default)]
struct ModuleContext<'a> {
    exports: ModuleInterfaces,
    name: Option<&'a str>,
    interface: Option<&'a mut ModuleInterface>,
}
fn inspect_inner(
    parsed: &mut Parse<'_>,
    limit: usize,
    calls: bool,
    host_functions: HashMap<String, Rc<HostFunction>>,
    recover_trailing_member: bool,
    collect_members: bool,
    context: ModuleContext<'_>,
) -> (
    Vec<NameReference>,
    Vec<LexicalBinding>,
    Vec<MemberCompletion>,
) {
    let ModuleContext {
        exports: module_exports,
        name: module,
        interface,
    } = context;
    // Only the editor may inspect a missing final member name. Other syntax errors
    // still suppress metadata, and execution continues to reject this source.
    if !parsed.is_valid() && !recover_trailing_member {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut checker = Checker {
        module: module.map(str::to_owned),
        type_names: HashMap::new(),
        imports: std::collections::HashSet::new(),
        binding_modules: HashMap::new(),
        module_exports,
        exports: std::collections::HashSet::new(),
        scopes: vec![HashMap::new()],
        scope_spans: vec![Span::new(0, parsed.source.len() + 1)],
        bindings: Vec::new(),
        binding_types: HashMap::new(),
        binding_shapes: HashMap::new(),
        expression_shapes: HashMap::new(),
        record_fields: HashMap::new(),
        return_types: Vec::new(),
        inferred_returns: Vec::new(),
        function_shapes: HashMap::new(),
        suspending_functions: std::collections::HashSet::new(),
        user_types: HashMap::new(),
        constructor_aliases: HashMap::new(),
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
        region_depth: 0,
        region_strict: Vec::new(),
        binding_regions: HashMap::new(),
        moved: HashMap::new(),
    };
    checker.statements(&parsed.module.items);
    if let Some(interface) = interface {
        interface.types = checker.user_types.clone();
        for (&id, parameters) in &checker.function_parameters {
            let result = checker
                .function_shapes
                .get(&id)
                .cloned()
                .unwrap_or_else(|| {
                    checker
                        .function_results
                        .get(&id)
                        .cloned()
                        .map_or(Shape::Unknown, Shape::Typed)
                });
            interface.functions.insert(
                id,
                Shape::Function(
                    parameters.clone(),
                    Box::new(result),
                    checker.suspending_functions.contains(&id),
                ),
            );
        }
        for name in &checker.exports {
            if let Some(binding) = checker.scopes[0].get(name.as_str()) {
                let shape = checker.binding_shape(binding);
                interface.fields.insert(name.clone(), shape);
                interface.symbols.insert(name.clone(), binding.1);
            }
        }
    }
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
    checker
        .references
        .sort_by_key(|reference| reference.usage.start);
    checker.references.dedup();
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

pub(crate) type ModuleInterfaces = HashMap<String, Rc<ModuleInterface>>;
#[derive(Clone, Default)]
pub(crate) struct ModuleInterface {
    fields: std::collections::BTreeMap<String, Shape>,
    types: HashMap<String, Rc<UserDefinition>>,
    functions: HashMap<usize, Shape>,
    symbols: HashMap<String, Span>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}
impl ModuleInterface {
    pub(crate) fn contracts(&self) -> HashMap<usize, FunctionContract> {
        self.functions
            .iter()
            .filter_map(|(&id, shape)| {
                if let Shape::Function(parameters, result, _) = shape {
                    Some((
                        id,
                        (
                            parameters.clone(),
                            if let Shape::Typed(t) = result.as_ref() {
                                Some(t.clone())
                            } else {
                                None
                            },
                        ),
                    ))
                } else {
                    None
                }
            })
            .collect()
    }
}
/// Build interfaces in dependency order; the runtime separately diagnoses missing imports/cycles.
pub(crate) fn module_interfaces(modules: &[(&str, &crate::Program<'_>)]) -> ModuleInterfaces {
    let mut output = HashMap::new();
    let registered: std::collections::HashSet<_> = modules.iter().map(|(name, _)| *name).collect();
    for _ in 0..modules.len() {
        let mut progress = false;
        for (name, program) in modules {
            if output.contains_key(*name)
                || program
                    .import_refs()
                    .iter()
                    .any(|i| registered.contains(i.text) && !output.contains_key(i.text))
            {
                continue;
            }
            let mut parsed = program.parsed_clone();
            let mut interface = ModuleInterface::default();
            inspect_inner(
                &mut parsed,
                crate::InputLimits::conservative().max_diagnostics,
                true,
                HashMap::new(),
                false,
                false,
                ModuleContext {
                    exports: output.clone(),
                    name: Some(name),
                    interface: Some(&mut interface),
                },
            );
            interface.diagnostics = parsed.diagnostics.clone();
            output.insert(name.to_string(), Rc::new(interface));
            progress = true;
        }
        if !progress {
            break;
        }
    }
    output
}
#[derive(Clone)]
struct UserDefinition {
    span: Span,
    module: Option<String>,
    field_spans: HashMap<String, Span>,
    variant_spans: HashMap<String, Span>,
    fields: Option<std::collections::BTreeMap<String, ValueType>>,
    variants: std::collections::BTreeMap<String, Vec<ValueType>>,
}
struct Checker<'s> {
    module: Option<String>,
    type_names: HashMap<String, String>,
    imports: std::collections::HashSet<String>,
    binding_modules: HashMap<usize, String>,
    module_exports: ModuleInterfaces,
    exports: std::collections::HashSet<String>,
    user_types: HashMap<String, Rc<UserDefinition>>,
    constructor_aliases: HashMap<usize, (String, Option<String>)>,
    member_completions: Vec<MemberCompletion>,
    collect_members: bool,
    host_functions: HashMap<String, Rc<HostFunction>>,
    calls: bool,
    scopes: Vec<HashMap<&'s str, Binding>>,
    scope_spans: Vec<Span>,
    bindings: Vec<LexicalBinding>,
    binding_types: HashMap<usize, ValueType>,
    binding_shapes: HashMap<usize, Shape>,
    expression_shapes: HashMap<(usize, usize), Shape>,
    record_fields: HashMap<usize, Vec<String>>,
    return_types: Vec<Option<ValueType>>,
    inferred_returns: Vec<Vec<Shape>>,
    function_shapes: HashMap<usize, Shape>,
    suspending_functions: std::collections::HashSet<usize>,
    function_results: HashMap<usize, ValueType>,
    function_parameters: HashMap<usize, Vec<Option<ValueType>>>,
    builtin_aliases: HashMap<usize, crate::Builtin>,
    references: Vec<NameReference>,
    diagnostics: Vec<Diagnostic>,
    valid: bool,
    limit: usize,
    region_depth: usize,
    region_strict: Vec<bool>,
    binding_regions: HashMap<usize, usize>,
    /// Bindings emptied by `move(...)`; a read is an error until reassigned.
    moved: HashMap<usize, Span>,
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
                StmtKind::Region { body, .. } => Self::inspect(&body.stmts),
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
    /// Non-fatal lint: the program stays valid and runs, but the host UI
    /// surfaces the suspicious pattern.
    fn warn(&mut self, span: Span, code: &'static str, message: &'static str) {
        if self.diagnostics.len() < self.limit {
            self.diagnostics.push(Diagnostic::new(span, code, message));
        }
    }
    /// Deepest region level among recorded name references inside `span` that
    /// resolve to bindings declared deeper than `level`; `None` when nothing
    /// escapes.
    fn escape_level(&self, span: Span, level: usize) -> Option<usize> {
        self.references
            .iter()
            .filter(|reference| {
                reference.usage.start >= span.start && reference.usage.end <= span.end
            })
            .filter_map(|reference| {
                let definition = reference.definition?;
                let binding_level = self.binding_regions.get(&definition.start).copied()?;
                (binding_level > level).then_some(binding_level)
            })
            .max()
    }
    /// Explicit `promote(value)` intent: a single-argument call to the
    /// builtin, not shadowed by a user binding.
    fn is_promote_call(&self, expression: &Expr<'s>) -> bool {
        self.is_unshadowed_call(expression, "promote")
    }
    /// Explicit `move(binding)`: takes the value out of a mutable cell,
    /// leaving it empty. Also an explicit relocation intent for escapes.
    fn is_move_call(&self, expression: &Expr<'s>) -> bool {
        self.is_unshadowed_call(expression, "move")
    }
    fn is_unshadowed_call(&self, expression: &Expr<'s>, name: &str) -> bool {
        let ExprKind::Call { callee, arguments } = &expression.kind else {
            return false;
        };
        arguments.len() == 1
            && matches!(&callee.kind, ExprKind::Name(callee_name)
                if callee_name.text == name
                    && !self.scopes.iter().any(|scope| scope.contains_key(callee_name.text)))
    }
    /// Check a `move(...)` call: single mutable-binding argument, not moved
    /// before; marks the binding as emptied for subsequent reads.
    fn move_call(&mut self, span: Span, arguments: &[Expr<'s>]) {
        let [target] = arguments else {
            self.error(span, "move-arity", "move requires exactly one argument");
            return;
        };
        let ExprKind::Name(name) = &target.kind else {
            self.error(
                target.span,
                "move-target",
                "move requires a mutable binding name",
            );
            self.expr(target);
            return;
        };
        let resolved = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name.text))
            .map(|binding| (binding.0, binding.1));
        match resolved {
            None => self.expr(target),
            Some((constant, definition)) => {
                if constant {
                    self.error(
                        name.span,
                        "move-immutable",
                        "move requires a mutable binding",
                    );
                } else if self.moved.insert(definition.start, name.span).is_some() {
                    self.error(
                        name.span,
                        "moved-value",
                        "Binding was already moved out of its cell",
                    );
                }
                self.references.push(NameReference {
                    definition_module: None,
                    usage: name.span,
                    definition: Some(definition),
                });
            }
        }
    }
    /// Reports an implicit region escape: a hard error when any crossed
    /// region is strict, a non-fatal lint otherwise. Values whose shape
    /// cannot carry captured cells (scalar copies, scalar collections) move
    /// freely and are never reported.
    fn check_escape(&mut self, value: &Expr<'s>, target_level: usize, context: &'static str) {
        if !self.shape(value).can_carry_cells() {
            return;
        }
        let span = value.span;
        let Some(escaped) = self.escape_level(span, target_level) else {
            return;
        };
        if self
            .region_strict
            .get(target_level..escaped)
            .is_some_and(|crossed| crossed.iter().any(|strict| *strict))
        {
            self.error(
                span,
                "region-escape",
                "Value allocated in a strict region escapes; wrap it in promote(...) to allow the escape",
            );
        } else {
            self.warn(span, "region-escape", context);
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
            self.binding_regions
                .insert(name.span.start, self.region_depth);
            if self.collect_members {
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
    }
    fn identity(&self, name: &str) -> String {
        format!("{}::{}", self.module.as_deref().unwrap_or("<entry>"), name)
    }
    fn resolve_type(&self, name: &str) -> Option<String> {
        let normalized = name.split('.').map(str::trim).collect::<Vec<_>>().join(".");
        let name = normalized.as_str();
        self.type_names.get(name).cloned().or_else(|| {
            let (module, ty) = name.split_once('.')?;
            // Standalone compilation defers imported contracts to graph validation.
            (self.imports.contains(module) && !self.module_exports.contains_key(module))
                .then(|| format!("{module}::{ty}"))
        })
    }
    fn binding_shape(&self, binding: &Binding) -> Shape {
        let id = binding.1.start;
        if let Some((name, variant)) = self.constructor_aliases.get(&id) {
            return Shape::Constructor(name.clone(), variant.clone());
        }
        if let Some(parameters) = self.function_parameters.get(&id) {
            return Shape::Function(
                parameters.clone(),
                Box::new(self.function_shapes.get(&id).cloned().unwrap_or_else(|| {
                    self.function_results
                        .get(&id)
                        .cloned()
                        .map_or(Shape::Unknown, Shape::Typed)
                })),
                self.suspending_functions.contains(&id),
            );
        }
        self.binding_shapes.get(&id).cloned().unwrap_or_else(|| {
            self.binding_types
                .get(&id)
                .cloned()
                .map_or(Shape::Unknown, Shape::Typed)
        })
    }
    fn constrain(
        &mut self,
        expression: &Expr<'s>,
        expected: Option<ValueType>,
        parameters: &std::collections::HashSet<usize>,
    ) {
        match &expression.kind {
            ExprKind::Name(name) => {
                if let Some(binding) = self.scopes.iter().rev().find_map(|s| s.get(name.text))
                    && parameters.contains(&binding.1.start)
                    && let Some(expected) = expected
                {
                    let id = binding.1.start;
                    if let Some(actual) = self.binding_types.get(&id) {
                        if actual != &expected {
                            self.error(
                                name.span,
                                "parameter-type",
                                "Parameter has incompatible type requirements; add an annotation",
                            );
                        }
                    } else {
                        self.binding_types.insert(id, expected);
                    }
                }
            }
            ExprKind::Binary {
                left,
                right,
                operator,
            } => {
                let l = self.shape(left);
                let r = self.shape(right);
                let boolean = matches!(*operator, "and" | "or" | "&&" | "||");
                let comparable = matches!(*operator, "+" | "-" | "%" | "<" | ">" | "<=" | ">=");
                let context = if boolean {
                    Some(ValueType::Bool)
                } else if matches!(*operator, "+" | "-" | "*" | "/" | "%") {
                    expected
                } else {
                    None
                };
                let inferred = |shape: &Shape| {
                    if let Shape::Typed(ty @ (ValueType::Number | ValueType::String)) = shape {
                        Some(ty.clone())
                    } else {
                        None
                    }
                };
                let mut lc = context
                    .clone()
                    .or_else(|| comparable.then(|| inferred(&r)).flatten());
                let mut rc = context.or_else(|| comparable.then(|| inferred(&l)).flatten());
                if matches!(*operator, "*" | "/") {
                    if matches!(l, Shape::Typed(ValueType::Vector(_) | ValueType::Angle)) {
                        rc = Some(ValueType::Number);
                    }
                    if *operator == "*"
                        && matches!(r, Shape::Typed(ValueType::Vector(_) | ValueType::Angle))
                    {
                        lc = Some(ValueType::Number);
                    }
                }
                self.constrain(left, lc, parameters);
                self.constrain(right, rc, parameters);
            }
            ExprKind::Unary { operator, value } => self.constrain(
                value,
                if matches!(*operator, "not" | "!") {
                    Some(ValueType::Bool)
                } else {
                    expected
                },
                parameters,
            ),
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                self.constrain(condition, Some(ValueType::Bool), parameters);
                self.constrain(then_value, expected.clone(), parameters);
                self.constrain(else_value, expected, parameters);
            }
            ExprKind::Call { callee, arguments } => {
                let types = self.known_user_parameters(callee).or_else(|| {
                    self.known_host(callee)
                        .map(|h| h.parameters.iter().cloned().map(Some).collect())
                });
                for (i, arg) in arguments.iter().enumerate() {
                    self.constrain(
                        arg,
                        types.as_ref().and_then(|t| t.get(i)).cloned().flatten(),
                        parameters,
                    );
                }
            }
            _ => {}
        }
    }
    fn constrain_body(
        &mut self,
        statements: &[Stmt<'s>],
        expected: Option<ValueType>,
        parameters: &std::collections::HashSet<usize>,
    ) {
        self.scopes.push(HashMap::new());
        for statement in statements {
            match &statement.kind {
                StmtKind::Return(Some(value)) => {
                    self.constrain(value, expected.clone(), parameters)
                }
                StmtKind::Show(value) | StmtKind::Expr(value) | StmtKind::Yield(value) => {
                    self.constrain(value, None, parameters)
                }
                StmtKind::Declaration {
                    role: _,
                    name,
                    constant,
                    value,
                    ty,
                } => {
                    let ty = ty.as_ref().and_then(|t| {
                        ValueType::annotation_with(t, &|n| self.resolve_type(n)).ok()
                    });
                    self.constrain(value, ty.clone(), parameters);
                    let shape = self.shape(value);
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(name.text, (*constant, name.span, None, None));
                    if let Some(ty) = ty.or_else(|| {
                        if let Shape::Typed(t) = &shape {
                            Some(t.clone())
                        } else {
                            None
                        }
                    }) {
                        self.binding_types.insert(name.span.start, ty);
                    }
                    if *constant {
                        self.binding_shapes.insert(name.span.start, shape);
                    }
                }
                StmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.constrain(condition, Some(ValueType::Bool), parameters);
                    self.constrain_body(&then_block.stmts, expected.clone(), parameters);
                    if let Some(block) = else_block {
                        self.constrain_body(&block.stmts, expected.clone(), parameters);
                    }
                }
                StmtKind::While { condition, body } => {
                    self.constrain(condition, Some(ValueType::Bool), parameters);
                    self.constrain_body(&body.stmts, expected.clone(), parameters);
                }
                StmtKind::For { binding, body, .. } => {
                    self.scopes.push(HashMap::from([(
                        binding.text,
                        (true, binding.span, None, None),
                    )]));
                    self.constrain_body(&body.stmts, expected.clone(), parameters);
                    self.scopes.pop();
                }
                StmtKind::Region { body, .. } => {
                    self.constrain_body(&body.stmts, expected.clone(), parameters);
                }
                _ => {}
            }
        }
        self.scopes.pop();
    }
    fn return_candidates(&self, statements: &[Stmt<'s>], output: &mut Vec<Shape>) {
        for statement in statements {
            match &statement.kind {
                StmtKind::Return(value) => output.push(
                    value
                        .as_ref()
                        .map_or(Shape::Typed(ValueType::Null), |v| self.shape(v)),
                ),
                StmtKind::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.return_candidates(&then_block.stmts, output);
                    if let Some(block) = else_block {
                        self.return_candidates(&block.stmts, output);
                    }
                }
                StmtKind::While { body, .. } | StmtKind::For { body, .. } => {
                    self.return_candidates(&body.stmts, output)
                }
                StmtKind::Region { body, .. } => self.return_candidates(&body.stmts, output),
                _ => {}
            }
        }
    }
    fn require_contract(&mut self, shape: &Shape, span: Span) {
        if let Shape::Function(parameters, result, _) = shape
            && (parameters.iter().any(Option::is_none) || !result.has_contract())
        {
            self.error(span, "dynamic-contract", "Function contract cannot be inferred completely; annotate dynamic parameters and return type");
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
    fn contains_yield(statements: &[Stmt<'s>]) -> bool {
        statements.iter().any(|statement| match &statement.kind {
            StmtKind::Yield(_) => true,
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
    fn known_suspending(&self, value: &Expr<'s>) -> bool {
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
    fn declare_user_type(&mut self, name: &Name<'s>) {
        if self.scopes.len() != 1 {
            self.error(
                name.span,
                "nested-type",
                "User types must be declared at module scope",
            );
        }
        let reserved = crate::builtin_catalog()
            .iter()
            .any(|(n, _)| *n == name.text)
            || matches!(name.text, "Option" | "Result" | "list" | "tuple")
            || ValueType::annotation(&crate::Type {
                name: name.clone(),
                path: vec![name.clone()],
                arguments: Vec::new(),
            })
            .is_ok();
        if reserved {
            self.error(
                name.span,
                "reserved-type",
                "Type name is reserved by the language",
            );
        }
        self.declare(name, true, name.span.end);
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
    }
    fn user_constructor(&self, expression: &Expr<'s>) -> Option<(String, Option<String>)> {
        if let Shape::Constructor(name, variant) = self.shape(expression) {
            Some((name, variant))
        } else {
            None
        }
    }
    fn struct_fields(&self, shape: &Shape) -> Option<std::collections::BTreeMap<String, Shape>> {
        let Shape::Typed(ValueType::User(name)) = shape else {
            return None;
        };
        Some(
            self.user_types
                .get(name)?
                .fields
                .as_ref()?
                .iter()
                .map(|(name, ty)| (name.clone(), Shape::Typed(ty.clone())))
                .collect(),
        )
    }
    fn statement(&mut self, statement: &Stmt<'s>) {
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
                let mut shapes = std::collections::BTreeMap::new();
                let mut contracts = std::collections::BTreeMap::new();
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
                self.declare(name, true, statement.span.end);
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
            } => {
                if Self::contains_yield(&body.stmts) {
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
                let parameter_types = parameters
                    .iter()
                    .map(|parameter| {
                        parameter.ty.as_ref().and_then(|ty| {
                            ValueType::annotation_with(ty, &|name| self.resolve_type(name)).ok()
                        })
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
                let parameter_ids: std::collections::HashSet<_> = self
                    .scopes
                    .last()
                    .unwrap()
                    .values()
                    .map(|b| b.1.start)
                    .collect();
                let mut seed = result_type.clone();
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
    fn irrefutable(pattern: &Expr<'s>) -> bool {
        match &pattern.kind {
            ExprKind::Name(_) => true,
            ExprKind::Tuple(items) => items.iter().all(Self::irrefutable),
            ExprKind::Map(fields) => fields.iter().all(|(_, value)| Self::irrefutable(value)),
            _ => false,
        }
    }
    fn check_exhaustiveness(&mut self, value: &Expr<'s>, arms: &[crate::MatchArm<'s>]) {
        if !self.calls {
            return;
        }
        let shape = self.shape(value);
        if let Some(fields) = self.struct_fields(&shape) {
            let covered =
                arms.iter()
                    .filter(|arm| arm.guard.is_none())
                    .any(|arm| {
                        match &arm.pattern.kind {
                    ExprKind::Name(_) => true,
                    ExprKind::Map(patterns) => patterns.iter().all(|(key, pattern)| {
                        matches!(&key.kind, ExprKind::Name(name) if fields.contains_key(name.text))
                            && Self::irrefutable(pattern)
                    }),
                    _ => false,
                }
                    });
            if !covered {
                self.error(
                    value.span,
                    "non-exhaustive-match",
                    "Struct match requires an irrefutable pattern or fallback",
                );
            }
            return;
        }
        let required = match &shape {
            Shape::Typed(ValueType::Bool) => vec!["true".to_owned(), "false".to_owned()],
            Shape::Typed(ValueType::User(name)) => self
                .user_types
                .get(name)
                .filter(|d| d.fields.is_none())
                .map(|d| d.variants.keys().cloned().collect())
                .unwrap_or_default(),
            Shape::Typed(ValueType::Option(_)) => vec!["Some".into(), "None".into()],
            Shape::Typed(ValueType::Result(_, _)) => vec!["Ok".into(), "Err".into()],
            _ => return,
        };
        if required.is_empty() {
            return;
        }
        let mut covered = std::collections::HashSet::new();
        for arm in arms.iter().filter(|arm| arm.guard.is_none()) {
            match &arm.pattern.kind {
                ExprKind::Name(_) => return,
                ExprKind::Bool(value) => {
                    covered.insert(value.to_string());
                }
                ExprKind::Call { callee, arguments } if arguments.iter().all(Self::irrefutable) => {
                    if let Some((name, Some(variant))) = self.user_constructor(callee) {
                        if shape == Shape::Typed(ValueType::User(name)) {
                            covered.insert(variant);
                        }
                    } else if let ExprKind::Name(name) = &callee.kind {
                        covered.insert(name.text.to_owned());
                    }
                }
                _ => {}
            }
        }
        if required.iter().any(|variant| !covered.contains(variant)) {
            self.error(value.span, "non-exhaustive-match", "Match must cover every variant; guarded or refutable payload patterns require a fallback");
        }
    }
    fn match_pattern_types(&mut self, pattern: &Expr<'s>, shape: &Shape) {
        if self.collect_members
            && let ExprKind::Call { callee, .. } = &pattern.kind
        {
            self.expr(callee);
        }
        if self.collect_members
            && let (Shape::Typed(ValueType::User(id)), ExprKind::Map(fields)) =
                (shape, &pattern.kind)
            && let Some(definition) = self.user_types.get(id)
        {
            for (key, _) in fields {
                if let ExprKind::Name(field) = &key.kind
                    && let Some(span) = definition.field_spans.get(field.text)
                {
                    self.references.push(NameReference {
                        usage: field.span,
                        definition: Some(*span),
                        definition_module: definition
                            .module
                            .clone()
                            .filter(|m| self.module.as_ref() != Some(m)),
                    });
                }
            }
        }
        if self.calls
            && matches!(shape, Shape::Typed(_))
            && matches!(pattern.kind, ExprKind::Tuple(_) | ExprKind::Map(_))
        {
            self.check_pattern_shape(pattern, shape);
        }
        match &pattern.kind {
            ExprKind::Name(_) => {
                if let Shape::Typed(ty) = shape {
                    self.pattern_type(pattern, ty);
                }
            }
            ExprKind::Call { callee, arguments } => {
                if let Some((name, Some(variant))) = self.user_constructor(callee) {
                    if shape.rejects(&ValueType::User(name.clone())) {
                        self.error(
                            pattern.span,
                            "pattern-type",
                            "Pattern variant belongs to another type",
                        );
                    }
                    let types = self.user_types[&name].variants[&variant].clone();
                    if arguments.len() != types.len() {
                        self.error(
                            pattern.span,
                            "pattern-arity",
                            "Variant payload count does not match declaration",
                        );
                    }
                    for (pattern, ty) in arguments.iter().zip(types) {
                        self.match_pattern_types(pattern, &Shape::Typed(ty));
                    }
                } else if matches!(&callee.kind, ExprKind::Member { .. })
                    && !self.unresolved_import(callee)
                {
                    self.error(
                        callee.span,
                        "unknown-variant",
                        "Unknown enum variant pattern",
                    );
                } else if let (ExprKind::Name(name), Shape::Typed(ty)) = (&callee.kind, shape) {
                    let payload = match (name.text, ty) {
                        ("Some", ValueType::Option(ty))
                        | ("Ok", ValueType::Result(ty, _))
                        | ("Err", ValueType::Result(_, ty)) => Some(ty.as_ref().clone()),
                        ("None", ValueType::Option(_)) => None,
                        _ => {
                            if self.calls {
                                self.error(
                                    pattern.span,
                                    "pattern-type",
                                    "Variant pattern does not match value type",
                                );
                            }
                            None
                        }
                    };
                    if let (Some(ty), Some(pattern)) = (payload, arguments.first()) {
                        self.match_pattern_types(pattern, &Shape::Typed(ty));
                    }
                }
                self.expr(callee);
            }
            ExprKind::Tuple(patterns) => {
                if let Shape::Typed(ValueType::Tuple(types)) = shape {
                    for (pattern, ty) in patterns.iter().zip(types) {
                        self.match_pattern_types(pattern, &Shape::Typed(ty.clone()));
                    }
                }
            }
            ExprKind::Map(patterns) => {
                if let Some(fields) = self.struct_fields(shape) {
                    for (key, pattern) in patterns {
                        if let ExprKind::Name(name) = &key.kind {
                            if let Some(shape) = fields.get(name.text) {
                                self.match_pattern_types(pattern, shape);
                            } else if self.calls {
                                self.error(
                                    name.span,
                                    "unknown-struct-field",
                                    "Unknown struct field in pattern",
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
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
    fn annotation_references(&mut self, ty: &crate::Type<'s>) {
        let qualified = ty.qualified_name();
        if let Some(id) = self.resolve_type(&qualified)
            && let Some(definition) = self.user_types.get(&id)
        {
            let start = ty
                .path
                .last()
                .map_or(ty.name.span.start, |part| part.span.start);
            let module = definition
                .module
                .clone()
                .filter(|m| self.module.as_ref() != Some(m));
            // A qualified annotation refers to the public alias, which may reexport
            // a differently named nominal type from another module.
            let public = qualified.split_once('.').and_then(|(m, n)| {
                self.module_exports
                    .get(m.trim())
                    .and_then(|i| i.symbols.get(n.trim()))
                    .map(|span| (m.trim().to_owned(), *span))
            });
            let (span, module) =
                public.map_or((definition.span, module), |(m, span)| (span, Some(m)));
            self.references.push(NameReference {
                usage: Span::new(start, ty.name.span.end),
                definition: Some(span),
                definition_module: module,
            });
        }
        for argument in &ty.arguments {
            self.annotation_references(argument);
        }
    }
    fn unresolved_import(&self, expression: &Expr<'s>) -> bool {
        match &expression.kind {
            ExprKind::Name(name) => {
                self.imports.contains(name.text) && !self.module_exports.contains_key(name.text)
            }
            ExprKind::Member { object, .. } => self.unresolved_import(object),
            _ => false,
        }
    }
    fn expression_module(&self, expression: &Expr<'s>) -> Option<String> {
        let ExprKind::Name(name) = &expression.kind else {
            return None;
        };
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name.text))
            .and_then(|b| self.binding_modules.get(&b.1.start))
            .cloned()
    }
    fn member_reference(&mut self, object: &Expr<'s>, field: &Name<'s>) {
        if !self.collect_members {
            return;
        }
        if let Some(module) = self.expression_module(object)
            && let Some(span) = self
                .module_exports
                .get(&module)
                .and_then(|i| i.symbols.get(field.text))
        {
            self.references.push(NameReference {
                usage: field.span,
                definition: Some(*span),
                definition_module: Some(module),
            });
            return;
        }
        let shape = self.shape(object);
        let id = match &shape {
            Shape::Typed(ValueType::User(id)) => Some(id.clone()),
            Shape::Record(fields) => fields.values().find_map(|s| {
                if let Shape::Constructor(id, _) = s {
                    Some(id.clone())
                } else {
                    None
                }
            }),
            _ => None,
        };
        if let Some(id) = id
            && let Some(definition) = self.user_types.get(&id)
            && let Some(span) = definition
                .field_spans
                .get(field.text)
                .or_else(|| definition.variant_spans.get(field.text))
        {
            self.references.push(NameReference {
                usage: field.span,
                definition: Some(*span),
                definition_module: definition
                    .module
                    .clone()
                    .filter(|m| self.module.as_ref() != Some(m)),
            });
        }
    }
    fn checked_annotation(&mut self, ty: &crate::Type<'s>) -> Option<ValueType> {
        if self.collect_members {
            self.annotation_references(ty);
        }
        match ValueType::annotation_with(ty, &|name| self.resolve_type(name)) {
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
        if self.collect_members
            && let (Shape::Typed(ValueType::User(id)), ExprKind::Map(fields)) =
                (shape, &pattern.kind)
            && let Some(definition) = self.user_types.get(id)
        {
            for (key, _) in fields {
                if let ExprKind::Name(field) = &key.kind
                    && let Some(span) = definition.field_spans.get(field.text)
                {
                    self.references.push(NameReference {
                        usage: field.span,
                        definition: Some(*span),
                        definition_module: definition
                            .module
                            .clone()
                            .filter(|m| self.module.as_ref() != Some(m)),
                    });
                }
            }
        }
        if let Some(fields) = self.struct_fields(shape) {
            self.check_pattern_shape(pattern, &Shape::Record(fields));
            return;
        }
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
                Shape::Unknown
                | Shape::Unsupported
                | Shape::Alternatives(_)
                | Shape::Constructor(..)
                | Shape::Function(..) => {}
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
        if let Some(fields) = self.struct_fields(shape) {
            self.pattern_shape_signatures(pattern, &Shape::Record(fields), signatures);
            return;
        }
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
        if let Shape::Typed(ValueType::Function(parameters, _)) = self.shape(expression) {
            let n = parameters.len();
            return Some(std::iter::once(n..=n).collect());
        }
        if let Shape::Function(parameters, _, _) = self.shape(expression) {
            let n = parameters.len();
            return Some(std::iter::once(n..=n).collect());
        }
        if let Some((name, variant)) = self.user_constructor(expression) {
            let count = variant
                .as_ref()
                .map_or(1, |variant| self.user_types[&name].variants[variant].len());
            return Some(std::iter::once(count..=count).collect());
        }
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
        let shape = self.shape(expression);
        if let Some(fields) = self.struct_fields(&shape) {
            return Some(fields.into_keys().collect());
        }
        match shape {
            Shape::Record(fields) => Some(fields.into_keys().collect()),
            _ => None,
        }
    }
    fn known_user_parameters(&self, expression: &Expr<'s>) -> Option<Vec<Option<ValueType>>> {
        if let Shape::Typed(ValueType::Function(parameters, _)) = self.shape(expression) {
            return Some(parameters.into_iter().map(Some).collect());
        }
        if let Shape::Function(parameters, _, _) = self.shape(expression) {
            return Some(parameters);
        }
        if let Some((name, variant)) = self.user_constructor(expression) {
            return Some(variant.map_or_else(
                || vec![None],
                |variant| {
                    self.user_types[&name].variants[&variant]
                        .iter()
                        .cloned()
                        .map(Some)
                        .collect()
                },
            ));
        }
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
        if let Shape::Typed(ValueType::Function(_, result)) = self.shape(callee) {
            return Shape::Typed(*result);
        }
        if let Shape::Function(_, result, _) = self.shape(callee) {
            return *result;
        }
        if let Some((name, _)) = self.user_constructor(callee) {
            return Shape::Typed(ValueType::User(name));
        }
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
                .function_shapes
                .get(&binding.1.start)
                .cloned()
                .unwrap_or_else(|| {
                    self.function_results
                        .get(&binding.1.start)
                        .map_or(Shape::Unknown, |ty| Shape::Typed(ty.clone()))
                });
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
        if let Some(shape) = self
            .expression_shapes
            .get(&(expression.span.start, expression.span.end))
        {
            return shape.clone();
        }
        match &expression.kind {
            ExprKind::Try(value) => match self.shape(value) {
                Shape::Typed(ValueType::Option(ty) | ValueType::Result(ty, _)) => Shape::Typed(*ty),
                _ => Shape::Unknown,
            },
            ExprKind::Name(name) => self
                .scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name.text))
                .map_or(Shape::Unknown, |binding| self.binding_shape(binding)),
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
                if let Some(fields) = self.struct_fields(&self.shape(object)) {
                    return fields.get(field.text).cloned().unwrap_or(Shape::Unknown);
                }
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
                    Shape::Alternatives(vec![left, self.shape(else_value)])
                }
            }
            _ => Shape::Unknown,
        }
    }
    fn known_non_callable(&self, value: &Expr<'s>) -> bool {
        fn data(shape: &Shape) -> bool {
            match shape {
                Shape::Alternatives(shapes) => shapes.iter().any(data),
                Shape::Typed(ValueType::Function(..)) => false,
                Shape::Typed(_) | Shape::List(_) | Shape::Tuple(_) | Shape::Record(_) => true,
                _ => false,
            }
        }
        matches!(value.kind, ExprKind::Map(_)) || data(&self.shape(value))
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
        for argument in arguments {
            self.expr(argument);
        }
        if self.calls && self.known_suspending(callee) {
            self.error(
                callee.span,
                "coroutine-call",
                "Suspending functions require spawn_coroutine instead of a synchronous call",
            );
        }
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
        if self.calls
            && let Some((name, None)) = self.user_constructor(callee)
        {
            if self.collect_members
                && let Some(Expr {
                    kind: ExprKind::Map(fields),
                    ..
                }) = arguments.first()
                && let Some(definition) = self.user_types.get(&name)
            {
                for (key, _) in fields {
                    if let ExprKind::Name(field) = &key.kind
                        && let Some(span) = definition.field_spans.get(field.text)
                    {
                        self.references.push(NameReference {
                            usage: field.span,
                            definition: Some(*span),
                            definition_module: definition
                                .module
                                .clone()
                                .filter(|m| self.module.as_ref() != Some(m)),
                        });
                    }
                }
            }

            let supplied = implicit
                .as_ref()
                .map(|(_, shape)| shape.clone())
                .or_else(|| arguments.first().map(|arg| self.shape(arg)));
            if let Some(shape) = supplied {
                if let Shape::Record(fields) = shape {
                    let expected = self.user_types[&name].fields.clone().unwrap_or_default();
                    if fields.keys().ne(expected.keys()) {
                        self.error(
                            callee.span,
                            "struct-fields",
                            "Struct fields must exactly match declaration",
                        );
                    }
                    for (field, ty) in expected {
                        if fields.get(&field).is_some_and(|shape| shape.rejects(&ty)) {
                            self.error(
                                callee.span,
                                "struct-field-type",
                                "Struct field type does not match declaration",
                            );
                        }
                    }
                } else if !matches!(shape, Shape::Unknown) {
                    self.error(
                        callee.span,
                        "struct-fields",
                        "Struct constructor requires a record",
                    );
                }
            }
        }
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
                    && self.known_suspending(callback)
                {
                    self.error(
                        callback.span,
                        "coroutine-call",
                        "Synchronous callbacks cannot suspend",
                    );
                }

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
            ExprKind::Try(value) => {
                if self.calls {
                    let operand = self.shape(value);
                    let result = self.return_types.last().cloned().flatten();
                    match (&operand, result.as_ref()) {
                        (Shape::Typed(ValueType::Option(_)), Some(ValueType::Option(_))) => {},
                        (Shape::Typed(ValueType::Result(_, error)), Some(ValueType::Result(_, expected))) if !disjoint(error, expected) => {},
                        (Shape::Unknown, Some(ValueType::Option(_) | ValueType::Result(_, _))) => {},
                        _ => self.error(expr.span, "try-type", "? requires Option or Result compatible with the enclosing function return type"),
                    }
                }
                self.expr(value);
            }
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
                self.return_types.push(None);
                self.inferred_returns.push(Vec::new());
                self.scopes.push(HashMap::new());
                self.scope_spans.push(body.span);
                for parameter in parameters {
                    self.pattern(parameter, body.span.start);
                }
                self.expr(body);
                self.return_types.pop();
                self.inferred_returns.pop();
                self.scopes.pop();
                self.scope_spans.pop();
            }
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                if self.calls
                    && *operator == "="
                    && let ExprKind::Name(name) = &target.kind
                    && let Some(expected) = self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                        .and_then(|binding| self.binding_types.get(&binding.1.start))
                    && self.shape(value).rejects(expected)
                {
                    self.error(
                        value.span,
                        "assignment-type",
                        "Assigned value type does not match binding type",
                    );
                }
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
                if *operator != "=" {
                    let operation = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        _ => "",
                    };
                    let binary = Expr {
                        span: expr.span,
                        kind: ExprKind::Binary {
                            operator: operation,
                            left: target.clone(),
                            right: value.clone(),
                        },
                    };
                    if self.calls
                        && let ExprKind::Name(name) = &target.kind
                        && let Some(expected) = self
                            .scopes
                            .iter()
                            .rev()
                            .find_map(|scope| scope.get(name.text))
                            .and_then(|binding| self.binding_types.get(&binding.1.start))
                        && self.shape(&binary).rejects(expected)
                    {
                        self.error(
                            expr.span,
                            "assignment-type",
                            "Compound assignment changes binding type",
                        );
                    }
                    self.expr(&binary);
                } else {
                    // Plain reassignment revives a moved binding's cell.
                    if let ExprKind::Name(name) = &target.kind {
                        let id = self
                            .scopes
                            .iter()
                            .rev()
                            .find_map(|scope| scope.get(name.text))
                            .map(|binding| binding.1.start);
                        if let Some(id) = id {
                            self.moved.remove(&id);
                        }
                    }
                    self.expr(target);
                    self.expr(value);
                }
                // Compound assignment writes a freshly computed value into an
                // existing outer cell; no region data or cell can flow out.
                if self.region_depth > 0
                    && *operator == "="
                    && let Some(name) = lvalue_root(target)
                    && let Some(binding) = self
                        .scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(name.text))
                {
                    let target_level = self
                        .binding_regions
                        .get(&binding.1.start)
                        .copied()
                        .unwrap_or(0);
                    if self.region_depth > target_level
                        && !self.is_promote_call(value)
                        && !self.is_move_call(value)
                    {
                        self.check_escape(
                            value,
                            target_level,
                            "Assigned value may capture cells allocated in an inner region; they will be promoted to the target binding's region",
                        );
                    }
                }
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
                    let specialized = |shape: &Shape| {
                        matches!(
                            shape,
                            Shape::Typed(
                                ValueType::Vector(_)
                                    | ValueType::Matrix4
                                    | ValueType::Quaternion
                                    | ValueType::Angle
                            )
                        )
                    };
                    if !specialized(&a) && !specialized(&b) {
                        let valid = !a.rejects(&ValueType::Number)
                            && !b.rejects(&ValueType::Number)
                            || (*operator == "+"
                                && !a.rejects(&ValueType::String)
                                && !b.rejects(&ValueType::String));
                        if !valid {
                            self.error(
                                expr.span,
                                "arithmetic-operands",
                                "Arithmetic requires numbers or compatible operands",
                            );
                        }
                    }
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
                if matches!(&callee.kind, ExprKind::Name(name) if name.text == "move")
                    && !self.scopes.iter().any(|scope| scope.contains_key("move"))
                {
                    self.move_call(expr.span, arguments);
                } else {
                    self.call(callee, arguments, None);
                }
            }
            ExprKind::Member { object, field } => {
                self.member_reference(object, field);
                let check_member = self.calls && !field.text.is_empty();
                if check_member {
                    let unsupported = match self.shape(object) {
                        Shape::Typed(ty) => !matches!(
                            ty,
                            ValueType::Vector(_) | ValueType::Mesh | ValueType::User(_)
                        ),
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
                    let object_shape = self.shape(object);
                    let members = if let Some(fields) = self.struct_fields(&object_shape) {
                        fields.into_keys().collect()
                    } else {
                        match object_shape {
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
                        }
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
                self.check_exhaustiveness(value, arms);
                let shape = self.shape(value);
                let mut results = Vec::new();
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.scope_spans.push(arm.span);
                    self.match_bindings(&arm.pattern, arm.pattern.span.end);
                    self.match_pattern_types(&arm.pattern, &shape);
                    if let Some(guard) = &arm.guard {
                        self.condition(guard);
                    }
                    self.expr(&arm.value);
                    results.push(self.shape(&arm.value));
                    self.scopes.pop();
                    self.scope_spans.pop();
                }
                let result = if results.iter().all(|shape| Some(shape) == results.first()) {
                    results.first().cloned().unwrap_or(Shape::Unknown)
                } else {
                    Shape::Alternatives(results)
                };
                self.expression_shapes
                    .insert((expr.span.start, expr.span.end), result);
            }
            ExprKind::Name(name) => {
                let definition = self
                    .scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(name.text))
                    .map(|binding| binding.1);
                if let Some(definition) = definition
                    && self.moved.contains_key(&definition.start)
                {
                    self.error(
                        name.span,
                        "moved-value",
                        "Binding was moved out of its cell; assign it again before reading",
                    );
                }
                self.references.push(NameReference {
                    definition_module: None,
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
    Constructor(String, Option<String>),
    Function(Vec<Option<ValueType>>, Box<Shape>, bool),
    Alternatives(Vec<Shape>),
    Unknown,
    Typed(ValueType),
    List(Vec<Shape>),
    Tuple(Vec<Shape>),
    Record(std::collections::BTreeMap<String, Shape>),
    // Functions and records with unknown keys have no current ValueType contract.
    Unsupported,
}
impl Shape {
    /// Whether a value of this shape can carry captured cells — closures or
    /// containers whose payload is not statically known to be scalar. Pure
    /// scalar copies never move a cell out of its region.
    fn can_carry_cells(&self) -> bool {
        fn type_carries(ty: &ValueType) -> bool {
            match ty {
                ValueType::Function(..)
                | ValueType::User(_)
                | ValueType::HostObject(_)
                | ValueType::Sequence => true,
                ValueType::Option(inner) => type_carries(inner),
                ValueType::Result(ok, err) => type_carries(ok) || type_carries(err),
                ValueType::List(element) => type_carries(element),
                ValueType::Tuple(items) => items.iter().any(type_carries),
                _ => false,
            }
        }
        match self {
            Self::Function(..) | Self::Constructor(..) | Self::Unknown | Self::Unsupported => true,
            Self::Alternatives(items) | Self::List(items) | Self::Tuple(items) => {
                items.iter().any(Self::can_carry_cells)
            }
            Self::Record(fields) => fields.values().any(Self::can_carry_cells),
            Self::Typed(ty) => type_carries(ty),
        }
    }
    fn has_contract(&self) -> bool {
        match self {
            Self::Typed(_) => true,
            Self::List(items) | Self::Tuple(items) => items.iter().all(Self::has_contract),
            Self::Record(fields) => fields.values().all(Self::has_contract),
            _ => false,
        }
    }
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
            Self::Alternatives(shapes) => shapes.iter().any(|shape| shape.rejects(expected)),
            Self::Unknown => false,
            Self::Function(parameters, result, suspends) => {
                if let ValueType::Function(expected_parameters, expected_result) = expected {
                    *suspends
                        || parameters.len() != expected_parameters.len()
                        || parameters
                            .iter()
                            .zip(expected_parameters)
                            .any(|(actual, expected)| actual.as_ref() != Some(expected))
                        || result.rejects(expected_result)
                        || matches!(result.as_ref(), Shape::Unknown)
                } else {
                    true
                }
            }
            Self::Unsupported | Self::Record(_) | Self::Constructor(..) => true,
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

fn lvalue_root<'a, 's>(expr: &'a Expr<'s>) -> Option<&'a Name<'s>> {
    match &expr.kind {
        ExprKind::Name(name) => Some(name),
        ExprKind::Index { object, .. } | ExprKind::Member { object, .. } => lvalue_root(object),
        _ => None,
    }
}
