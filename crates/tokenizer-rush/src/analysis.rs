//! Lexical binding checks. Unknown names remain available to the host.
use crate::{Expr, ExprKind, HostFunction, Name, Parse, ValueType};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::RangeInclusive;
use std::rc::Rc;
use themoretheless_tokenizer_core::{Diagnostic, Span};

mod expr;
mod inference;
mod patterns;
mod regions;
mod shape;
mod stmt;

use self::inference::BlockExits;
use self::shape::Shape;

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
    pub member_paths: BTreeMap<String, Vec<String>>,
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
            parsed.diagnostics.push(Diagnostic::new(
                Span::new(id, id + 1),
                "dynamic-contract",
                "Function contract cannot be inferred completely; annotate dynamic parameters and return type",
            ));
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
        imports: HashSet::new(),
        binding_modules: HashMap::new(),
        module_exports,
        exports: HashSet::new(),
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
        suspending_functions: HashSet::new(),
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
        hoisted_declarations: HashSet::new(),
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
    Option<Vec<RangeInclusive<usize>>>,
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
    Option<Vec<RangeInclusive<usize>>>,
    Option<Rc<HostFunction>>,
);

pub(crate) type ModuleInterfaces = HashMap<String, Rc<ModuleInterface>>;

#[derive(Clone, Default)]
pub(crate) struct ModuleInterface {
    fields: BTreeMap<String, Shape>,
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
    let registered: HashSet<_> = modules.iter().map(|(name, _)| *name).collect();
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
    fields: Option<BTreeMap<String, ValueType>>,
    variants: BTreeMap<String, Vec<ValueType>>,
}

struct Checker<'s> {
    module: Option<String>,
    type_names: HashMap<String, String>,
    imports: HashSet<String>,
    binding_modules: HashMap<usize, String>,
    module_exports: ModuleInterfaces,
    exports: HashSet<String>,
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
    suspending_functions: HashSet<usize>,
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
    /// Pre-declared `import` / `struct` / `enum` / `fn` definition start offsets in the active scope.
    hoisted_declarations: HashSet<usize>,
}

impl<'s> Checker<'s> {
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

    fn declare(&mut self, name: &Name<'s>, constant: bool, start: usize) {
        let scope = self.scopes.last_mut().unwrap();
        let is_hoisted_self = self.hoisted_declarations.remove(&name.span.start)
            && scope.get(name.text).is_some_and(|b| b.1 == name.span);
        if !is_hoisted_self && scope.contains_key(name.text) {
            self.error(
                name.span,
                "duplicate-binding",
                "Name is already declared in this scope",
            );
        } else {
            let existing = scope.get(name.text).cloned();
            scope.insert(
                name.text,
                (
                    constant,
                    name.span,
                    existing.as_ref().and_then(|b| b.2.clone()),
                    existing.as_ref().and_then(|b| b.3.clone()),
                ),
            );
            self.binding_regions
                .insert(name.span.start, self.region_depth);
            if self.collect_members {
                self.bindings.push(LexicalBinding {
                    name: name.text.to_owned(),
                    definition: name.span,
                    visible: Span::new(start, self.scope_spans.last().unwrap().end),
                    depth: self.scopes.len() - 1,
                    members: Vec::new(),
                    member_paths: BTreeMap::new(),
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
}
