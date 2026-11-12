//! Conservative static expression types. Unknown host information suppresses
//! diagnostics; the transport AST and runtime evaluator are deliberately unchanged.
use crate::frontend::SpanTable;
use crate::semantics;
use serde_json::{Value as V, json};
use std::collections::{HashMap, HashSet};

pub const BUILTIN_PROPERTY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct TypeContext {
    properties: HashMap<String, HashMap<String, String>>,
    components: HashMap<String, HashMap<String, String>>,
    required_component_props: HashMap<String, HashSet<String>>,
    component_prop_constraints: HashMap<String, HashMap<String, V>>,
    component_events: HashMap<String, HashMap<String, Vec<String>>>,
    component_styles: HashMap<String, HashSet<String>>,
    registered_components: HashSet<String>,
    actions: HashMap<String, Vec<String>>,
    enums: HashMap<String, Vec<String>>,
    structs: HashMap<String, HashMap<String, String>>,
    struct_constraints: HashMap<String, HashMap<String, V>>,
    tokens: HashMap<String, String>,
    i18n: HashMap<String, String>,
    styles: HashSet<String>,
    module_paths: HashSet<String>,
    unresolved_module_uses: HashSet<String>,
    strict_styles: bool,
}
impl Default for TypeContext {
    fn default() -> Self {
        Self::builtin()
    }
}
impl TypeContext {
    pub fn empty() -> Self {
        Self {
            properties: HashMap::new(),
            components: HashMap::new(),
            required_component_props: HashMap::new(),
            component_prop_constraints: HashMap::new(),
            component_events: HashMap::new(),
            component_styles: HashMap::new(),
            registered_components: HashSet::new(),
            actions: HashMap::new(),
            enums: HashMap::new(),
            structs: HashMap::new(),
            struct_constraints: HashMap::new(),
            tokens: HashMap::new(),
            i18n: HashMap::new(),
            styles: HashSet::new(),
            module_paths: HashSet::new(),
            unresolved_module_uses: HashSet::new(),
            strict_styles: false,
        }
    }
    pub fn builtin() -> Self {
        let mut context = Self::empty();
        for (name, ty) in [
            ("width", "Length"),
            ("height", "Length"),
            ("minWidth", "Length"),
            ("maxWidth", "Length"),
            ("minHeight", "Length"),
            ("maxHeight", "Length"),
            ("x", "Length"),
            ("y", "Length"),
            ("fontSize", "Length"),
            ("disabled", "Bool"),
            ("show", "Bool"),
            ("hide", "Bool"),
            ("visible", "Bool"),
            ("open", "Bool"),
            ("dismiss", "Bool"),
            ("backdrop", "Bool"),
            ("transitionDuration", "Duration"),
        ] {
            context.set_property("*", name, ty);
        }
        for node in ["Text", "Surface", "Button"] {
            context.set_property(node, "text", "String");
        }
        for node in ["TextInput", "TextField"] {
            context.set_property(node, "value", "String");
            context.set_property(node, "placeholder", "String");
        }
        context.set_property("Slider", "value", "Number | Length");
        context
    }
    pub fn set_property(&mut self, node: &str, name: &str, ty: &str) {
        self.properties
            .entry(node.into())
            .or_default()
            .insert(name.into(), ty.into());
    }
    pub fn set_component_property(&mut self, component: &str, name: &str, ty: &str) {
        self.components
            .entry(component.into())
            .or_default()
            .insert(name.into(), ty.into());
    }
    pub fn set_required_component_property(&mut self, component: &str, name: &str, required: bool) {
        let entry = self
            .required_component_props
            .entry(component.into())
            .or_default();
        if required {
            entry.insert(name.into());
        } else {
            entry.remove(name);
        }
    }
    pub fn set_component_property_constraint(
        &mut self,
        component: &str,
        name: &str,
        constraints: V,
    ) {
        if constraints.is_object() {
            self.component_prop_constraints
                .entry(component.into())
                .or_default()
                .insert(name.into(), constraints);
        }
    }
    pub fn set_component_event(&mut self, component: &str, event: &str, arguments: Vec<String>) {
        self.component_events
            .entry(component.into())
            .or_default()
            .insert(event.into(), arguments);
    }
    pub fn set_action(&mut self, name: &str, arguments: Vec<String>) {
        self.actions.insert(name.into(), arguments);
    }
    pub fn set_enum(&mut self, name: &str, variants: Vec<String>) {
        self.enums.insert(name.into(), variants);
    }
    pub fn set_struct(&mut self, name: &str, fields: HashMap<String, String>) {
        self.structs.insert(name.into(), fields);
    }
    pub fn set_struct_constraint(&mut self, struct_name: &str, field: &str, constraints: V) {
        if constraints.is_object() {
            self.struct_constraints
                .entry(struct_name.into())
                .or_default()
                .insert(field.into(), constraints);
        }
    }
    pub fn set_token(&mut self, key: &str, ty: &str) {
        self.tokens.insert(key.into(), ty.into());
    }
    pub fn set_i18n(&mut self, key: &str, ty: &str) {
        self.i18n.insert(key.into(), ty.into());
    }
    pub fn add_style(&mut self, name: &str) {
        self.styles.insert(name.into());
    }
    pub fn set_strict_styles(&mut self, strict: bool) {
        self.strict_styles = strict;
    }
    pub fn property_type(&self, node: &str, name: &str) -> Option<&str> {
        self.components
            .get(node)
            .and_then(|p| p.get(name))
            .or_else(|| self.properties.get(node).and_then(|p| p.get(name)))
            .or_else(|| self.properties.get("*").and_then(|p| p.get(name)))
            .map(String::as_str)
    }
    pub fn register_module_ast(&mut self, path: Option<&str>, doc: &V) {
        if let Some(p) = path {
            self.module_paths.insert(p.into());
            self.unresolved_module_uses.remove(p);
        }
        if let Some(uses) = doc["uses"].as_array() {
            for u in uses.iter().filter_map(V::as_str) {
                if !self.module_paths.contains(u) {
                    self.unresolved_module_uses.insert(u.into());
                }
            }
        }
        if let Some(enums) = doc["enums"].as_object() {
            for (name, variants) in enums {
                if let Some(arr) = variants.as_array() {
                    let list = arr.iter().filter_map(V::as_str).map(String::from).collect();
                    self.set_enum(name, list);
                }
            }
        }
        if let Some(structs) = doc["structs"].as_object() {
            for (name, fields) in structs {
                if let Some(obj) = fields.as_object() {
                    let map = obj
                        .iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.into())))
                        .collect();
                    self.set_struct(name, map);
                }
            }
        }
        if let Some(sc) = doc["structConstraints"].as_object() {
            for (sname, fields) in sc {
                if let Some(obj) = fields.as_object() {
                    for (fname, c) in obj {
                        self.set_struct_constraint(sname, fname, c.clone());
                    }
                }
            }
        }
        if let Some(styles) = doc["styles"].as_object() {
            for sname in styles.keys() {
                self.add_style(sname);
            }
        }
        let empty_spans = SpanTable::default();
        let inferred_dicts = {
            let mut analyzer = Analyzer::new(doc, &empty_spans, self);
            analyzer.infer_declarations();
            analyzer.dictionary_cache
        };
        for (cache_key, ty) in inferred_dicts {
            if !ty.known() {
                continue;
            }
            if let Some(key) = cache_key.strip_prefix("tokens.") {
                self.set_token(key, &ty.display());
            } else if let Some(key) = cache_key.strip_prefix("i18n.") {
                self.set_i18n(key, &ty.display());
            }
        }
        let is_design = !doc["designBody"].is_null()
            || (doc["entries"].as_array().is_some_and(|a| !a.is_empty())
                && doc["nodes"].as_array().is_none_or(Vec::is_empty)
                && doc["propDefinitions"]
                    .as_object()
                    .is_none_or(serde_json::Map::is_empty));
        let comp_name = doc["name"].as_str().unwrap_or("");
        let is_component = !is_design
            && !comp_name.is_empty()
            && (comp_name != "Preview"
                || doc["propDefinitions"]
                    .as_object()
                    .is_some_and(|o| !o.is_empty())
                || doc["eventDefinitions"]
                    .as_object()
                    .is_some_and(|o| !o.is_empty())
                || !doc["base"].is_null());
        if is_component {
            self.registered_components.insert(comp_name.into());
            if let Some(base) = doc["base"].as_str() {
                if let Some(base_props) = self.components.get(base).cloned() {
                    let target = self.components.entry(comp_name.into()).or_default();
                    for (k, v) in base_props {
                        target.entry(k).or_insert(v);
                    }
                }
                if let Some(base_req) = self.required_component_props.get(base).cloned() {
                    let target = self
                        .required_component_props
                        .entry(comp_name.into())
                        .or_default();
                    for req in base_req {
                        if doc["defaults"].get(&req).is_none() {
                            target.insert(req);
                        }
                    }
                }
                if let Some(base_c) = self.component_prop_constraints.get(base).cloned() {
                    let target = self
                        .component_prop_constraints
                        .entry(comp_name.into())
                        .or_default();
                    for (k, v) in base_c {
                        target.entry(k).or_insert(v);
                    }
                }
                if let Some(base_ev) = self.component_events.get(base).cloned() {
                    let target = self.component_events.entry(comp_name.into()).or_default();
                    for (k, v) in base_ev {
                        target.entry(k).or_insert(v);
                    }
                }
                if let Some(base_st) = self.component_styles.get(base).cloned() {
                    self.component_styles
                        .entry(comp_name.into())
                        .or_default()
                        .extend(base_st);
                }
            }
            if let Some(styles) = doc["styles"].as_object() {
                let target = self.component_styles.entry(comp_name.into()).or_default();
                for sname in styles.keys() {
                    target.insert(sname.clone());
                }
            }
            if let Some(prop_defs) = doc["propDefinitions"].as_object() {
                for (pname, pdef) in prop_defs {
                    if let Some(pty) = pdef["type"].as_str() {
                        self.set_component_property(comp_name, pname, pty);
                    }
                    self.set_required_component_property(
                        comp_name,
                        pname,
                        pdef["required"] == true,
                    );
                    if pdef["constraints"].is_object() {
                        self.set_component_property_constraint(
                            comp_name,
                            pname,
                            pdef["constraints"].clone(),
                        );
                    }
                }
            }
            if let Some(event_defs) = doc["eventDefinitions"].as_object() {
                for (ename, eargs) in event_defs {
                    if let Some(arr) = eargs.as_array() {
                        let types = arr
                            .iter()
                            .filter_map(|a| a["type"].as_str().map(String::from))
                            .collect();
                        self.set_component_event(comp_name, ename, types);
                    }
                }
            }
        }
    }
    pub fn register_module_source(&mut self, source: &str) -> Result<(), crate::Diagnostic> {
        let doc = crate::frontend::parse(source).map_err(crate::Diagnostic::from)?;
        self.register_module_ast(None, &doc);
        Ok(())
    }
    pub fn register_named_module_source(
        &mut self,
        path: &str,
        source: &str,
    ) -> Result<(), crate::Diagnostic> {
        let doc = crate::frontend::parse(source).map_err(crate::Diagnostic::from)?;
        self.register_module_ast(Some(path), &doc);
        Ok(())
    }
    pub fn with_module_source(mut self, source: &str) -> Result<Self, crate::Diagnostic> {
        self.register_module_source(source)?;
        Ok(self)
    }
    pub fn with_named_module_source(
        mut self,
        path: &str,
        source: &str,
    ) -> Result<Self, crate::Diagnostic> {
        self.register_named_module_source(path, source)?;
        Ok(self)
    }
    pub(crate) fn can_validate_styles(&self, doc: &V) -> bool {
        let uses = doc["uses"].as_array();
        if uses.is_none_or(Vec::is_empty) && doc["base"].is_null() {
            return true;
        }
        let uses_ok = self.strict_styles
            || (uses.is_none_or(|list| {
                list.iter()
                    .all(|u| u.as_str().is_some_and(|p| self.module_paths.contains(p)))
            }) && self
                .unresolved_module_uses
                .iter()
                .all(|p| self.module_paths.contains(p)));
        let base_ok = doc["base"].is_null()
            || doc["base"]
                .as_str()
                .is_some_and(|b| self.registered_components.contains(b));
        uses_ok && base_ok
    }
    pub(crate) fn has_style_for_doc(&self, doc: &V, style_name: &str) -> bool {
        doc["styles"]
            .as_object()
            .is_some_and(|s| s.contains_key(style_name))
            || self.styles.contains(style_name)
            || doc["base"].as_str().is_some_and(|b| {
                self.component_styles
                    .get(b)
                    .is_some_and(|s| s.contains(style_name))
            })
    }
    fn parse_module_entry(entry: &V) -> Result<V, String> {
        if let Some(src) = entry.as_str() {
            crate::frontend::parse(src)
                .map_err(|err| format!("TypeContext.modules: {}", err.message))
        } else if entry.is_object() {
            Ok(entry.clone())
        } else {
            Err("Элемент TypeContext.modules должен быть строкой .forma или AST-объектом".into())
        }
    }
    pub fn from_json(value: &V) -> Result<Self, String> {
        let mut context = Self::builtin();
        let Some(root) = value.as_object() else {
            return if value.is_null() {
                Ok(context)
            } else {
                Err("TypeContext должен быть объектом".into())
            };
        };
        if let Some(strict) = root.get("strictStyles") {
            let Some(flag) = strict.as_bool() else {
                return Err("TypeContext.strictStyles должен быть Bool".into());
            };
            context.set_strict_styles(flag);
        }
        if let Some(modules) = root.get("modules") {
            let mut parsed: Vec<(Option<String>, V)> = Vec::new();
            if let Some(list) = modules.as_array() {
                for item in list {
                    parsed.push((None, Self::parse_module_entry(item)?));
                }
            } else if let Some(map) = modules.as_object() {
                for (path, item) in map {
                    parsed.push((Some(path.clone()), Self::parse_module_entry(item)?));
                }
            } else {
                return Err("TypeContext.modules должен быть списком или объектом".into());
            }
            for (path, doc) in &parsed {
                if doc["base"].is_null() {
                    context.register_module_ast(path.as_deref(), doc);
                }
            }
            for (path, doc) in &parsed {
                if !doc["base"].is_null() {
                    context.register_module_ast(path.as_deref(), doc);
                }
            }
        }
        for (section, component) in [("properties", false), ("components", true)] {
            if let Some(nodes) = root.get(section).and_then(V::as_object) {
                for (node, props) in nodes {
                    let Some(props) = props.as_object() else {
                        return Err(format!("TypeContext.{section}.{node} должен быть объектом"));
                    };
                    if component {
                        context.registered_components.insert(node.clone());
                    }
                    for (name, spec) in props {
                        if let Some(ty) = spec.as_str() {
                            if component {
                                context.set_component_property(node, name, ty);
                            } else {
                                context.set_property(node, name, ty);
                            }
                        } else if component && let Some(spec_obj) = spec.as_object() {
                            let Some(ty) = spec_obj.get("type").and_then(V::as_str) else {
                                return Err(format!("Тип {node}.{name} должен быть строкой"));
                            };
                            context.set_component_property(node, name, ty);
                            if let Some(req) = spec_obj.get("required").and_then(V::as_bool) {
                                context.set_required_component_property(node, name, req);
                            }
                            if let Some(constraints) = spec_obj.get("constraints")
                                && constraints.is_object()
                            {
                                context.set_component_property_constraint(
                                    node,
                                    name,
                                    constraints.clone(),
                                );
                            }
                        } else {
                            return Err(format!("Тип {node}.{name} должен быть строкой"));
                        }
                    }
                }
            }
        }
        if let Some(comp_events) = root.get("componentEvents").and_then(V::as_object) {
            for (comp, events) in comp_events {
                let Some(events) = events.as_object() else {
                    return Err(format!(
                        "TypeContext.componentEvents.{comp} должен быть объектом"
                    ));
                };
                for (ename, args) in events {
                    let Some(args) = args.as_array() else {
                        return Err(format!(
                            "TypeContext.componentEvents.{comp}.{ename} должен быть списком"
                        ));
                    };
                    let mut types = Vec::with_capacity(args.len());
                    for ty in args {
                        let Some(ty) = ty.as_str() else {
                            return Err(format!(
                                "Тип аргумента события {comp}.{ename} должен быть строкой"
                            ));
                        };
                        types.push(ty.into());
                    }
                    context.set_component_event(comp, ename, types);
                }
            }
        }
        if let Some(actions) = root.get("actions").and_then(V::as_object) {
            for (name, args) in actions {
                let Some(args) = args.as_array() else {
                    return Err(format!("TypeContext.actions.{name} должен быть списком"));
                };
                let mut types = Vec::with_capacity(args.len());
                for ty in args {
                    let Some(ty) = ty.as_str() else {
                        return Err(format!("Тип аргумента action {name} должен быть строкой"));
                    };
                    types.push(ty.into());
                }
                context.set_action(name, types);
            }
        }
        if let Some(enums) = root.get("enums").and_then(V::as_object) {
            for (name, variants) in enums {
                let Some(arr) = variants.as_array() else {
                    return Err(format!("TypeContext.enums.{name} должен быть списком"));
                };
                let mut list = Vec::with_capacity(arr.len());
                for v in arr {
                    let Some(v) = v.as_str() else {
                        return Err(format!("Вариант enum {name} должен быть строкой"));
                    };
                    list.push(v.into());
                }
                context.set_enum(name, list);
            }
        }
        if let Some(structs) = root.get("structs").and_then(V::as_object) {
            for (name, fields) in structs {
                let Some(obj) = fields.as_object() else {
                    return Err(format!("TypeContext.structs.{name} должен быть объектом"));
                };
                let mut map = HashMap::with_capacity(obj.len());
                for (fname, fty) in obj {
                    let Some(fty) = fty.as_str() else {
                        return Err(format!("Тип поля {name}.{fname} должен быть строкой"));
                    };
                    map.insert(fname.clone(), fty.into());
                }
                context.set_struct(name, map);
            }
        }
        if let Some(sc) = root.get("structConstraints").and_then(V::as_object) {
            for (sname, fields) in sc {
                let Some(obj) = fields.as_object() else {
                    return Err(format!(
                        "TypeContext.structConstraints.{sname} должен быть объектом"
                    ));
                };
                for (fname, constraints) in obj {
                    context.set_struct_constraint(sname, fname, constraints.clone());
                }
            }
        }
        for (section, is_i18n) in [("tokens", false), ("i18n", true)] {
            if let Some(entries) = root.get(section).and_then(V::as_object) {
                for (key, ty) in entries {
                    let Some(ty) = ty.as_str() else {
                        return Err(format!("TypeContext.{section}.{key} должен быть строкой"));
                    };
                    if is_i18n {
                        context.set_i18n(key, ty);
                    } else {
                        context.set_token(key, ty);
                    }
                }
            }
        }
        if let Some(styles) = root.get("styles") {
            let Some(arr) = styles.as_array() else {
                return Err("TypeContext.styles должен быть списком".into());
            };
            for s in arr {
                let Some(name) = s.as_str() else {
                    return Err("Имя стиля в TypeContext.styles должно быть строкой".into());
                };
                context.add_style(name);
            }
        }
        Ok(context)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Unknown,
    Any,
    Null,
    Bool,
    Number,
    Int,
    String,
    Color,
    Length,
    Duration,
    Angle,
    List(Box<Ty>),
    Map(Box<Ty>),
    Record(Box<Ty>),
    Object,
    Named(String),
    Union(Vec<Ty>),
}
impl Ty {
    fn display(&self) -> String {
        match self {
            Self::Unknown => "Unknown".into(),
            Self::Any => "Any".into(),
            Self::Null => "Null".into(),
            Self::Bool => "Bool".into(),
            Self::Number => "Number".into(),
            Self::Int => "Int".into(),
            Self::String => "String".into(),
            Self::Color => "Color".into(),
            Self::Length => "Length".into(),
            Self::Duration => "Duration".into(),
            Self::Angle => "Angle".into(),
            Self::List(t) => format!("List<{}>", t.display()),
            Self::Map(t) => format!("Map<{}>", t.display()),
            Self::Record(t) => format!("Record<String, {}>", t.display()),
            Self::Object => "Object".into(),
            Self::Named(n) => n.clone(),
            Self::Union(ts) => ts.iter().map(Self::display).collect::<Vec<_>>().join(" | "),
        }
    }
    fn known(&self) -> bool {
        !matches!(self, Self::Unknown | Self::Any)
    }
}
fn push_unique(out: &mut Vec<Ty>, ty: Ty) {
    if !out.contains(&ty) {
        out.push(ty);
    }
}
fn union(types: impl IntoIterator<Item = Ty>) -> Ty {
    let mut out = Vec::new();
    for ty in types {
        if ty == Ty::Unknown {
            return Ty::Unknown;
        }
        match ty {
            Ty::Union(ts) => {
                for t in ts {
                    push_unique(&mut out, t);
                }
            }
            other => push_unique(&mut out, other),
        }
    }
    match out.len() {
        0 => Ty::Unknown,
        1 => out.pop().unwrap(),
        _ => Ty::Union(out),
    }
}
fn unwrap_list_once(ty: &Ty) -> Ty {
    match ty {
        Ty::List(inner) => (**inner).clone(),
        Ty::Union(items) => union(items.iter().map(unwrap_list_once)),
        other => other.clone(),
    }
}
fn non_null(ty: Ty) -> Ty {
    match ty {
        Ty::Union(ts) => union(ts.into_iter().filter(|t| *t != Ty::Null)),
        Ty::Null => Ty::Unknown,
        other => other,
    }
}
fn split_top_level(s: &str, separator: char) -> Vec<&str> {
    let mut depth = 0i32;
    let mut last = 0;
    let mut out = Vec::new();
    for (index, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ if ch == separator && depth == 0 => {
                out.push(s[last..index].trim());
                last = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s[last..].trim());
    out
}
fn parse_type(raw: &str) -> Ty {
    let raw = raw.trim();
    match raw {
        "Unknown" | "" => return Ty::Unknown,
        "Any" => return Ty::Any,
        "Bool" | "Boolean" => return Ty::Bool,
        "Number" | "Float" => return Ty::Number,
        "Int" => return Ty::Int,
        "String" | "Asset" => return Ty::String,
        "Color" => return Ty::Color,
        "Length" => return Ty::Length,
        "Duration" => return Ty::Duration,
        "Angle" => return Ty::Angle,
        "List" => return Ty::List(Box::new(Ty::Any)),
        "Map" => return Ty::Map(Box::new(Ty::Any)),
        "Object" => return Ty::Object,
        _ if !raw.contains(['|', '?', '<']) => return Ty::Named(raw.into()),
        _ => {}
    }
    let alternatives = split_top_level(raw, '|');
    if alternatives.len() > 1 {
        return union(alternatives.into_iter().map(parse_type));
    }
    if let Some(inner) = raw.strip_suffix('?') {
        return union([parse_type(inner), Ty::Null]);
    }
    for (prefix, constructor) in [("List<", Ty::List as fn(Box<Ty>) -> Ty), ("Map<", Ty::Map)] {
        if let Some(inner) = raw.strip_prefix(prefix).and_then(|s| s.strip_suffix('>')) {
            return constructor(Box::new(parse_type(inner)));
        }
    }
    if let Some(inner) = raw
        .strip_prefix("Record<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let parts = split_top_level(inner, ',');
        if parts.len() == 2 && parts[0] == "String" {
            return Ty::Record(Box::new(parse_type(parts[1])));
        }
        return Ty::Unknown;
    }
    Ty::Named(raw.into())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compatible {
    Yes,
    No,
    Unknown,
}
fn compatible(actual: &Ty, expected: &Ty) -> Compatible {
    if *expected == Ty::Any {
        return Compatible::Yes;
    }
    if matches!(actual, Ty::Unknown | Ty::Any) || *expected == Ty::Unknown {
        return Compatible::Unknown;
    }
    if let Ty::Union(types) = actual {
        if types.iter().any(|ty| matches!(ty, Ty::Unknown | Ty::Any)) {
            return Compatible::Unknown;
        }
        let mut unknown = false;
        for ty in types {
            match compatible(ty, expected) {
                Compatible::No => return Compatible::No,
                Compatible::Unknown => unknown = true,
                Compatible::Yes => {}
            }
        }
        return if unknown {
            Compatible::Unknown
        } else {
            Compatible::Yes
        };
    }
    if let Ty::Union(types) = expected {
        let mut unknown = false;
        for ty in types {
            match compatible(actual, ty) {
                Compatible::Yes => return Compatible::Yes,
                Compatible::Unknown => unknown = true,
                Compatible::No => {}
            }
        }
        return if unknown {
            Compatible::Unknown
        } else {
            Compatible::No
        };
    }
    match (actual, expected) {
        (Ty::Int, Ty::Number | Ty::Length | Ty::Angle)
        | (Ty::Number, Ty::Length | Ty::Angle)
        | (Ty::Map(_), Ty::Object)
        | (Ty::Record(_), Ty::Object)
        | (Ty::Named(_), Ty::Object) => Compatible::Yes,
        (Ty::List(a), Ty::List(b)) | (Ty::Map(a), Ty::Map(b)) | (Ty::Record(a), Ty::Record(b)) => {
            compatible(a, b)
        }
        (Ty::Map(a), Ty::Record(b)) | (Ty::Record(a), Ty::Map(b)) => compatible(a, b),
        (Ty::Named(_), Ty::Map(_) | Ty::Record(_)) => Compatible::Unknown,
        (a, b) if a == b => Compatible::Yes,
        _ => Compatible::No,
    }
}

#[derive(Debug, Clone, Copy)]
struct Span {
    from: usize,
    to: usize,
}
fn expr_matches(span_raw: &str, doc_raw: &str) -> bool {
    if span_raw == doc_raw {
        return true;
    }
    if let (Some(sr), Some(dr)) = (span_raw.strip_prefix('!'), doc_raw.strip_prefix('!')) {
        return dr.strip_prefix("state.") == Some(sr) || dr.strip_prefix("props.") == Some(sr);
    }
    doc_raw.strip_prefix("state.") == Some(span_raw)
        || doc_raw.strip_prefix("props.") == Some(span_raw)
}
fn ast_span_equal(span_val: &V, doc_val: &V) -> bool {
    match (span_val, doc_val) {
        (V::Null, V::Null) => true,
        (V::Bool(a), V::Bool(b)) => a == b,
        (V::Number(a), V::Number(b)) => a.as_f64() == b.as_f64(),
        (V::String(a), V::String(b)) => a == b,
        (V::Array(a), V::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| ast_span_equal(x, y))
        }
        (V::Object(a), V::Object(b)) => {
            if a.len() != b.len() {
                return false;
            }
            if a.len() == 1
                && let (Some(V::String(sa)), Some(V::String(sb))) = (a.get("expr"), b.get("expr"))
            {
                return expr_matches(sa, sb);
            }
            a.iter()
                .all(|(k, va)| b.get(k).is_some_and(|vb| ast_span_equal(va, vb)))
        }
        _ => false,
    }
}
fn ast_range(value: &V) -> Option<Span> {
    Some(Span {
        from: value["from"].as_u64()? as usize,
        to: value["to"].as_u64()? as usize,
    })
}
fn node_range(value: &V) -> Option<Span> {
    Some(Span {
        from: value["start"].as_u64()? as usize,
        to: value["end"].as_u64()? as usize,
    })
}
pub(crate) fn match_container_span(table: &SpanTable, value: &V) -> Option<(usize, usize)> {
    let span = if value["type"] == "Match" {
        node_range(value)
    } else {
        root_span(table, value, None)
    }?;
    Some((span.from, span.to))
}
pub(crate) fn next_child_span(
    table: &SpanTable,
    value: &V,
    parent: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    let span = child_span_after(
        table,
        value,
        parent.map(|(from, to)| Span { from, to }),
        after.map(|(from, to)| Span { from, to }),
    )?;
    Some((span.from, span.to))
}
fn root_span(table: &SpanTable, value: &V, range: Option<Span>) -> Option<Span> {
    if let Some(range) = range {
        let start = table.entries.partition_point(|e| e.from < range.from);
        let mut best: Option<Span> = None;
        for entry in &table.entries[start..] {
            if entry.from > range.to {
                break;
            }
            if entry.to > range.to {
                continue;
            }
            let len = entry.to - entry.from;
            let is_exact = entry.from == range.from && entry.to == range.to;
            if !is_exact && best.is_some_and(|b| len < b.to - b.from) {
                continue;
            }
            if ast_span_equal(&entry.value, value) {
                let candidate = Span {
                    from: entry.from,
                    to: entry.to,
                };
                if is_exact {
                    return Some(candidate);
                }
                best = Some(candidate);
            }
        }
        return best;
    }
    let mut best: Option<Span> = None;
    for entry in &table.entries {
        let len = entry.to - entry.from;
        if best.is_some_and(|b| len < b.to - b.from) {
            continue;
        }
        if ast_span_equal(&entry.value, value) {
            best = Some(Span {
                from: entry.from,
                to: entry.to,
            });
        }
    }
    best
}
fn child_span(table: &SpanTable, value: &V, parent: Option<Span>) -> Option<Span> {
    child_span_after(table, value, parent, None)
}
fn child_span_after(
    table: &SpanTable,
    value: &V,
    parent: Option<Span>,
    after: Option<Span>,
) -> Option<Span> {
    let parent = parent?;
    let min_from = after.map_or(parent.from, |span| span.to).max(parent.from);
    if min_from > parent.to {
        return None;
    }
    let start = table.entries.partition_point(|e| e.from < min_from);
    let mut best: Option<((usize, usize), Span)> = None;
    for entry in &table.entries[start..] {
        if entry.from > parent.to {
            break;
        }
        if entry.to > parent.to {
            continue;
        }
        let key = (entry.to - entry.from, entry.from);
        if best.is_some_and(|(bk, _)| key >= bk) {
            continue;
        }
        if ast_span_equal(&entry.value, value) {
            best = Some((
                key,
                Span {
                    from: entry.from,
                    to: entry.to,
                },
            ));
        }
    }
    best.map(|(_, span)| span)
}
fn pure_function_arity(name: &str) -> Option<(usize, Option<usize>, &'static str)> {
    match name {
        "coalesce" | "min" | "max" => Some((1, None, ">= 1")),
        "merge" => Some((2, None, ">= 2")),
        "remap" => Some((5, Some(5), "5")),
        "clamp" | "replace" | "find" | "lerp" => Some((3, Some(3), "3")),
        "plural" => Some((3, Some(4), "3..=4")),
        "contains" | "includes" | "indexOf" | "startsWith" | "endsWith" | "split" | "take"
        | "skip" | "range" | "pluck" | "alpha" | "lighten" | "darken" | "saturate"
        | "desaturate" | "contrast" | "groupBy" | "pow" | "repeat" | "chunk" | "zip" | "pick"
        | "omit" => Some((2, Some(2), "2")),
        "slice" | "mix" | "isValid" | "padStart" | "padEnd" | "truncate" => {
            Some((2, Some(3), "2..=3"))
        }
        "join" | "formatNumber" | "sortBy" | "unique" => Some((1, Some(2), "1..=2")),
        _ if crate::frontend::is_pure_function(name) => Some((1, Some(1), "1")),
        _ => None,
    }
}
fn is_static_literal(val: &V) -> bool {
    matches!(val, V::Bool(_) | V::Number(_) | V::String(_) | V::Array(_))
}

#[derive(Debug, Default, Clone)]
struct ConditionFacts {
    when_true: HashMap<String, Ty>,
    when_false: HashMap<String, Ty>,
}
fn is_nullable(ty: &Ty) -> bool {
    matches!(ty, Ty::Null) || matches!(ty, Ty::Union(ts) if ts.contains(&Ty::Null))
}
fn narrow_non_null(ty: &Ty) -> Option<Ty> {
    if !is_nullable(ty) {
        return None;
    }
    let nn = non_null(ty.clone());
    (nn != Ty::Unknown && nn != *ty).then_some(nn)
}
fn narrow_by_type(subject: &Ty, target: &Ty) -> Option<Ty> {
    if !subject.known() || !target.known() {
        return None;
    }
    if let Ty::Union(members) = subject {
        let kept = members
            .iter()
            .filter(|m| {
                compatible(target, m) != Compatible::No || compatible(m, target) != Compatible::No
            })
            .cloned()
            .collect::<Vec<_>>();
        if !kept.is_empty() && kept.len() < members.len() {
            return Some(union(kept));
        }
    }
    None
}
fn is_nonempty_named_pattern(pat: &str) -> bool {
    matches!(
        pat,
        "email" | "digits" | "alpha" | "alphanumeric" | "nonempty" | "url" | "hex-color" | "uuid"
    )
}
fn extract_raw_ref_path(value: &V) -> Option<String> {
    if let Some(raw) = value["expr"].as_str() {
        let first = raw.as_bytes().first().copied()?;
        if !(first.is_ascii_alphabetic() || first == b'_')
            || matches!(raw, "auto" | "content" | "_")
            || raw.ends_with('*')
        {
            return None;
        }
        return Some(raw.into());
    }
    if value["expression"]["kind"] == "member"
        && value["expression"]["optional"] != true
        && value["expression"]["computed"] != true
        && let Some(prop) = value["expression"]["property"].as_str()
        && let Some(base) = extract_raw_ref_path(&value["expression"]["object"])
    {
        return Some(format!("{base}.{prop}"));
    }
    None
}
fn merge_and_facts(left: ConditionFacts, right: ConditionFacts) -> ConditionFacts {
    let mut when_true = left.when_true.clone();
    when_true.extend(right.when_true);
    let mut right_false_env = left.when_true;
    right_false_env.extend(right.when_false);
    let mut when_false = HashMap::new();
    for (k, t1) in left.when_false {
        if let Some(t2) = right_false_env.get(&k) {
            let joined = union([t1, t2.clone()]);
            if joined.known() {
                when_false.insert(k, joined);
            }
        }
    }
    ConditionFacts {
        when_true,
        when_false,
    }
}
fn merge_or_facts(left: ConditionFacts, right: ConditionFacts) -> ConditionFacts {
    let mut when_false = left.when_false.clone();
    when_false.extend(right.when_false);
    let mut right_true_env = left.when_false;
    right_true_env.extend(right.when_true);
    let mut when_true = HashMap::new();
    for (k, t1) in left.when_true {
        if let Some(t2) = right_true_env.get(&k) {
            let joined = union([t1, t2.clone()]);
            if joined.known() {
                when_true.insert(k, joined);
            }
        }
    }
    ConditionFacts {
        when_true,
        when_false,
    }
}

struct MatchSubjectCtx<'a> {
    subject_ty: &'a Ty,
    subject_canon: Option<String>,
    tuple_slots: Vec<(Option<String>, Ty)>,
    covered_atoms: Vec<String>,
    covered_tuple_atoms: Vec<Vec<String>>,
}
impl<'a> MatchSubjectCtx<'a> {
    fn new(analyzer: &Analyzer<'_>, subject: &V, subject_ty: &'a Ty) -> Self {
        let subject_canon = analyzer.extract_canonical_path(subject);
        let tuple_slots = analyzer.infer_subject_tuple_types(subject);
        let covered_tuple_atoms = vec![Vec::new(); tuple_slots.len()];
        Self {
            subject_ty,
            subject_canon,
            tuple_slots,
            covered_atoms: Vec::new(),
            covered_tuple_atoms,
        }
    }
    fn record_unguarded(&mut self, branch: &V) {
        Analyzer::record_unguarded_branch_coverage(
            branch,
            &mut self.covered_atoms,
            &mut self.covered_tuple_atoms,
        );
    }
}

#[derive(Debug)]
pub struct TypecheckReport {
    pub diagnostics: Vec<V>,
    pub expressions_total: usize,
    pub expressions_typed: usize,
}
struct Analyzer<'a> {
    doc: &'a V,
    spans: &'a SpanTable,
    context: &'a TypeContext,
    diagnostics: Vec<V>,
    expressions_total: usize,
    expressions_typed: usize,
    locals: HashMap<String, Ty>,
    narrowed: HashMap<String, Ty>,
    let_cache: HashMap<String, Ty>,
    dictionary_cache: HashMap<String, Ty>,
    visiting: HashSet<String>,
    expected: Option<Ty>,
    in_design_override: bool,
}
fn contextual_list_element(expected: Option<&Ty>) -> Option<Ty> {
    match expected? {
        Ty::List(elem) => Some((**elem).clone()),
        Ty::Union(ts) => {
            let mut found = None;
            for t in ts {
                if *t == Ty::Null {
                    continue;
                }
                if let Ty::List(elem) = t
                    && found.is_none()
                {
                    found = Some((**elem).clone());
                } else {
                    return None;
                }
            }
            found
        }
        _ => None,
    }
}
fn contextual_record_value(expected: Option<&Ty>) -> Option<Ty> {
    match expected? {
        Ty::Map(v) | Ty::Record(v) => Some((**v).clone()),
        Ty::Union(ts) => {
            let mut found = None;
            for t in ts {
                if *t == Ty::Null {
                    continue;
                }
                if let Ty::Map(v) | Ty::Record(v) = t
                    && found.is_none()
                {
                    found = Some((**v).clone());
                } else {
                    return None;
                }
            }
            found
        }
        _ => None,
    }
}
fn has_struct_in(doc: &V, context: &TypeContext, name: &str) -> bool {
    doc["structs"].get(name).is_some() || context.structs.contains_key(name)
}
fn contextual_struct_name<'a>(
    doc: &'a V,
    context: &'a TypeContext,
    expected: Option<&'a Ty>,
) -> Option<&'a str> {
    match expected? {
        Ty::Named(name) if has_struct_in(doc, context, name.as_str()) => Some(name.as_str()),
        Ty::Union(ts) => {
            let mut found = None;
            for t in ts {
                if *t == Ty::Null {
                    continue;
                }
                if let Ty::Named(name) = t
                    && has_struct_in(doc, context, name.as_str())
                    && found.is_none()
                {
                    found = Some(name.as_str());
                } else {
                    return None;
                }
            }
            found
        }
        _ => None,
    }
}
impl<'a> Analyzer<'a> {
    fn new(doc: &'a V, spans: &'a SpanTable, context: &'a TypeContext) -> Self {
        Self {
            doc,
            spans,
            context,
            diagnostics: Vec::new(),
            expressions_total: 0,
            expressions_typed: 0,
            locals: HashMap::new(),
            narrowed: HashMap::new(),
            let_cache: HashMap::new(),
            dictionary_cache: HashMap::new(),
            visiting: HashSet::new(),
            expected: None,
            in_design_override: false,
        }
    }
    fn enum_variants(&self, name: &str) -> Option<Vec<&str>> {
        if let Some(arr) = self.doc["enums"].get(name).and_then(V::as_array) {
            return Some(arr.iter().filter_map(V::as_str).collect());
        }
        self.context
            .enums
            .get(name)
            .map(|list| list.iter().map(String::as_str).collect())
    }
    fn has_enum(&self, name: &str) -> bool {
        self.doc["enums"].get(name).is_some() || self.context.enums.contains_key(name)
    }
    fn struct_field_type(&self, struct_name: &str, field: &str) -> Option<&str> {
        self.doc["structs"]
            .get(struct_name)
            .and_then(|fields| fields.get(field))
            .and_then(V::as_str)
            .or_else(|| {
                self.context
                    .structs
                    .get(struct_name)
                    .and_then(|fields| fields.get(field))
                    .map(String::as_str)
            })
    }
    fn struct_field_names(&self, struct_name: &str) -> Option<Vec<&str>> {
        if let Some(fields) = self.doc["structs"].get(struct_name).and_then(V::as_object) {
            return Some(fields.keys().map(String::as_str).collect());
        }
        self.context
            .structs
            .get(struct_name)
            .map(|fields| fields.keys().map(String::as_str).collect())
    }
    fn struct_field_constraints(&self, struct_name: &str, field: &str) -> Option<&V> {
        let local = &self.doc["structConstraints"][struct_name][field];
        if local.is_object() {
            return Some(local);
        }
        self.context
            .struct_constraints
            .get(struct_name)
            .and_then(|fields| fields.get(field))
    }
    fn declared_prop_type_str(&self, name: &str) -> Option<&str> {
        self.doc["propDefinitions"]
            .get(name)
            .and_then(|d| d["type"].as_str())
            .or_else(|| {
                let base = self.doc["base"].as_str()?;
                self.context.property_type(base, name)
            })
    }
    fn declared_prop_constraints(&self, name: &str) -> Option<&V> {
        let local = &self.doc["propDefinitions"][name]["constraints"];
        if local.is_object() {
            return Some(local);
        }
        let base = self.doc["base"].as_str()?;
        self.context
            .component_prop_constraints
            .get(base)
            .and_then(|c| c.get(name))
    }
    fn has_prop_def(&self, name: &str) -> bool {
        self.declared_prop_type_str(name).is_some()
    }
    fn has_state_def(&self, name: &str) -> bool {
        self.doc["stateDefinitions"].get(name).is_some()
    }
    fn canonicalize_ref_path(&self, raw: &str) -> Option<String> {
        let (head, rest) = raw.split_once('.').unwrap_or((raw, ""));
        if rest.is_empty() {
            if self.locals.contains_key(head) {
                return Some(format!("local:{head}"));
            }
            if matches!(head, "props" | "state" | "tokens" | "i18n" | "viewport") {
                return None;
            }
            if self.has_state_def(head) {
                return Some(format!("state.{head}"));
            }
            if self.has_prop_def(head) {
                return Some(format!("props.{head}"));
            }
            if self.doc["lets"].get(head).is_some() {
                return Some(format!("let:{head}"));
            }
            return None;
        }
        match head {
            "props" => {
                let (name, sub_rest) = if rest == "font.size" {
                    ("fontSize", "")
                } else if let Some(after) = rest.strip_prefix("font.size.") {
                    ("fontSize", after)
                } else {
                    rest.split_once('.').unwrap_or((rest, ""))
                };
                let prefix = if self.has_prop_def(name) {
                    format!("props.{name}")
                } else if self.doc["lets"].get(name).is_some() {
                    format!("let:{name}")
                } else {
                    return None;
                };
                Some(if sub_rest.is_empty() {
                    prefix
                } else {
                    format!("{prefix}.{sub_rest}")
                })
            }
            "state" => {
                let (prop_name, _sub_rest) = rest.split_once('.').unwrap_or((rest, ""));
                self.has_state_def(prop_name)
                    .then(|| format!("state.{rest}"))
            }
            "tokens" | "i18n" | "viewport" => None,
            _ => {
                if self.locals.contains_key(head) {
                    Some(format!("local:{head}.{rest}"))
                } else if self.has_state_def(head) {
                    Some(format!("state.{head}.{rest}"))
                } else if self.has_prop_def(head) {
                    Some(format!("props.{head}.{rest}"))
                } else if self.doc["lets"].get(head).is_some() {
                    Some(format!("let:{head}.{rest}"))
                } else {
                    None
                }
            }
        }
    }
    fn extract_canonical_path(&self, value: &V) -> Option<String> {
        let raw = extract_raw_ref_path(value)?;
        self.canonicalize_ref_path(&raw)
    }
    fn apply_narrowing(&mut self, facts: &HashMap<String, Ty>) -> HashMap<String, Ty> {
        let saved = self.narrowed.clone();
        for (k, v) in facts {
            self.narrowed.insert(k.clone(), v.clone());
        }
        saved
    }
    fn clear_local_narrowing(&mut self, local_name: &str) {
        let exact = format!("local:{local_name}");
        let prefix = format!("local:{local_name}.");
        self.narrowed
            .retain(|k, _| k != &exact && !k.starts_with(&prefix));
    }
    fn record_equality_facts(
        &self,
        expr: &V,
        expr_ty: &Ty,
        other: &V,
        other_ty: &Ty,
        is_eq: bool,
        facts: &mut ConditionFacts,
    ) {
        let Some(canon) = self.extract_canonical_path(expr) else {
            return;
        };
        let (eq_map, neq_map) = if is_eq {
            (&mut facts.when_true, &mut facts.when_false)
        } else {
            (&mut facts.when_false, &mut facts.when_true)
        };
        if other.is_null() || *other_ty == Ty::Null {
            if compatible(&Ty::Null, expr_ty) != Compatible::No {
                eq_map.insert(canon.clone(), Ty::Null);
            }
            if let Some(nn) = narrow_non_null(expr_ty) {
                neq_map.insert(canon, nn);
            }
        } else if !is_nullable(other_ty)
            && other_ty.known()
            && let Some(narrowed) = narrow_by_type(expr_ty, other_ty)
        {
            eq_map.insert(canon, narrowed);
        }
    }
    fn emit_message(
        &mut self,
        code: &str,
        message: String,
        expected: String,
        actual: String,
        span: Option<Span>,
        context: &str,
    ) {
        let Some(span) = span else { return };
        self.diagnostics.push(json!({
            "code": code,
            "severity": "error",
            "message": message,
            "from": span.from,
            "to": span.to,
            "expected": expected,
            "actual": actual,
            "context": context,
        }));
    }
    fn emit(
        &mut self,
        code: &str,
        expected: String,
        actual: String,
        span: Option<Span>,
        context: &str,
    ) {
        self.emit_message(
            code,
            format!("{context}: ожидался {expected}, получен {actual}"),
            expected,
            actual,
            span,
            context,
        );
    }
    fn check(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        code: &str,
        span: Option<Span>,
        context: std::fmt::Arguments<'_>,
    ) -> bool {
        if compatible(actual, expected) == Compatible::No {
            let ctx = context.to_string();
            self.emit(code, expected.display(), actual.display(), span, &ctx);
            false
        } else {
            true
        }
    }
    fn check_static_constraints(
        &mut self,
        value: &V,
        constraints: &V,
        span: Option<Span>,
        label: &str,
    ) -> bool {
        if !constraints.is_object() || !is_static_literal(value) {
            return true;
        }
        if let Err(err) = semantics::check_constraints(value, constraints, label) {
            let expected = if let Some(range) = constraints["range"].as_array()
                && range.len() == 2
            {
                format!("{}..={}", range[0], range[1])
            } else if let Some(pat) = constraints["pattern"].as_str() {
                format!("pattern '{pat}'")
            } else {
                "constraint".into()
            };
            self.emit_message(
                err.code,
                err.message,
                expected,
                value.to_string(),
                span,
                label,
            );
            false
        } else {
            true
        }
    }
    fn child(&self, value: &V, parent: Option<Span>) -> Option<Span> {
        child_span(self.spans, value, parent)
    }
    fn child_after(&self, value: &V, parent: Option<Span>, after: Option<Span>) -> Option<Span> {
        child_span_after(self.spans, value, parent, after)
    }
    fn infer_root(&mut self, value: &V, range: Option<Span>) -> Ty {
        let span = root_span(self.spans, value, range).or(range);
        self.infer(value, span)
    }
    fn infer_expected(&mut self, value: &V, expected: Ty, range: Option<Span>) -> Ty {
        let prev = self.expected.replace(expected);
        let ty = self.infer_root(value, range);
        self.expected = prev;
        ty
    }
    fn infer(&mut self, value: &V, span: Option<Span>) -> Ty {
        self.infer_with_facts(value, span).0
    }
    fn infer_with_facts(&mut self, value: &V, span: Option<Span>) -> (Ty, ConditionFacts) {
        self.expressions_total += 1;
        let (ty, facts) = self.infer_inner(value, span);
        if ty.known() {
            self.expressions_typed += 1;
        }
        (ty, facts)
    }
    fn infer_inner(&mut self, value: &V, span: Option<Span>) -> (Ty, ConditionFacts) {
        let contextual = self.expected.take();
        if value.is_null() {
            return (Ty::Null, ConditionFacts::default());
        }
        if value.is_boolean() {
            return (Ty::Bool, ConditionFacts::default());
        }
        if let Some(number) = value.as_f64() {
            let ty = if number.fract() == 0.0 {
                Ty::Int
            } else {
                Ty::Number
            };
            return (ty, ConditionFacts::default());
        }
        if value.is_string() {
            return (Ty::String, ConditionFacts::default());
        }
        if let Some(items) = value.as_array() {
            let elem_expected = contextual_list_element(contextual.as_ref());
            let mut prev_span = None;
            let types = items
                .iter()
                .map(|item| {
                    let child = self.child_after(item, span, prev_span);
                    if child.is_some() {
                        prev_span = child;
                    }
                    self.expected = elem_expected.clone();
                    self.infer(item, child.or(span))
                })
                .collect::<Vec<_>>();
            self.expected = None;
            return (Ty::List(Box::new(union(types))), ConditionFacts::default());
        }
        if let Some(raw) = value["expr"].as_str() {
            return (
                self.infer_expr_reference(raw, span),
                ConditionFacts::default(),
            );
        }
        if let Some(parts) = value["interpolation"].as_array() {
            let mut prev_span = None;
            for part in parts {
                if part.is_string() {
                    continue;
                }
                let child = self.child_after(part, span, prev_span);
                if child.is_some() {
                    prev_span = child;
                }
                let part_span = child.or(span);
                let ty = self.infer(part, part_span);
                if !self.is_scalar(&ty) {
                    self.emit(
                        "incompatible-operand",
                        "String | Number | Bool | Null".into(),
                        ty.display(),
                        part_span,
                        "Строковая интерполяция",
                    );
                }
            }
            return (Ty::String, ConditionFacts::default());
        }
        if value.get("match").is_some() {
            return (
                self.infer_match_value(value, span, contextual),
                ConditionFacts::default(),
            );
        }
        if let Some(expression) = value.get("expression") {
            return self.infer_expression(expression, value, span, contextual);
        }
        if let Some(object) = value.as_object() {
            let doc = self.doc;
            let context = self.context;
            if let Some(struct_name) = contextual_struct_name(doc, context, contextual.as_ref())
                && let Some(mut field_names) = self.struct_field_names(struct_name)
            {
                let missing_required = field_names.iter().any(|fname| {
                    !self
                        .struct_field_type(struct_name, fname)
                        .unwrap_or("")
                        .trim()
                        .ends_with('?')
                        && !object.contains_key(*fname)
                });
                if !missing_required {
                    let mut valid = true;
                    field_names.sort_unstable();
                    let expected_keys = field_names.join(" | ");
                    let struct_name_owned = struct_name.to_string();
                    for (key, item) in object {
                        let child = self.child(item, span).or(span);
                        let field_expected = self
                            .struct_field_type(&struct_name_owned, key)
                            .map(parse_type);
                        self.expected = field_expected.clone();
                        let actual = self.infer(item, child);
                        if let Some(expected_ty) = &field_expected {
                            let ok = self.check(
                                &actual,
                                expected_ty,
                                "incompatible-binding",
                                child,
                                format_args!("Поле {struct_name_owned}.{key}"),
                            );
                            valid &= ok;
                            if ok
                                && let Some(constraints) = self
                                    .struct_field_constraints(&struct_name_owned, key)
                                    .cloned()
                            {
                                valid &= self.check_static_constraints(
                                    item,
                                    &constraints,
                                    child,
                                    &format!("Поле {struct_name_owned}.{key}"),
                                );
                            }
                        } else {
                            self.emit(
                                "unknown-field",
                                expected_keys.clone(),
                                key.clone(),
                                child,
                                &format!("Структура {struct_name_owned}"),
                            );
                            valid = false;
                        }
                    }
                    self.expected = None;
                    let ty = if valid {
                        Ty::Named(struct_name_owned)
                    } else {
                        Ty::Unknown
                    };
                    return (ty, ConditionFacts::default());
                }
            }
            let val_expected = contextual_record_value(contextual.as_ref());
            let types = object
                .values()
                .map(|item| {
                    let child = self.child(item, span).or(span);
                    self.expected = val_expected.clone();
                    self.infer(item, child)
                })
                .collect::<Vec<_>>();
            self.expected = None;
            return (
                Ty::Record(Box::new(union(types))),
                ConditionFacts::default(),
            );
        }
        (Ty::Unknown, ConditionFacts::default())
    }
    fn is_scalar(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Unknown | Ty::Any => true,
            Ty::Union(types) => types.iter().all(|t| self.is_scalar(t)),
            Ty::List(_) | Ty::Map(_) | Ty::Record(_) | Ty::Object => false,
            Ty::Named(name) => self.has_enum(name),
            _ => true,
        }
    }
    fn infer_expr_reference(&mut self, raw: &str, span: Option<Span>) -> Ty {
        if let Some(path) = raw.strip_prefix('!') {
            let actual = self.infer_reference(path, span);
            self.check(
                &actual,
                &Ty::Bool,
                "incompatible-operand",
                span,
                format_args!("Оператор !"),
            );
            return if actual == Ty::Unknown {
                Ty::Unknown
            } else {
                Ty::Bool
            };
        }
        if let Some(hex) = raw.strip_prefix('#') {
            return if matches!(hex.len(), 3 | 4 | 6 | 8)
                && hex.bytes().all(|b| b.is_ascii_hexdigit())
            {
                Ty::Color
            } else {
                Ty::Unknown
            };
        }
        if matches!(raw, "*" | "auto" | "content" | "-") || raw.ends_with('*') {
            return Ty::Unknown;
        }
        if let Some(ty) = unit_type(raw) {
            return ty;
        }
        self.infer_reference(raw, span)
    }
    fn infer_reference(&mut self, raw: &str, span: Option<Span>) -> Ty {
        if !self.narrowed.is_empty()
            && let Some(canon) = self.canonicalize_ref_path(raw)
        {
            if let Some(ty) = self.narrowed.get(&canon) {
                return ty.clone();
            }
            for (idx, _) in canon.rmatch_indices('.') {
                let prefix = &canon[..idx];
                let suffix = &canon[idx + 1..];
                if let Some(prefix_ty) = self.narrowed.get(prefix).cloned() {
                    return self.resolve_member_path(prefix_ty, suffix.split('.'), span);
                }
            }
        }
        let (head, rest) = raw.split_once('.').unwrap_or((raw, ""));
        if rest.is_empty() {
            if let Some(ty) = self.locals.get(head) {
                return ty.clone();
            }
            if matches!(head, "props" | "state" | "tokens" | "i18n" | "viewport") {
                return Ty::Unknown;
            }
            if let Some(ty) = self.doc["stateDefinitions"]
                .get(head)
                .and_then(|d| d["type"].as_str())
            {
                return parse_type(ty);
            }
            if let Some(ty) = self.declared_prop_type_str(head) {
                return parse_type(ty);
            }
            if self.doc["lets"].get(head).is_some() {
                return self.infer_let(head);
            }
            if let Some(variants) = self.enum_variants(head) {
                let expected = variants
                    .into_iter()
                    .map(|v| format!("{head}.{v}"))
                    .collect::<Vec<_>>()
                    .join(" | ");
                self.emit(
                    "unknown-variant",
                    expected,
                    raw.into(),
                    span,
                    &format!("Перечисление {head}"),
                );
            }
            return Ty::Unknown;
        }
        match head {
            "props" => {
                let (name, sub_rest) = if rest == "font.size" {
                    ("fontSize", "")
                } else if let Some(after) = rest.strip_prefix("font.size.") {
                    ("fontSize", after)
                } else {
                    rest.split_once('.').unwrap_or((rest, ""))
                };
                let base = self
                    .declared_prop_type_str(name)
                    .map(parse_type)
                    .or_else(|| {
                        if self.doc["lets"].get(name).is_some() {
                            Some(self.infer_let(name))
                        } else {
                            None
                        }
                    });
                base.map_or(Ty::Unknown, |ty| {
                    if sub_rest.is_empty() {
                        ty
                    } else {
                        self.resolve_member_path(ty, sub_rest.split('.'), span)
                    }
                })
            }
            "state" => {
                let (prop_name, sub_rest) = rest.split_once('.').unwrap_or((rest, ""));
                self.doc["stateDefinitions"]
                    .get(prop_name)
                    .and_then(|d| d["type"].as_str())
                    .map(parse_type)
                    .map_or(Ty::Unknown, |ty| {
                        if sub_rest.is_empty() {
                            ty
                        } else {
                            self.resolve_member_path(ty, sub_rest.split('.'), span)
                        }
                    })
            }
            "tokens" | "i18n" => self.infer_dictionary(head, rest, raw),
            "viewport" => match rest {
                "width" | "height" => Ty::Number,
                "compact" => Ty::Bool,
                _ => Ty::Unknown,
            },
            _ => {
                if let Some(ty) = self.locals.get(head).cloned() {
                    return self.resolve_member_path(ty, rest.split('.'), span);
                }
                if let Some(variants) = self.enum_variants(head) {
                    if !rest.contains('.') && variants.contains(&rest) {
                        return Ty::Named(head.into());
                    }
                    let expected = variants
                        .into_iter()
                        .map(|v| format!("{head}.{v}"))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    self.emit(
                        "unknown-variant",
                        expected,
                        raw.into(),
                        span,
                        &format!("Перечисление {head}"),
                    );
                }
                Ty::Unknown
            }
        }
    }
    fn resolve_member_path<'b>(
        &mut self,
        base: Ty,
        path: impl IntoIterator<Item = &'b str>,
        span: Option<Span>,
    ) -> Ty {
        path.into_iter()
            .fold(base, |ty, part| self.resolve_member(&ty, part, false, span))
    }
    fn resolve_member(
        &mut self,
        base: &Ty,
        property: &str,
        optional: bool,
        span: Option<Span>,
    ) -> Ty {
        let target = if optional {
            non_null(base.clone())
        } else {
            base.clone()
        };
        self.check_struct_field(&target, property, span);
        self.member_type(&target, property)
    }
    fn check_struct_field(&mut self, base: &Ty, property: &str, span: Option<Span>) {
        match base {
            Ty::Named(name) => {
                if let Some(mut keys) = self.struct_field_names(name)
                    && !keys.contains(&property)
                {
                    keys.sort_unstable();
                    self.emit(
                        "unknown-field",
                        keys.join(" | "),
                        property.into(),
                        span,
                        &format!("Структура {name}"),
                    );
                }
            }
            Ty::Union(types) => {
                let non_null_types = types.iter().filter(|t| **t != Ty::Null).collect::<Vec<_>>();
                if non_null_types.is_empty() {
                    return;
                }
                let mut all_known_structs = true;
                let mut any_has_field = false;
                let mut expected_keys = Vec::new();
                for ty in non_null_types {
                    if let Ty::Named(name) = ty
                        && let Some(keys) = self.struct_field_names(name)
                    {
                        if keys.contains(&property) {
                            any_has_field = true;
                        }
                        for k in keys {
                            if !expected_keys.contains(&k) {
                                expected_keys.push(k);
                            }
                        }
                    } else {
                        all_known_structs = false;
                        break;
                    }
                }
                if all_known_structs && !any_has_field {
                    expected_keys.sort_unstable();
                    let expected_str = expected_keys.join(" | ");
                    self.emit(
                        "unknown-field",
                        expected_str,
                        property.into(),
                        span,
                        &format!("Поле структуры {}", base.display()),
                    );
                }
            }
            _ => {}
        }
    }
    fn member_type(&self, base: &Ty, property: &str) -> Ty {
        match base {
            Ty::Named(name) => self
                .struct_field_type(name, property)
                .map(parse_type)
                .unwrap_or(Ty::Unknown),
            Ty::List(item) if property == "length" => {
                let _ = item;
                Ty::Int
            }
            Ty::List(item) if property.parse::<usize>().is_ok() => (**item).clone(),
            Ty::Map(item) | Ty::Record(item) => (**item).clone(),
            Ty::String if property == "length" => Ty::Int,
            Ty::Union(types) => union(types.iter().map(|ty| self.member_type(ty, property))),
            _ => Ty::Unknown,
        }
    }
    fn infer_let(&mut self, name: &str) -> Ty {
        if let Some(ty) = self.let_cache.get(name) {
            return ty.clone();
        }
        let key = format!("let.{name}");
        if !self.visiting.insert(key.clone()) {
            return Ty::Unknown;
        }
        let doc = self.doc;
        let value = &doc["lets"][name];
        let ty = if value.is_null() {
            Ty::Null
        } else {
            self.infer_root(value, ast_range(&doc["defaultRanges"][name]))
        };
        self.visiting.remove(&key);
        self.let_cache.insert(name.into(), ty.clone());
        ty
    }
    fn context_dictionary_type(&self, section: &str, key: &str) -> Ty {
        let map = if section == "i18n" {
            &self.context.i18n
        } else {
            &self.context.tokens
        };
        if let Some(raw) = map.get(key) {
            return parse_type(raw);
        }
        if !key.contains('.') {
            let suffix = format!(".{key}");
            let matched = map
                .iter()
                .filter(|(k, _)| k.ends_with(&suffix))
                .map(|(_, v)| parse_type(v))
                .collect::<Vec<_>>();
            if !matched.is_empty() {
                return union(matched);
            }
        }
        Ty::Unknown
    }
    fn infer_dictionary(&mut self, section: &str, key: &str, cache_key: &str) -> Ty {
        if let Some(ty) = self.dictionary_cache.get(cache_key) {
            return ty.clone();
        }
        if !self.visiting.insert(cache_key.into()) {
            return Ty::Unknown;
        }
        let doc = self.doc;
        let value = &doc[section][key];
        let ty = if value.is_null() {
            self.context_dictionary_type(section, key)
        } else {
            self.infer_root(value, None)
        };
        self.visiting.remove(cache_key);
        self.dictionary_cache.insert(cache_key.into(), ty.clone());
        ty
    }
    fn infer_expression(
        &mut self,
        expression: &V,
        whole: &V,
        span: Option<Span>,
        expected: Option<Ty>,
    ) -> (Ty, ConditionFacts) {
        match expression["kind"].as_str().unwrap_or("") {
            "conditional" => {
                let condition = &expression["condition"];
                let condition_span = self.child(condition, span);
                let mut prev_span = condition_span;
                let cond_target = condition_span.or(span);
                let (condition_ty, cond_facts) = self.infer_with_facts(condition, cond_target);
                self.check(
                    &condition_ty,
                    &Ty::Bool,
                    "incompatible-condition",
                    cond_target,
                    format_args!("Условный оператор"),
                );
                let mut valid = true;
                let types = ["then", "else"].map(|key| {
                    let value = &expression[key];
                    let child = self.child_after(value, span, prev_span);
                    if child.is_some() {
                        prev_span = child;
                    }
                    let branch_span = child.or(span);
                    let branch_facts = if key == "then" {
                        &cond_facts.when_true
                    } else {
                        &cond_facts.when_false
                    };
                    let saved_narrowed = self.apply_narrowing(branch_facts);
                    self.expected = expected.clone();
                    let ty = self.infer(value, branch_span);
                    self.narrowed = saved_narrowed;
                    if let Some(expected) = &expected {
                        valid &= self.check(
                            &ty,
                            expected,
                            "incompatible-binding",
                            branch_span,
                            format_args!("Ветвь условного оператора"),
                        );
                    }
                    ty
                });
                self.expected = None;
                let res = if valid { union(types) } else { Ty::Unknown };
                (res, ConditionFacts::default())
            }
            "unary" => {
                let argument = &expression["argument"];
                let child = self.child(argument, span).or(span);
                let (actual, mut facts) = self.infer_with_facts(argument, child);
                let operator = expression["operator"].as_str().unwrap_or("");
                let expected = if operator == "!" {
                    Ty::Bool
                } else {
                    Ty::Number
                };
                self.check(
                    &actual,
                    &expected,
                    "incompatible-operand",
                    child,
                    format_args!("Оператор {operator}"),
                );
                if operator == "!" {
                    std::mem::swap(&mut facts.when_true, &mut facts.when_false);
                } else {
                    facts = ConditionFacts::default();
                }
                let ty = if actual == Ty::Unknown {
                    Ty::Unknown
                } else if operator == "!" {
                    Ty::Bool
                } else if actual == Ty::Int {
                    Ty::Int
                } else {
                    Ty::Number
                };
                (ty, facts)
            }
            "member" => {
                let object = &expression["object"];
                let object_span = self.child(object, span);
                let base = self.infer(object, object_span.or(span));
                let optional = expression["optional"] == true;
                let Some(property) = expression["property"].as_str() else {
                    let dynamic = &expression["property"];
                    let dynamic_span = self.child_after(dynamic, span, object_span).or(span);
                    let key = self.infer(dynamic, dynamic_span);
                    self.check(
                        &key,
                        &union([Ty::String, Ty::Number]),
                        "incompatible-operand",
                        dynamic_span,
                        format_args!("Вычисляемое свойство"),
                    );
                    let target = if optional { non_null(base) } else { base };
                    let mut result = match target {
                        Ty::List(item) => union([*item, Ty::Null]),
                        Ty::Map(item) | Ty::Record(item) => union([*item, Ty::Null]),
                        _ => Ty::Unknown,
                    };
                    if optional && result != Ty::Unknown {
                        result = union([result, Ty::Null]);
                    }
                    return (result, ConditionFacts::default());
                };
                if !optional
                    && expression["computed"] != true
                    && !self.narrowed.is_empty()
                    && let Some(canon) = self.extract_canonical_path(whole)
                    && let Some(narrowed_ty) = self.narrowed.get(&canon).cloned()
                {
                    self.check_struct_field(&base, property, span);
                    return (narrowed_ty, ConditionFacts::default());
                }
                let mut result = if property == "length" && matches!(base, Ty::String | Ty::List(_))
                {
                    Ty::Int
                } else if let Some(object) = object.as_object()
                    && !object.contains_key("expr")
                    && !object.contains_key("expression")
                {
                    object
                        .get(property)
                        .map(|v| self.infer(v, self.child(v, object_span).or(span)))
                        .unwrap_or(Ty::Null)
                } else {
                    self.resolve_member(&base, property, optional, span)
                };
                if optional {
                    result = union([result, Ty::Null]);
                }
                (result, ConditionFacts::default())
            }
            "call" => {
                let name = expression["name"].as_str().unwrap_or("");
                let args = expression["args"].as_array().map_or(&[][..], Vec::as_slice);
                let mut types = Vec::with_capacity(args.len());
                let mut spans = Vec::with_capacity(args.len());
                let mut prev_span = None;
                for arg in args {
                    let child = self.child_after(arg, span, prev_span);
                    if child.is_some() {
                        prev_span = child;
                    }
                    let arg_span = child.or(span);
                    spans.push(arg_span);
                    types.push(self.infer(arg, arg_span));
                }
                let res = self.infer_call(name, args, &types, &spans, span);
                let mut facts = ConditionFacts::default();
                if name == "isEmpty"
                    && args.len() == 1
                    && let Some(canon) = self.extract_canonical_path(&args[0])
                    && let Some(nn) = narrow_non_null(&types[0])
                {
                    facts.when_false.insert(canon, nn);
                } else if name == "isValid"
                    && (args.len() == 3
                        || (args.len() == 2
                            && args[1].as_str().is_some_and(is_nonempty_named_pattern)))
                    && let Some(canon) = self.extract_canonical_path(&args[0])
                    && let Some(nn) = narrow_non_null(&types[0])
                {
                    facts.when_true.insert(canon, nn);
                }
                (res, facts)
            }
            "binary" => self.infer_binary(expression, span),
            _ => (Ty::Unknown, ConditionFacts::default()),
        }
    }
    fn infer_binary(&mut self, expression: &V, span: Option<Span>) -> (Ty, ConditionFacts) {
        let operator = expression["operator"].as_str().unwrap_or("");
        let left = &expression["left"];
        let right = &expression["right"];
        let left_found = self.child(left, span);
        let right_found = self.child_after(right, span, left_found);
        let left_span = left_found.or(span);
        let right_span = right_found.or(span);
        if operator == "&&" || operator == "||" {
            let (a, left_facts) = self.infer_with_facts(left, left_span);
            let rhs_env = if operator == "&&" {
                &left_facts.when_true
            } else {
                &left_facts.when_false
            };
            let saved_narrowed = self.apply_narrowing(rhs_env);
            let (b, right_facts) = self.infer_with_facts(right, right_span);
            self.narrowed = saved_narrowed;
            self.check(
                &a,
                &Ty::Bool,
                "incompatible-operand",
                left_span,
                format_args!("Оператор {operator}"),
            );
            self.check(
                &b,
                &Ty::Bool,
                "incompatible-operand",
                right_span,
                format_args!("Оператор {operator}"),
            );
            let facts = if operator == "&&" {
                merge_and_facts(left_facts, right_facts)
            } else {
                merge_or_facts(left_facts, right_facts)
            };
            let ty = if a == Ty::Unknown || b == Ty::Unknown {
                Ty::Unknown
            } else {
                Ty::Bool
            };
            return (ty, facts);
        }
        let a = self.infer(left, left_span);
        let b = self.infer(right, right_span);
        match operator {
            "??" => {
                let ty = match a {
                    Ty::Unknown | Ty::Any => Ty::Unknown,
                    Ty::Null => b,
                    Ty::Union(_) => union([non_null(a), b]),
                    _ => a,
                };
                (ty, ConditionFacts::default())
            }
            "==" | "!=" | "===" | "!==" => {
                let is_eq = matches!(operator, "==" | "===");
                let mut facts = ConditionFacts::default();
                self.record_equality_facts(left, &a, right, &b, is_eq, &mut facts);
                self.record_equality_facts(right, &b, left, &a, is_eq, &mut facts);
                (Ty::Bool, facts)
            }
            "<" | "<=" | ">" | ">=" => {
                let numeric = |t: &Ty| matches!(t, Ty::Number | Ty::Int | Ty::Unknown | Ty::Any);
                let string = |t: &Ty| matches!(t, Ty::String | Ty::Unknown | Ty::Any);
                let ty = if !((numeric(&a) && numeric(&b)) || (string(&a) && string(&b))) {
                    self.emit(
                        "incompatible-operand",
                        "Number и Number или String и String".into(),
                        format!("{} и {}", a.display(), b.display()),
                        span,
                        &format!("Оператор {operator}"),
                    );
                    Ty::Unknown
                } else {
                    Ty::Bool
                };
                (ty, ConditionFacts::default())
            }
            ".." | "..=" => {
                self.check(
                    &a,
                    &Ty::Number,
                    "incompatible-operand",
                    left_span,
                    format_args!("Диапазон"),
                );
                self.check(
                    &b,
                    &Ty::Number,
                    "incompatible-operand",
                    right_span,
                    format_args!("Диапазон"),
                );
                (Ty::List(Box::new(Ty::Int)), ConditionFacts::default())
            }
            "+" => {
                let ty = if matches!(a, Ty::Unknown | Ty::Any) || matches!(b, Ty::Unknown | Ty::Any)
                {
                    Ty::Unknown
                } else if a == Ty::Int && b == Ty::Int {
                    Ty::Int
                } else if matches!(a, Ty::Number | Ty::Int) && matches!(b, Ty::Number | Ty::Int) {
                    Ty::Number
                } else if a == Ty::String && b == Ty::String {
                    Ty::String
                } else {
                    self.emit(
                        "incompatible-operand",
                        "Number и Number или String и String".into(),
                        format!("{} и {}", a.display(), b.display()),
                        span,
                        "Оператор +",
                    );
                    Ty::Unknown
                };
                (ty, ConditionFacts::default())
            }
            "-" | "*" | "/" | "%" => {
                self.check(
                    &a,
                    &Ty::Number,
                    "incompatible-operand",
                    left_span,
                    format_args!("Оператор {operator}"),
                );
                self.check(
                    &b,
                    &Ty::Number,
                    "incompatible-operand",
                    right_span,
                    format_args!("Оператор {operator}"),
                );
                let ty = if a == Ty::Unknown || b == Ty::Unknown {
                    Ty::Unknown
                } else if operator != "/" && a == Ty::Int && b == Ty::Int {
                    Ty::Int
                } else {
                    Ty::Number
                };
                (ty, ConditionFacts::default())
            }
            _ => (Ty::Unknown, ConditionFacts::default()),
        }
    }
    fn subtract_covered_atoms(&self, subject_ty: &Ty, covered: &[String]) -> Ty {
        if covered.is_empty() {
            return subject_ty.clone();
        }
        let Ty::Union(members) = subject_ty else {
            return subject_ty.clone();
        };
        let kept = members
            .iter()
            .filter(|member| {
                let mut atoms = Vec::new();
                if !self.finite_domain_atoms(member, &mut atoms) || atoms.is_empty() {
                    return true;
                }
                !atoms.iter().all(|a| covered.iter().any(|c| c == a))
            })
            .cloned()
            .collect::<Vec<_>>();
        if kept.is_empty() {
            subject_ty.clone()
        } else {
            union(kept)
        }
    }
    fn infer_subject_tuple_types(&self, subject: &V) -> Vec<(Option<String>, Ty)> {
        let Some(items) = subject.as_array() else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| {
                let canon = self.extract_canonical_path(item);
                let ty = canon
                    .as_deref()
                    .and_then(|c| self.lookup_canonical_type(c))
                    .unwrap_or(Ty::Unknown);
                (canon, ty)
            })
            .collect()
    }
    fn lookup_canonical_type(&self, canon: &str) -> Option<Ty> {
        if let Some(ty) = self.narrowed.get(canon) {
            return Some(ty.clone());
        }
        if let Some(local) = canon.strip_prefix("local:") {
            let (head, rest) = local.split_once('.').unwrap_or((local, ""));
            let base = self.locals.get(head)?.clone();
            return Some(if rest.is_empty() {
                base
            } else {
                rest.split('.').fold(base, |ty, p| self.member_type(&ty, p))
            });
        }
        if let Some(prop) = canon.strip_prefix("props.") {
            let (head, rest) = prop.split_once('.').unwrap_or((prop, ""));
            let base = parse_type(self.declared_prop_type_str(head)?);
            return Some(if rest.is_empty() {
                base
            } else {
                rest.split('.').fold(base, |ty, p| self.member_type(&ty, p))
            });
        }
        if let Some(st) = canon.strip_prefix("state.") {
            let (head, rest) = st.split_once('.').unwrap_or((st, ""));
            let base = parse_type(
                self.doc["stateDefinitions"]
                    .get(head)
                    .and_then(|d| d["type"].as_str())?,
            );
            return Some(if rest.is_empty() {
                base
            } else {
                rest.split('.').fold(base, |ty, p| self.member_type(&ty, p))
            });
        }
        if let Some(l) = canon.strip_prefix("let:") {
            let (head, rest) = l.split_once('.').unwrap_or((l, ""));
            let base = self.let_cache.get(head)?.clone();
            return Some(if rest.is_empty() {
                base
            } else {
                rest.split('.').fold(base, |ty, p| self.member_type(&ty, p))
            });
        }
        None
    }
    fn check_match_arm_header(
        &mut self,
        ctx: &MatchSubjectCtx<'_>,
        branch: &V,
        match_span: Option<Span>,
        cursor: &mut Option<Span>,
        group_guard_fallback: bool,
    ) -> HashMap<String, Ty> {
        let remaining_ty = self.subtract_covered_atoms(ctx.subject_ty, &ctx.covered_atoms);
        let mut arm_narrowing = HashMap::new();
        let (pat_found, narrowed_subject) = self.check_pattern(
            &branch["pattern"],
            ctx.subject_ty,
            &remaining_ty,
            match_span,
            *cursor,
        );
        if pat_found.is_some() {
            *cursor = pat_found;
        }
        if let Some(canon) = ctx.subject_canon.as_deref()
            && let Some(narrowed) = narrowed_subject
            && narrowed.known()
            && narrowed != *ctx.subject_ty
        {
            arm_narrowing.insert(canon.into(), narrowed);
        } else if !ctx.tuple_slots.is_empty() {
            if branch["pattern"]["expr"] == "_" {
                for (i, (slot_canon, slot_ty)) in ctx.tuple_slots.iter().enumerate() {
                    if let Some(canon) = slot_canon
                        && let Some(cov) = ctx.covered_tuple_atoms.get(i)
                    {
                        let rem = self.subtract_covered_atoms(slot_ty, cov);
                        if rem.known() && rem != *slot_ty {
                            arm_narrowing.insert(canon.clone(), rem);
                        }
                    }
                }
            } else if let Some(pats) = branch["pattern"].as_array()
                && pats.len() == ctx.tuple_slots.len()
            {
                for (i, ((slot_canon, slot_ty), pat_item)) in
                    ctx.tuple_slots.iter().zip(pats.iter()).enumerate()
                {
                    let Some(canon) = slot_canon else { continue };
                    let rem = ctx.covered_tuple_atoms.get(i).map_or_else(
                        || slot_ty.clone(),
                        |cov| self.subtract_covered_atoms(slot_ty, cov),
                    );
                    if let Some(narrowed) = self.narrow_pattern_only(pat_item, &rem)
                        && narrowed.known()
                        && narrowed != *slot_ty
                    {
                        arm_narrowing.insert(canon.clone(), narrowed);
                    }
                }
            }
        }
        let saved = self.apply_narrowing(&arm_narrowing);
        if let Some(guard) = branch.get("guard") {
            let guard_found = if group_guard_fallback && match_span.is_none() {
                root_span(self.spans, guard, None)
            } else {
                self.child_after(guard, match_span, *cursor)
            };
            if guard_found.is_some() {
                *cursor = guard_found;
            }
            let guard_span = guard_found.or(match_span);
            let (guard_ty, guard_facts) = self.infer_with_facts(guard, guard_span);
            self.check(
                &guard_ty,
                &Ty::Bool,
                "incompatible-condition",
                guard_span,
                format_args!("Guard match"),
            );
            for (k, v) in guard_facts.when_true {
                self.narrowed.insert(k, v);
            }
        }
        saved
    }
    fn record_unguarded_branch_coverage(
        branch: &V,
        covered_atoms: &mut Vec<String>,
        covered_tuple_atoms: &mut [Vec<String>],
    ) {
        if branch.get("guard").is_some() {
            return;
        }
        Self::collect_pattern_atoms(&branch["pattern"], covered_atoms);
        if !covered_tuple_atoms.is_empty()
            && let Some(pats) = branch["pattern"].as_array()
            && pats.len() == covered_tuple_atoms.len()
        {
            for i in 0..pats.len() {
                if pats
                    .iter()
                    .enumerate()
                    .all(|(k, p)| k == i || p["expr"] == "_")
                {
                    Self::collect_pattern_atoms(&pats[i], &mut covered_tuple_atoms[i]);
                }
            }
        }
    }
    fn narrow_pattern_only(&self, pattern: &V, remaining: &Ty) -> Option<Ty> {
        if !remaining.known() {
            return None;
        }
        if pattern["expr"] == "_" {
            return Some(remaining.clone());
        }
        if let Some(alts) = pattern["or"].as_array() {
            let mut types = Vec::with_capacity(alts.len());
            for alt in alts {
                types.push(self.narrow_pattern_only(alt, remaining)?);
            }
            return Some(union(types));
        }
        if pattern.is_null() {
            return Some(Ty::Null);
        }
        if pattern.is_boolean() {
            return narrow_by_type(remaining, &Ty::Bool).or_else(|| Some(remaining.clone()));
        }
        if pattern.is_number()
            || (pattern["expression"]["kind"] == "binary"
                && ["..", "..="]
                    .contains(&pattern["expression"]["operator"].as_str().unwrap_or("")))
        {
            return narrow_by_type(&non_null(remaining.clone()), &Ty::Number)
                .or_else(|| narrow_non_null(remaining));
        }
        if pattern.is_string() {
            return narrow_by_type(&non_null(remaining.clone()), &Ty::String)
                .or_else(|| narrow_non_null(remaining));
        }
        if pattern.get("comparison").is_some() {
            return narrow_non_null(remaining);
        }
        if let Some(raw) = pattern["expr"].as_str()
            && let Some((head, rest)) = raw.split_once('.')
            && !rest.contains('.')
            && self.has_enum(head)
        {
            return narrow_by_type(remaining, &Ty::Named(head.into()))
                .or_else(|| Some(Ty::Named(head.into())));
        }
        None
    }
    fn infer_match_value(&mut self, value: &V, span: Option<Span>, expected: Option<Ty>) -> Ty {
        let subject = &value["match"];
        let subject_found = self.child(subject, span);
        let subject_span = subject_found.or(span);
        let subject_ty = self.infer(subject, subject_span);
        let mut match_ctx = MatchSubjectCtx::new(self, subject, &subject_ty);
        let branches = value["branches"].as_array().map_or(&[][..], Vec::as_slice);
        let mut branch_types = Vec::new();
        let mut valid = true;
        let mut cursor = subject_found;
        for branch in branches {
            let saved_narrowed =
                self.check_match_arm_header(&match_ctx, branch, span, &mut cursor, false);
            let result = &branch["value"];
            let result_found = self.child_after(result, span, cursor);
            if result_found.is_some() {
                cursor = result_found;
            }
            let result_span = result_found.or(span);
            self.expected = expected.clone();
            let ty = self.infer(result, result_span);
            self.narrowed = saved_narrowed;
            if let Some(expected) = &expected {
                valid &= self.check(
                    &ty,
                    expected,
                    "incompatible-match-branch",
                    result_span,
                    format_args!("Ветвь match"),
                );
            }
            branch_types.push(ty);
            match_ctx.record_unguarded(branch);
        }
        self.expected = None;
        self.check_match_exhaustiveness(&subject_ty, branches, span);
        if valid {
            union(branch_types)
        } else {
            Ty::Unknown
        }
    }
    fn check_pattern(
        &mut self,
        pattern: &V,
        subject: &Ty,
        remaining: &Ty,
        parent: Option<Span>,
        after: Option<Span>,
    ) -> (Option<Span>, Option<Ty>) {
        let found = self.child_after(pattern, parent, after);
        if pattern["expr"] == "_" {
            return (found, remaining.known().then(|| remaining.clone()));
        }
        let span = found.or(parent);
        if let Some(alternatives) = pattern["or"].as_array() {
            let mut sub_after = None;
            let mut narrowed_alts = Vec::with_capacity(alternatives.len());
            let mut all_narrowed = true;
            for alternative in alternatives {
                let (alt_found, alt_ty) =
                    self.check_pattern(alternative, subject, remaining, span, sub_after);
                if alt_found.is_some() {
                    sub_after = alt_found;
                }
                if let Some(t) = alt_ty {
                    narrowed_alts.push(t);
                } else {
                    all_narrowed = false;
                }
            }
            let narrowed = if all_narrowed && !narrowed_alts.is_empty() {
                Some(union(narrowed_alts))
            } else {
                remaining.known().then(|| remaining.clone())
            };
            return (found, narrowed);
        }
        if pattern.get("comparison").is_some() {
            let value = &pattern["value"];
            let value_span = self.child(value, span).or(span);
            let actual = self.infer(value, value_span);
            if !matches!(subject, Ty::Unknown | Ty::Any)
                && compatible(&actual, subject) == Compatible::No
                && compatible(subject, &actual) == Compatible::No
            {
                self.emit(
                    "incompatible-match-pattern",
                    subject.display(),
                    actual.display(),
                    value_span,
                    "Сравнение в match",
                );
            }
            let narrowed = narrow_by_type(&non_null(remaining.clone()), &actual)
                .or_else(|| narrow_non_null(remaining))
                .or_else(|| remaining.known().then(|| remaining.clone()));
            return (found, narrowed);
        }
        if pattern["expression"]["kind"] == "binary"
            && ["..", "..="].contains(&pattern["expression"]["operator"].as_str().unwrap_or(""))
        {
            self.infer(pattern, span);
            self.check(
                subject,
                &Ty::Number,
                "incompatible-match-pattern",
                span,
                format_args!("Диапазон в match"),
            );
            let narrowed = narrow_by_type(&non_null(remaining.clone()), &Ty::Number)
                .or_else(|| narrow_non_null(remaining))
                .or_else(|| remaining.known().then(|| remaining.clone()));
            return (found, narrowed);
        }
        if let Some(items) = pattern.as_array() {
            let extract_list = |ty: &Ty| match ty {
                Ty::List(elem) => (**elem).clone(),
                Ty::Union(ts) => {
                    let list_elems = ts
                        .iter()
                        .filter_map(|t| match t {
                            Ty::List(elem) => Some((**elem).clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if list_elems.is_empty() {
                        Ty::Unknown
                    } else {
                        union(list_elems)
                    }
                }
                _ => Ty::Unknown,
            };
            let item_subject = extract_list(subject);
            let item_remaining = extract_list(remaining);
            if !matches!(subject, Ty::Unknown | Ty::Any | Ty::List(_))
                && !matches!(subject, Ty::Union(ts) if ts.iter().any(|t| matches!(t, Ty::List(_) | Ty::Unknown | Ty::Any)))
            {
                self.emit(
                    "incompatible-match-pattern",
                    subject.display(),
                    "List".into(),
                    span,
                    "Паттерн match",
                );
            }
            let mut sub_after = None;
            for item in items {
                let (item_found, _) =
                    self.check_pattern(item, &item_subject, &item_remaining, span, sub_after);
                if item_found.is_some() {
                    sub_after = item_found;
                }
            }
            let narrowed = narrow_by_type(remaining, &Ty::List(Box::new(Ty::Any)))
                .or_else(|| narrow_non_null(remaining))
                .or_else(|| remaining.known().then(|| remaining.clone()));
            return (found, narrowed);
        }
        let actual = self.infer(pattern, span);
        if matches!(subject, Ty::Unknown | Ty::Any) || matches!(actual, Ty::Unknown | Ty::Any) {
            return (found, remaining.known().then(|| remaining.clone()));
        }
        let possible = match subject {
            Ty::Union(types) => types.iter().any(|expected| {
                compatible(&actual, expected) == Compatible::Yes
                    || compatible(expected, &actual) == Compatible::Yes
            }),
            expected => {
                compatible(&actual, expected) == Compatible::Yes
                    || compatible(expected, &actual) == Compatible::Yes
            }
        };
        if !possible {
            self.emit(
                "incompatible-match-pattern",
                subject.display(),
                actual.display(),
                span,
                "Паттерн match",
            );
        }
        let narrowed = if pattern.is_null() || actual == Ty::Null {
            Some(Ty::Null)
        } else {
            narrow_by_type(remaining, &actual)
                .or_else(|| remaining.known().then(|| remaining.clone()))
        };
        (found, narrowed)
    }
    fn finite_domain_atoms(&self, ty: &Ty, out: &mut Vec<String>) -> bool {
        match ty {
            Ty::Bool => {
                for atom in ["true", "false"] {
                    if !out.iter().any(|x| x == atom) {
                        out.push(atom.into());
                    }
                }
                true
            }
            Ty::Null => {
                if !out.iter().any(|x| x == "null") {
                    out.push("null".into());
                }
                true
            }
            Ty::Named(name) => {
                let Some(variants) = self.enum_variants(name) else {
                    return false;
                };
                for v in variants {
                    let atom = format!("{name}.{v}");
                    if !out.iter().any(|x| x == &atom) {
                        out.push(atom);
                    }
                }
                true
            }
            Ty::Union(types) => types.iter().all(|t| self.finite_domain_atoms(t, out)),
            _ => false,
        }
    }
    fn collect_pattern_atoms(pattern: &V, covered: &mut Vec<String>) -> bool {
        if pattern["expr"] == "_" {
            return true;
        }
        if let Some(alternatives) = pattern["or"].as_array() {
            let mut has_wildcard = false;
            for alt in alternatives {
                if Self::collect_pattern_atoms(alt, covered) {
                    has_wildcard = true;
                }
            }
            return has_wildcard;
        }
        let atom = if pattern.is_null() {
            Some("null".to_string())
        } else if let Some(b) = pattern.as_bool() {
            Some(b.to_string())
        } else if let Some(raw) = pattern["expr"].as_str() {
            let (head, rest) = raw.split_once('.').unwrap_or((raw, ""));
            if !rest.is_empty()
                && !rest.contains('.')
                && head.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
            {
                Some(raw.to_string())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(atom) = atom
            && !covered.iter().any(|x| x == &atom)
        {
            covered.push(atom);
        }
        false
    }
    fn check_match_exhaustiveness(&mut self, subject_ty: &Ty, branches: &[V], span: Option<Span>) {
        if branches.is_empty() {
            return;
        }
        let mut required = Vec::new();
        if !self.finite_domain_atoms(subject_ty, &mut required) || required.is_empty() {
            return;
        }
        let mut covered = Vec::new();
        for branch in branches {
            if branch.get("guard").is_some() {
                continue;
            }
            if Self::collect_pattern_atoms(&branch["pattern"], &mut covered) {
                return;
            }
        }
        let missing = required
            .into_iter()
            .filter(|req| !covered.iter().any(|c| c == req))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let covered_display = if covered.is_empty() {
                "∅".into()
            } else {
                covered.join(" | ")
            };
            self.emit(
                "non-exhaustive-match",
                missing.join(" | "),
                covered_display,
                span,
                "Неполный match",
            );
        }
    }
    fn infer_call(
        &mut self,
        name: &str,
        raw_args: &[V],
        args: &[Ty],
        spans: &[Option<Span>],
        call_span: Option<Span>,
    ) -> Ty {
        let Some((min_arity, max_arity, arity_label)) = pure_function_arity(name) else {
            self.emit(
                "unknown-function",
                "чистая функция".into(),
                name.into(),
                call_span,
                "Вызов функции",
            );
            return Ty::Unknown;
        };
        let arity_ok =
            args.len() >= min_arity && max_arity.is_none_or(|max_len| args.len() <= max_len);
        if !arity_ok {
            self.emit(
                "invalid-argument-count",
                arity_label.into(),
                args.len().to_string(),
                call_span,
                &format!("Вызов {name}"),
            );
        }
        let expected = |index: usize, ty: Ty| Some((index, ty));
        let mut requirements = Vec::new();
        let list_any = || Ty::List(Box::new(Ty::Any));
        let object = || {
            union([
                Ty::Map(Box::new(Ty::Any)),
                Ty::Record(Box::new(Ty::Any)),
                Ty::Object,
            ])
        };
        let scalar = || {
            union([
                Ty::Null,
                Ty::Bool,
                Ty::Number,
                Ty::String,
                Ty::Color,
                Ty::Length,
                Ty::Duration,
                Ty::Angle,
            ])
        };
        let result = match name {
            "String" => {
                if let Some(actual) = args.first()
                    && !self.is_scalar(actual)
                {
                    requirements.push(expected(0, scalar()));
                }
                Ty::String
            }
            "Number" => {
                requirements.push(expected(0, union([Ty::Number, Ty::String])));
                Ty::Number
            }
            "Bool" => {
                requirements.push(expected(0, Ty::Bool));
                Ty::Bool
            }
            "len" => {
                requirements.push(expected(0, union([Ty::String, list_any(), object()])));
                Ty::Int
            }
            "isEmpty" => Ty::Bool,
            "count" => {
                requirements.push(expected(0, list_any()));
                Ty::Int
            }
            "any" | "all" => {
                requirements.push(expected(0, Ty::List(Box::new(Ty::Bool))));
                Ty::Bool
            }
            "pluck" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::String)]);
                if let (Some(Ty::List(item)), Some(field)) =
                    (args.first(), raw_args.get(1).and_then(V::as_str))
                {
                    let field_ty = match item.as_ref() {
                        Ty::Map(v) | Ty::Record(v) => union([(**v).clone(), Ty::Null]),
                        other => self.resolve_member(
                            other,
                            field,
                            false,
                            spans.get(1).copied().flatten().or(call_span),
                        ),
                    };
                    if field_ty == Ty::Unknown {
                        list_any()
                    } else {
                        Ty::List(Box::new(field_ty))
                    }
                } else {
                    list_any()
                }
            }
            "groupBy" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::String)]);
                if let (Some(Ty::List(item)), Some(field)) =
                    (args.first(), raw_args.get(1).and_then(V::as_str))
                {
                    self.check_struct_field(
                        item,
                        field,
                        spans.get(1).copied().flatten().or(call_span),
                    );
                }
                Ty::List(Box::new(Ty::Object))
            }
            "sortBy" | "unique" => {
                requirements.push(expected(0, list_any()));
                if args.len() > 1 {
                    requirements.push(expected(1, Ty::String));
                    if let (Some(Ty::List(item)), Some(field)) =
                        (args.first(), raw_args.get(1).and_then(V::as_str))
                    {
                        self.check_struct_field(
                            item,
                            field,
                            spans.get(1).copied().flatten().or(call_span),
                        );
                    }
                }
                args.first().cloned().unwrap_or(Ty::Unknown)
            }
            "alpha" | "lighten" | "darken" | "saturate" | "desaturate" => {
                requirements.extend([expected(0, Ty::Color), expected(1, Ty::Number)]);
                Ty::Color
            }
            "invert" => {
                requirements.push(expected(0, Ty::Color));
                Ty::Color
            }
            "contrast" => {
                requirements.extend([expected(0, Ty::Color), expected(1, Ty::Color)]);
                Ty::Number
            }
            "mix" => {
                requirements.extend([expected(0, Ty::Color), expected(1, Ty::Color)]);
                if args.len() > 2 {
                    requirements.push(expected(2, Ty::Number));
                }
                Ty::Color
            }
            "min" | "max" => {
                for index in 0..args.len() {
                    requirements.push(expected(index, Ty::Number));
                }
                if !args.is_empty() && args.iter().all(|t| *t == Ty::Int) {
                    Ty::Int
                } else {
                    Ty::Number
                }
            }
            "clamp" => {
                for index in 0..3 {
                    requirements.push(expected(index, Ty::Number));
                }
                if arity_ok
                    && let (Some(lo), Some(hi)) = (
                        raw_args.get(1).and_then(V::as_f64),
                        raw_args.get(2).and_then(V::as_f64),
                    )
                    && lo > hi
                {
                    self.emit(
                        "invalid-range",
                        format!("min <= max (получено {lo} > {hi})"),
                        format!("{lo}..{hi}"),
                        call_span,
                        "clamp",
                    );
                }
                if args.len() == 3 && args.iter().all(|t| *t == Ty::Int) {
                    Ty::Int
                } else {
                    Ty::Number
                }
            }
            "lerp" => {
                for index in 0..3 {
                    requirements.push(expected(index, Ty::Number));
                }
                Ty::Number
            }
            "remap" => {
                for index in 0..5 {
                    requirements.push(expected(index, Ty::Number));
                }
                if arity_ok
                    && let (Some(i0), Some(i1)) = (
                        raw_args.get(1).and_then(V::as_f64),
                        raw_args.get(2).and_then(V::as_f64),
                    )
                    && i0 == i1
                {
                    self.emit(
                        "invalid-range",
                        "inMin != inMax".into(),
                        format!("{i0}..{i1}"),
                        call_span,
                        "remap",
                    );
                }
                Ty::Number
            }
            "pow" => {
                requirements.extend([expected(0, Ty::Number), expected(1, Ty::Number)]);
                Ty::Number
            }
            "round" | "floor" | "ceil" | "sign" => {
                requirements.push(expected(0, Ty::Number));
                Ty::Int
            }
            "abs" => {
                requirements.push(expected(0, Ty::Number));
                if args.first() == Some(&Ty::Int) {
                    Ty::Int
                } else {
                    Ty::Number
                }
            }
            "sqrt" => {
                requirements.push(expected(0, Ty::Number));
                Ty::Number
            }
            "avg" | "sum" => {
                requirements.push(expected(0, Ty::List(Box::new(Ty::Number))));
                Ty::Number
            }
            "trim" | "upper" | "lower" | "capitalize" => {
                requirements.push(expected(0, Ty::String));
                Ty::String
            }
            "padStart" | "padEnd" | "truncate" => {
                requirements.extend([expected(0, Ty::String), expected(1, Ty::Number)]);
                if args.len() > 2 {
                    requirements.push(expected(2, Ty::String));
                }
                Ty::String
            }
            "repeat" => {
                requirements.extend([expected(0, Ty::String), expected(1, Ty::Number)]);
                Ty::String
            }
            "contains" | "includes" | "indexOf" => {
                if let Some(actual) = args.first() {
                    if actual == &Ty::String {
                        requirements.push(expected(1, Ty::String));
                    } else if !matches!(actual, Ty::List(_) | Ty::Unknown | Ty::Any) {
                        requirements.push(expected(0, union([Ty::String, list_any()])));
                    }
                }
                if name == "indexOf" { Ty::Int } else { Ty::Bool }
            }
            "startsWith" | "endsWith" | "split" => {
                requirements.extend([expected(0, Ty::String), expected(1, Ty::String)]);
                if name == "split" {
                    Ty::List(Box::new(Ty::String))
                } else {
                    Ty::Bool
                }
            }
            "replace" => {
                for index in 0..3 {
                    requirements.push(expected(index, Ty::String));
                }
                Ty::String
            }
            "slice" => {
                requirements.push(expected(0, union([Ty::String, list_any()])));
                requirements.push(expected(1, Ty::Number));
                if args.len() > 2 {
                    requirements.push(expected(2, Ty::Number));
                }
                args.first().cloned().unwrap_or(Ty::Unknown)
            }
            "join" => {
                requirements.push(expected(0, list_any()));
                if args.len() > 1 {
                    requirements.push(expected(1, Ty::String));
                }
                Ty::String
            }
            "reverse" => {
                requirements.push(expected(0, union([Ty::String, list_any()])));
                args.first().cloned().unwrap_or(Ty::Unknown)
            }
            "take" | "skip" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::Number)]);
                args.first().cloned().unwrap_or(Ty::Unknown)
            }
            "range" => {
                requirements.extend([expected(0, Ty::Number), expected(1, Ty::Number)]);
                Ty::List(Box::new(Ty::Int))
            }
            "formatNumber" => {
                requirements.push(expected(0, Ty::Number));
                if args.len() > 1 {
                    requirements.push(expected(1, Ty::Number));
                }
                Ty::String
            }
            "find" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::String)]);
                if let (Some(Ty::List(item)), Some(field)) =
                    (args.first(), raw_args.get(1).and_then(V::as_str))
                {
                    let field_ty = self.resolve_member(
                        item,
                        field,
                        false,
                        spans.get(1).copied().flatten().or(call_span),
                    );
                    if arity_ok
                        && let Some(val_ty) = args.get(2)
                        && !matches!(field_ty, Ty::Unknown | Ty::Any)
                        && !matches!(val_ty, Ty::Unknown | Ty::Any)
                        && compatible(val_ty, &field_ty) == Compatible::No
                        && compatible(&field_ty, val_ty) == Compatible::No
                    {
                        self.emit(
                            "incompatible-argument",
                            field_ty.display(),
                            val_ty.display(),
                            spans.get(2).copied().flatten().or(call_span),
                            "find: аргумент 3",
                        );
                    }
                }
                match args.first() {
                    Some(Ty::List(item)) => union([(**item).clone(), Ty::Null]),
                    _ => Ty::Unknown,
                }
            }
            "compact" => {
                requirements.push(expected(0, list_any()));
                match args.first() {
                    Some(Ty::List(item)) => Ty::List(Box::new(non_null((**item).clone()))),
                    _ => list_any(),
                }
            }
            "flatten" => {
                requirements.push(expected(0, list_any()));
                match args.first() {
                    Some(Ty::List(item)) => Ty::List(Box::new(unwrap_list_once(item))),
                    _ => list_any(),
                }
            }
            "chunk" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::Number)]);
                match args.first() {
                    Some(Ty::List(item)) => Ty::List(Box::new(Ty::List(item.clone()))),
                    _ => Ty::Unknown,
                }
            }
            "zip" => {
                requirements.extend([expected(0, list_any()), expected(1, list_any())]);
                match (args.first(), args.get(1)) {
                    (Some(Ty::List(a)), Some(Ty::List(b))) => {
                        Ty::List(Box::new(Ty::List(Box::new(union([
                            (**a).clone(),
                            (**b).clone(),
                        ])))))
                    }
                    _ => list_any(),
                }
            }
            "first" | "last" => {
                requirements.push(expected(0, list_any()));
                match args.first() {
                    Some(Ty::List(item)) => union([(**item).clone(), Ty::Null]),
                    _ => Ty::Unknown,
                }
            }
            "keys" => {
                requirements.push(expected(0, object()));
                Ty::List(Box::new(Ty::String))
            }
            "values" => {
                requirements.push(expected(0, object()));
                match args.first() {
                    Some(Ty::Map(item) | Ty::Record(item)) => Ty::List(item.clone()),
                    _ => list_any(),
                }
            }
            "entries" => {
                requirements.push(expected(0, object()));
                match args.first() {
                    Some(Ty::Map(item) | Ty::Record(item)) => {
                        Ty::List(Box::new(Ty::List(Box::new(union([
                            Ty::String,
                            (**item).clone(),
                        ])))))
                    }
                    _ => list_any(),
                }
            }
            "fromEntries" => {
                requirements.push(expected(0, list_any()));
                Ty::Record(Box::new(Ty::Unknown))
            }
            "merge" => {
                for index in 0..args.len() {
                    requirements.push(expected(index, object()));
                }
                let mut val_types = Vec::new();
                let mut all_typed = !args.is_empty();
                for arg in args {
                    match arg {
                        Ty::Map(v) | Ty::Record(v) => val_types.push((**v).clone()),
                        _ => {
                            all_typed = false;
                            break;
                        }
                    }
                }
                if all_typed {
                    Ty::Record(Box::new(union(val_types)))
                } else {
                    Ty::Record(Box::new(Ty::Unknown))
                }
            }
            "pick" | "omit" => {
                requirements.extend([
                    expected(0, object()),
                    expected(1, Ty::List(Box::new(Ty::String))),
                ]);
                if let Some(Ty::Named(struct_name)) = args.first()
                    && let Some(keys) = raw_args.get(1).and_then(V::as_array)
                {
                    let base = Ty::Named(struct_name.clone());
                    let key_parent = spans.get(1).copied().flatten().or(call_span);
                    let mut prev_span = None;
                    for key_val in keys {
                        let key_span = self.child_after(key_val, key_parent, prev_span);
                        if key_span.is_some() {
                            prev_span = key_span;
                        }
                        if let Some(k) = key_val.as_str() {
                            self.check_struct_field(&base, k, key_span.or(key_parent));
                        }
                    }
                }
                match args.first() {
                    Some(Ty::Map(v) | Ty::Record(v)) => Ty::Record(v.clone()),
                    Some(Ty::Named(struct_name)) if name == "pick" => {
                        if let Some(keys) = raw_args.get(1).and_then(V::as_array)
                            && !keys.is_empty()
                            && keys.iter().all(V::is_string)
                        {
                            let base = Ty::Named(struct_name.clone());
                            let picked = keys
                                .iter()
                                .filter_map(V::as_str)
                                .map(|k| self.member_type(&base, k));
                            Ty::Record(Box::new(union(picked)))
                        } else {
                            Ty::Record(Box::new(Ty::Unknown))
                        }
                    }
                    _ => Ty::Record(Box::new(Ty::Unknown)),
                }
            }
            "coalesce" => union(args.iter().cloned().map(non_null)),
            "plural" => {
                requirements.push(expected(0, Ty::Number));
                for (index, arg) in args.iter().enumerate().skip(1) {
                    if !self.is_scalar(arg) {
                        requirements.push(expected(index, scalar()));
                    }
                }
                Ty::String
            }
            "isValid" => {
                if args.len() == 2 {
                    requirements.push(expected(1, Ty::String));
                } else {
                    requirements.extend([expected(1, Ty::Number), expected(2, Ty::Number)]);
                }
                Ty::Bool
            }
            _ => Ty::Unknown,
        };
        if arity_ok {
            for (index, expected) in requirements.into_iter().flatten() {
                if let Some(actual) = args.get(index) {
                    self.check(
                        actual,
                        &expected,
                        "incompatible-argument",
                        spans.get(index).copied().flatten().or(call_span),
                        format_args!("{name}: аргумент {}", index + 1),
                    );
                }
            }
        }
        result
    }
    fn property_type(&self, node: &str, name: &str) -> Option<Ty> {
        if node == self.doc["name"].as_str().unwrap_or("")
            && let Some(ty) = self.declared_prop_type_str(name)
        {
            return Some(parse_type(ty));
        }
        self.context.property_type(node, name).map(parse_type)
    }
    fn match_group_guarantees_prop(&self, group: &V, prop: &str) -> bool {
        let Some(branches) = group["branches"].as_array() else {
            return false;
        };
        if branches.is_empty() || !branches.iter().all(|b| b["props"].get(prop).is_some()) {
            return false;
        }
        let mut covered = Vec::new();
        for branch in branches {
            if branch.get("guard").is_some() {
                continue;
            }
            if Self::collect_pattern_atoms(&branch["pattern"], &mut covered) {
                return true;
            }
        }
        let Some(canon) = self.extract_canonical_path(&group["match"]) else {
            return false;
        };
        let Some(subject_ty) = self.lookup_canonical_type(&canon) else {
            return false;
        };
        let mut required = Vec::new();
        self.finite_domain_atoms(&subject_ty, &mut required)
            && !required.is_empty()
            && required.iter().all(|req| covered.iter().any(|c| c == req))
    }
    fn node_supplies_prop(&self, node: &V, prop: &str) -> bool {
        node["props"].get(prop).is_some()
            || node["bindings"].get(prop).is_some()
            || node["forward"]
                .as_array()
                .is_some_and(|f| f.iter().any(|x| x.as_str() == Some(prop)))
            || node["matches"].as_array().is_some_and(|groups| {
                groups
                    .iter()
                    .any(|g| self.match_group_guarantees_prop(g, prop))
            })
    }
    fn check_required_component_props(&mut self, node: &V, node_type: &str) {
        if self.in_design_override || node_type == "Patch" {
            return;
        }
        let Some(required) = self.context.required_component_props.get(node_type) else {
            return;
        };
        if required.is_empty() {
            return;
        }
        let mut missing = required
            .iter()
            .filter(|req| !self.node_supplies_prop(node, req))
            .cloned()
            .collect::<Vec<_>>();
        missing.sort_unstable();
        for req in missing {
            let expected_ty = self
                .context
                .property_type(node_type, &req)
                .unwrap_or("Any")
                .to_string();
            self.emit_message(
                "missing-required-prop",
                format!("Отсутствует обязательное свойство {node_type}.{req} ({expected_ty})"),
                format!("{req}: {expected_ty}"),
                "∅".into(),
                node_range(node),
                &format!("Компонент {node_type}"),
            );
        }
    }
    fn walk_props(&mut self, node: &V, node_type: &str) {
        self.check_required_component_props(node, node_type);
        self.walk_props_fields(
            &node["props"],
            &node["propertyRanges"],
            &node["statementRanges"],
            node_type,
        );
        if let Some(bindings) = node["bindings"].as_object() {
            for (name, path) in bindings {
                let Some(path) = path.as_str() else { continue };
                let range = ast_range(&node["statementRanges"][name]);
                let actual = self.infer_reference(path, range);
                if let Some(expected) = self.property_type(node_type, name) {
                    let forward = compatible(&actual, &expected);
                    let backward = compatible(&expected, &actual);
                    if forward == Compatible::No || backward == Compatible::No {
                        self.emit(
                            "incompatible-binding",
                            expected.display(),
                            actual.display(),
                            range,
                            &format!("Двусторонняя привязка {node_type}.{name}"),
                        );
                    }
                }
            }
        }
        if let Some(events) = node["events"].as_object() {
            for (event, action) in events {
                let Some(action) = action.as_str() else {
                    continue;
                };
                let args = node["eventArgs"][event]
                    .as_array()
                    .map_or(&[][..], Vec::as_slice);
                self.walk_action(action, args, ast_range(&node["statementRanges"][event]));
            }
        }
        if let Some(args) = node["slotArgs"].as_array() {
            for arg in args {
                self.infer_root(&arg["value"], node_range(node));
            }
        }
        if let Some(slots) = node["slots"].as_object() {
            for value in slots.values() {
                self.walk_slot_value(value, node_range(node));
            }
        }
        self.walk_groups(&node["matches"], node_type);
    }
    fn walk_slot_value(&mut self, value: &V, range: Option<Span>) {
        if value.get("propertyOverride").is_some() || value.get("fileOverride").is_some() {
            if !value["patch"].is_null() {
                self.walk_node(&value["patch"]);
            }
        } else if value.get("match").is_some() {
            let match_span = root_span(self.spans, value, range).or(range);
            let subject = &value["match"];
            let subject_found = self.child(subject, match_span);
            let subject_ty = self.infer(subject, subject_found.or(match_span));
            let mut match_ctx = MatchSubjectCtx::new(self, subject, &subject_ty);
            let branches = value["branches"].as_array().map_or(&[][..], Vec::as_slice);
            let mut cursor = subject_found;
            for branch in branches {
                let saved_narrowed =
                    self.check_match_arm_header(&match_ctx, branch, match_span, &mut cursor, false);
                self.walk_slot_value(&branch["value"], match_span);
                self.narrowed = saved_narrowed;
                match_ctx.record_unguarded(branch);
            }
            self.check_match_exhaustiveness(&subject_ty, branches, match_span);
        } else if value["type"].is_string() {
            self.walk_node(value);
        } else {
            self.infer_root(value, range);
        }
    }
    fn walk_action(&mut self, action: &str, args: &[V], range: Option<Span>) {
        let signature = action
            .strip_prefix("events.")
            .and_then(|name| {
                self.doc["eventDefinitions"]
                    .get(name)
                    .and_then(V::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|arg| arg["type"].as_str())
                            .collect::<Vec<_>>()
                    })
                    .or_else(|| {
                        let base = self.doc["base"].as_str()?;
                        self.context
                            .component_events
                            .get(base)
                            .and_then(|ev| ev.get(name))
                            .map(|types| types.iter().map(String::as_str).collect::<Vec<_>>())
                    })
            })
            .or_else(|| {
                action.strip_prefix("actions.").and_then(|name| {
                    self.context
                        .actions
                        .get(name)
                        .map(|types| types.iter().map(String::as_str).collect::<Vec<_>>())
                })
            });
        let arity_ok = if let Some(types) = &signature
            && args.len() != types.len()
        {
            self.emit(
                "invalid-argument-count",
                types.len().to_string(),
                args.len().to_string(),
                range,
                &format!("Вызов {action}"),
            );
            false
        } else {
            true
        };
        let mut prev_span = None;
        for (index, arg) in args.iter().enumerate() {
            let found = if range.is_some() {
                self.child_after(arg, range, prev_span)
            } else {
                root_span(self.spans, arg, None)
            };
            if found.is_some() {
                prev_span = found;
            }
            let span = found.or(range);
            let actual = self.infer(arg, span);
            if arity_ok
                && let Some(expected) = signature
                    .as_deref()
                    .and_then(|types| types.get(index))
                    .map(|ty| parse_type(ty))
            {
                self.check(
                    &actual,
                    &expected,
                    "incompatible-argument",
                    span,
                    format_args!("{action}: аргумент {}", index + 1),
                );
            }
        }
    }
    fn walk_groups(&mut self, groups: &V, node_type: &str) {
        for group in groups.as_array().into_iter().flatten() {
            let group_span = root_span(self.spans, group, None);
            let subject = &group["match"];
            let subject_found = if group_span.is_some() {
                self.child(subject, group_span)
            } else {
                root_span(self.spans, subject, None)
            };
            let subject_span = subject_found.or(group_span);
            let subject_ty = self.infer(subject, subject_span);
            let mut match_ctx = MatchSubjectCtx::new(self, subject, &subject_ty);
            let branches = group["branches"].as_array().map_or(&[][..], Vec::as_slice);
            let mut cursor = subject_found;
            for branch in branches {
                let saved_narrowed =
                    self.check_match_arm_header(&match_ctx, branch, group_span, &mut cursor, true);
                self.walk_props_fields(
                    &branch["props"],
                    &branch["propertyRanges"],
                    &branch["statementRanges"],
                    node_type,
                );
                self.narrowed = saved_narrowed;
                if let Some(stmt_ranges) = branch["statementRanges"].as_object() {
                    for r in stmt_ranges.values().filter_map(ast_range) {
                        if cursor.is_none_or(|c| r.to > c.to) {
                            cursor = Some(r);
                        }
                    }
                }
                match_ctx.record_unguarded(branch);
            }
            self.check_match_exhaustiveness(&subject_ty, branches, group_span);
        }
    }
    fn walk_props_fields(
        &mut self,
        props_val: &V,
        prop_ranges: &V,
        stmt_ranges: &V,
        node_type: &str,
    ) {
        if let Some(props) = props_val.as_object() {
            let doc = self.doc;
            let self_name = doc["name"].as_str().unwrap_or("");
            for (name, value) in props {
                let range = ast_range(&prop_ranges[name]).or_else(|| ast_range(&stmt_ranges[name]));
                let val_span = root_span(self.spans, value, range).or(range);
                if let Some(expected) = self.property_type(node_type, name) {
                    let actual = self.infer_expected(value, expected.clone(), range);
                    let ok = self.check(
                        &actual,
                        &expected,
                        "incompatible-binding",
                        val_span,
                        format_args!("Свойство {node_type}.{name}"),
                    );
                    if ok {
                        let constraints = if node_type == self_name {
                            self.declared_prop_constraints(name).cloned()
                        } else {
                            self.context
                                .component_prop_constraints
                                .get(node_type)
                                .and_then(|c| c.get(name))
                                .cloned()
                        };
                        if let Some(constraints) = constraints {
                            self.check_static_constraints(
                                value,
                                &constraints,
                                val_span,
                                &format!("Свойство {node_type}.{name}"),
                            );
                        }
                    }
                } else {
                    self.infer_root(value, range);
                }
            }
        }
    }
    fn walk_node(&mut self, node: &V) {
        match node["type"].as_str().unwrap_or("") {
            "Let" => {
                let ty = self.infer_root(&node["value"], node_range(node));
                if let Some(name) = node["name"].as_str() {
                    self.locals.insert(name.into(), ty);
                    self.clear_local_narrowing(name);
                }
            }
            "If" => {
                let condition = &node["condition"];
                let span =
                    root_span(self.spans, condition, node_range(node)).or_else(|| node_range(node));
                let (ty, cond_facts) = self.infer_with_facts(condition, span);
                self.check(
                    &ty,
                    &Ty::Bool,
                    "incompatible-condition",
                    span,
                    format_args!("Условие if"),
                );
                let saved_then_locals = self.locals.clone();
                let saved_then = self.apply_narrowing(&cond_facts.when_true);
                self.walk_nodes(&node["children"]);
                self.locals = saved_then_locals;
                self.narrowed = saved_then;
                let saved_else_locals = self.locals.clone();
                let saved_else = self.apply_narrowing(&cond_facts.when_false);
                self.walk_nodes(&node["elseChildren"]);
                self.locals = saved_else_locals;
                self.narrowed = saved_else;
            }
            "For" => {
                let collection = &node["collection"];
                let span = root_span(self.spans, collection, node_range(node))
                    .or_else(|| node_range(node));
                let ty = self.infer(collection, span);
                self.check(
                    &ty,
                    &Ty::List(Box::new(Ty::Any)),
                    "incompatible-binding",
                    span,
                    format_args!("Коллекция for"),
                );
                let saved_locals = self.locals.clone();
                let saved_narrowed = self.narrowed.clone();
                if let Some(name) = node["item"].as_str() {
                    let item = match ty {
                        Ty::List(item) => *item,
                        _ => Ty::Unknown,
                    };
                    self.locals.insert(name.into(), item);
                    self.clear_local_narrowing(name);
                }
                if let Some(index) = node["index"].as_str() {
                    self.locals.insert(index.into(), Ty::Int);
                    self.clear_local_narrowing(index);
                }
                if let Some(condition) = node.get("where") {
                    let span = root_span(self.spans, condition, node_range(node))
                        .or_else(|| node_range(node));
                    let (actual, where_facts) = self.infer_with_facts(condition, span);
                    self.check(
                        &actual,
                        &Ty::Bool,
                        "incompatible-condition",
                        span,
                        format_args!("Условие where"),
                    );
                    for (k, v) in where_facts.when_true {
                        self.narrowed.insert(k, v);
                    }
                }
                let key = &node["key"];
                let key_span =
                    root_span(self.spans, key, node_range(node)).or_else(|| node_range(node));
                let key_ty = self.infer(key, key_span);
                self.check(
                    &key_ty,
                    &union([Ty::String, Ty::Number]),
                    "incompatible-binding",
                    key_span,
                    format_args!("Ключ for"),
                );
                self.walk_nodes(&node["children"]);
                self.locals = saved_locals;
                self.narrowed = saved_narrowed;
                self.walk_nodes(&node["emptyChildren"]);
            }
            "Match" => {
                let match_span = node_range(node);
                let subject = &node["match"];
                let subject_found = self.child(subject, match_span);
                let subject_span = subject_found.or(match_span);
                let subject_ty = self.infer(subject, subject_span);
                let mut match_ctx = MatchSubjectCtx::new(self, subject, &subject_ty);
                let branches = node["branches"].as_array().map_or(&[][..], Vec::as_slice);
                let mut cursor = subject_found;
                for branch in branches {
                    let saved_locals = self.locals.clone();
                    let saved_narrowed = self.check_match_arm_header(
                        &match_ctx,
                        branch,
                        match_span,
                        &mut cursor,
                        false,
                    );
                    self.walk_nodes(&branch["children"]);
                    self.locals = saved_locals;
                    self.narrowed = saved_narrowed;
                    if let Some(children) = branch["children"].as_array() {
                        for child_node in children.iter().filter_map(node_range) {
                            if cursor.is_none_or(|c| child_node.to > c.to) {
                                cursor = Some(child_node);
                            }
                        }
                    }
                    match_ctx.record_unguarded(branch);
                }
                self.check_match_exhaustiveness(&subject_ty, branches, match_span);
            }
            node_type => {
                self.walk_props(node, node_type);
                let saved_locals = self.locals.clone();
                let saved_narrowed = self.narrowed.clone();
                self.walk_nodes(&node["children"]);
                self.locals = saved_locals;
                self.narrowed = saved_narrowed;
            }
        }
    }
    fn walk_nodes(&mut self, nodes: &V) {
        for node in nodes.as_array().into_iter().flatten() {
            self.walk_node(node);
        }
    }
    fn infer_declarations(&mut self) {
        let doc = self.doc;
        if let Some(defaults) = doc["defaults"].as_object() {
            let node_type = doc["name"].as_str().unwrap_or("");
            for (name, value) in defaults {
                if doc["lets"].get(name).is_some() {
                    self.infer_let(name);
                    continue;
                }
                let range = ast_range(&doc["defaultRanges"][name]);
                let val_span = root_span(self.spans, value, range).or(range);
                if let Some(expected) = self.property_type(node_type, name) {
                    let actual = self.infer_expected(value, expected.clone(), range);
                    let ok = self.check(
                        &actual,
                        &expected,
                        "incompatible-binding",
                        val_span,
                        format_args!("Default {node_type}.{name}"),
                    );
                    if ok && let Some(constraints) = self.declared_prop_constraints(name).cloned() {
                        self.check_static_constraints(
                            value,
                            &constraints,
                            val_span,
                            &format!("Default {node_type}.{name}"),
                        );
                    }
                } else {
                    self.infer_root(value, range);
                }
            }
        }
        if let Some(definitions) = doc["stateDefinitions"].as_object() {
            for (name, definition) in definitions {
                let range = ast_range(&doc["defaultRanges"][name]);
                let expected = parse_type(definition["type"].as_str().unwrap_or(""));
                let value = &definition["default"];
                let val_span = root_span(self.spans, value, range).or(range);
                let actual = self.infer_expected(value, expected.clone(), range);
                self.check(
                    &actual,
                    &expected,
                    "incompatible-binding",
                    val_span,
                    format_args!("Default state.{name}"),
                );
            }
        }
        if let Some(lets) = doc["lets"].as_object() {
            for name in lets.keys() {
                self.infer_let(name);
            }
        }
        for section in ["tokens", "i18n"] {
            if let Some(values) = doc[section].as_object() {
                for (key, value) in values {
                    if value.is_object()
                        && !value.as_object().unwrap().contains_key("expr")
                        && !value.as_object().unwrap().contains_key("expression")
                        && !value.as_object().unwrap().contains_key("interpolation")
                        && !value.as_object().unwrap().contains_key("match")
                        && values.keys().any(|other| {
                            other
                                .strip_prefix(key.as_str())
                                .is_some_and(|rest| rest.starts_with('.'))
                        })
                    {
                        continue;
                    }
                    let cache_key = format!("{section}.{key}");
                    self.infer_dictionary(section, key, &cache_key);
                }
            }
        }
        if let Some(styles) = doc["styles"].as_object() {
            for style in styles.values() {
                if style.get("props").is_some() {
                    self.walk_props_fields(
                        &style["props"],
                        &style["propertyRanges"],
                        &style["statementRanges"],
                        "*",
                    );
                } else {
                    self.walk_props_fields(style, &V::Null, &V::Null, "*");
                }
            }
        }
    }
    fn run(&mut self) {
        let doc = self.doc;
        self.infer_declarations();
        self.walk_nodes(&doc["nodes"]);
        self.walk_groups(&doc["matches"], doc["name"].as_str().unwrap_or(""));
        if let Some(values) = doc["slots"].as_object() {
            for value in values.values() {
                self.walk_slot_value(value, None);
            }
        }
        for shortcut in doc["shortcuts"].as_array().into_iter().flatten() {
            let args = shortcut["args"].as_array().map_or(&[][..], Vec::as_slice);
            self.walk_action(
                shortcut["action"].as_str().unwrap_or(""),
                args,
                ast_range(shortcut),
            );
        }
        self.in_design_override = true;
        self.walk_nodes(&doc["entries"]);
        for state in doc["states"].as_array().into_iter().flatten() {
            self.walk_nodes(&state["nodes"]);
        }
        self.in_design_override = false;
    }
}
fn unit_type(raw: &str) -> Option<Ty> {
    if !raw
        .as_bytes()
        .last()
        .is_some_and(|b| *b == b'%' || b.is_ascii_alphabetic())
        || !raw
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit() || matches!(*b, b'-' | b'+' | b'.'))
    {
        return None;
    }
    for (suffixes, ty) in [
        (
            &[
                "px", "%", "rem", "em", "vw", "vh", "vmin", "vmax", "ch", "fr",
            ][..],
            Ty::Length,
        ),
        (&["ms", "s"][..], Ty::Duration),
        (&["deg", "rad", "turn"][..], Ty::Angle),
    ] {
        for suffix in suffixes {
            if let Some(number) = raw.strip_suffix(suffix)
                && !number.is_empty()
                && number.parse::<f64>().is_ok()
            {
                return Some(ty);
            }
        }
    }
    None
}

pub(crate) fn analyze(doc: &V, spans: &SpanTable, context: &TypeContext) -> TypecheckReport {
    let mut analyzer = Analyzer::new(doc, spans, context);
    analyzer.run();
    TypecheckReport {
        diagnostics: analyzer.diagnostics,
        expressions_total: analyzer.expressions_total,
        expressions_typed: analyzer.expressions_typed,
    }
}

pub fn analyze_source(
    source: &str,
    context: &TypeContext,
) -> Result<TypecheckReport, crate::Diagnostic> {
    let (doc, spans) =
        crate::frontend::parse_with_span_table(source).map_err(crate::Diagnostic::from)?;
    Ok(analyze(&doc, &spans, context))
}
