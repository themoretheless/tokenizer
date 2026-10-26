//! Conservative static expression types. Unknown host information suppresses
//! diagnostics; the transport AST and runtime evaluator are deliberately unchanged.
use crate::evaluate;
use crate::frontend::SpanTable;
use serde_json::{Value as V, json};
use std::collections::{HashMap, HashSet};

pub const BUILTIN_PROPERTY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct TypeContext {
    properties: HashMap<String, HashMap<String, String>>,
    components: HashMap<String, HashMap<String, String>>,
    actions: HashMap<String, Vec<String>>,
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
            actions: HashMap::new(),
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
    pub fn set_action(&mut self, name: &str, arguments: Vec<String>) {
        self.actions.insert(name.into(), arguments);
    }
    pub fn property_type(&self, node: &str, name: &str) -> Option<&str> {
        self.components
            .get(node)
            .and_then(|p| p.get(name))
            .or_else(|| self.properties.get(node).and_then(|p| p.get(name)))
            .or_else(|| self.properties.get("*").and_then(|p| p.get(name)))
            .map(String::as_str)
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
        for (section, component) in [("properties", false), ("components", true)] {
            if let Some(nodes) = root.get(section).and_then(V::as_object) {
                for (node, props) in nodes {
                    let Some(props) = props.as_object() else {
                        return Err(format!("TypeContext.{section}.{node} должен быть объектом"));
                    };
                    for (name, ty) in props {
                        let Some(ty) = ty.as_str() else {
                            return Err(format!("Тип {node}.{name} должен быть строкой"));
                        };
                        if component {
                            context.set_component_property(node, name, ty);
                        } else {
                            context.set_property(node, name, ty);
                        }
                    }
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
fn union(types: impl IntoIterator<Item = Ty>) -> Ty {
    let mut out = Vec::new();
    for ty in types {
        if ty == Ty::Unknown {
            return Ty::Unknown;
        }
        let alternatives = match ty {
            Ty::Union(ts) => ts,
            other => vec![other],
        };
        for ty in alternatives {
            if !out.contains(&ty) {
                out.push(ty);
            }
        }
    }
    match out.len() {
        0 => Ty::Unknown,
        1 => out.pop().unwrap(),
        _ => Ty::Union(out),
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
    match raw {
        "Unknown" | "" => Ty::Unknown,
        "Any" => Ty::Any,
        "Bool" | "Boolean" => Ty::Bool,
        "Number" | "Float" => Ty::Number,
        "Int" => Ty::Int,
        "String" | "Asset" => Ty::String,
        "Color" => Ty::Color,
        "Length" => Ty::Length,
        "Duration" => Ty::Duration,
        "Angle" => Ty::Angle,
        "List" => Ty::List(Box::new(Ty::Any)),
        "Map" => Ty::Map(Box::new(Ty::Any)),
        "Object" => Ty::Object,
        name => Ty::Named(name.into()),
    }
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
fn root_span(table: &SpanTable, value: &V, range: Option<Span>) -> Option<Span> {
    let candidates = table.entries.iter().filter(|entry| {
        evaluate::equal(&entry.value, value)
            && range.is_none_or(|span| entry.from >= span.from && entry.to <= span.to)
    });
    if let Some(range) = range {
        if let Some(exact) = table.entries.iter().find(|entry| {
            entry.from == range.from && entry.to == range.to && evaluate::equal(&entry.value, value)
        }) {
            return Some(Span {
                from: exact.from,
                to: exact.to,
            });
        }
    }
    let best = candidates.max_by_key(|entry| entry.to - entry.from)?;
    Some(Span {
        from: best.from,
        to: best.to,
    })
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
    table
        .entries
        .iter()
        .filter(|entry| {
            evaluate::equal(&entry.value, value)
                && entry.from >= parent.from
                && entry.to <= parent.to
                && after.is_none_or(|span| entry.from > span.from)
        })
        .min_by_key(|entry| (entry.to - entry.from, entry.from))
        .map(|entry| Span {
            from: entry.from,
            to: entry.to,
        })
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
    let_cache: HashMap<String, Ty>,
    dictionary_cache: HashMap<String, Ty>,
    visiting: HashSet<String>,
    expected: Vec<Ty>,
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
            let_cache: HashMap::new(),
            dictionary_cache: HashMap::new(),
            visiting: HashSet::new(),
            expected: Vec::new(),
        }
    }
    fn emit(
        &mut self,
        code: &str,
        expected: String,
        actual: String,
        span: Option<Span>,
        context: &str,
    ) {
        let Some(span) = span else { return };
        self.diagnostics.push(json!({
            "code": code,
            "severity": "error",
            "message": format!("{context}: ожидался {expected}, получен {actual}"),
            "from": span.from,
            "to": span.to,
            "expected": expected,
            "actual": actual,
            "context": context,
        }));
    }
    fn check(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        code: &str,
        span: Option<Span>,
        context: &str,
    ) -> bool {
        if compatible(actual, expected) == Compatible::No {
            self.emit(code, expected.display(), actual.display(), span, context);
            false
        } else {
            true
        }
    }
    fn child(&self, value: &V, parent: Option<Span>) -> Option<Span> {
        child_span(self.spans, value, parent)
    }
    fn infer_root(&mut self, value: &V, range: Option<Span>) -> Ty {
        let span = root_span(self.spans, value, range).or(range);
        self.infer(value, span)
    }
    fn infer_expected(&mut self, value: &V, expected: Ty, range: Option<Span>) -> Ty {
        self.expected.push(expected);
        let ty = self.infer_root(value, range);
        self.expected.pop();
        ty
    }
    fn infer(&mut self, value: &V, span: Option<Span>) -> Ty {
        self.expressions_total += 1;
        let ty = self.infer_inner(value, span);
        if ty.known() {
            self.expressions_typed += 1;
        }
        ty
    }
    fn infer_inner(&mut self, value: &V, span: Option<Span>) -> Ty {
        if value.is_null() {
            return Ty::Null;
        }
        if value.is_boolean() {
            return Ty::Bool;
        }
        if let Some(number) = value.as_f64() {
            return if number.fract() == 0.0 {
                Ty::Int
            } else {
                Ty::Number
            };
        }
        if value.is_string() {
            return Ty::String;
        }
        if let Some(items) = value.as_array() {
            let types = items
                .iter()
                .map(|item| {
                    let child = self.child(item, span);
                    self.infer(item, child)
                })
                .collect::<Vec<_>>();
            return Ty::List(Box::new(union(types)));
        }
        if let Some(raw) = value["expr"].as_str() {
            return self.infer_expr_reference(raw, span);
        }
        if let Some(parts) = value["interpolation"].as_array() {
            for part in parts {
                if part.is_string() {
                    continue;
                }
                let child = self.child(part, span);
                let ty = self.infer(part, child);
                if !self.is_scalar(&ty) {
                    self.emit(
                        "incompatible-operand",
                        "String | Number | Bool | Null".into(),
                        ty.display(),
                        child,
                        "Строковая интерполяция",
                    );
                }
            }
            return Ty::String;
        }
        if value.get("match").is_some() {
            return self.infer_match_value(value, span);
        }
        if let Some(expression) = value.get("expression") {
            return self.infer_expression(expression, value, span);
        }
        if let Some(object) = value.as_object() {
            let types = object
                .values()
                .map(|item| {
                    let child = self.child(item, span);
                    self.infer(item, child)
                })
                .collect::<Vec<_>>();
            return Ty::Record(Box::new(union(types)));
        }
        Ty::Unknown
    }
    fn is_scalar(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Unknown | Ty::Any => true,
            Ty::Union(types) => types.iter().all(|t| self.is_scalar(t)),
            Ty::List(_) | Ty::Map(_) | Ty::Record(_) | Ty::Object => false,
            Ty::Named(name) => self.doc["enums"].get(name).is_some(),
            _ => true,
        }
    }
    fn infer_expr_reference(&mut self, raw: &str, span: Option<Span>) -> Ty {
        if let Some(path) = raw.strip_prefix('!') {
            let actual = self.infer_reference(path);
            self.check(
                &actual,
                &Ty::Bool,
                "incompatible-operand",
                span,
                "Оператор !",
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
        self.infer_reference(raw)
    }
    fn infer_reference(&mut self, raw: &str) -> Ty {
        let mut parts = raw.split('.');
        let head = parts.next().unwrap_or("");
        let tail = parts.collect::<Vec<_>>();
        if tail.is_empty() {
            if let Some(ty) = self.locals.get(head) {
                return ty.clone();
            }
        }
        match head {
            "props" => {
                if tail.is_empty() {
                    return Ty::Unknown;
                }
                let name = if tail[0] == "font" && tail.get(1) == Some(&"size") {
                    "fontSize"
                } else {
                    tail[0]
                };
                let skip = if name == "fontSize" && tail[0] == "font" {
                    2
                } else {
                    1
                };
                let base = self.doc["propDefinitions"]
                    .get(name)
                    .and_then(|d| d["type"].as_str())
                    .map(parse_type)
                    .or_else(|| {
                        if self.doc["lets"].get(name).is_some() {
                            Some(self.infer_let(name))
                        } else {
                            None
                        }
                    });
                base.map_or(Ty::Unknown, |ty| self.member_path(ty, &tail[skip..]))
            }
            "state" => {
                if tail.is_empty() {
                    return Ty::Unknown;
                }
                self.doc["stateDefinitions"]
                    .get(tail[0])
                    .and_then(|d| d["type"].as_str())
                    .map(parse_type)
                    .map_or(Ty::Unknown, |ty| self.member_path(ty, &tail[1..]))
            }
            "tokens" | "i18n" => {
                let key = tail.join(".");
                if key.is_empty() {
                    return Ty::Unknown;
                }
                let cache_key = format!("{head}.{key}");
                self.infer_dictionary(head, &key, &cache_key)
            }
            "viewport" if tail.len() == 1 => match tail[0] {
                "width" | "height" => Ty::Number,
                "compact" => Ty::Bool,
                _ => Ty::Unknown,
            },
            _ => {
                if tail.len() == 1 && self.doc["enums"].get(head).is_some() {
                    if self.doc["enums"][head].as_array().is_some_and(|variants| {
                        variants.iter().any(|v| v.as_str() == Some(tail[0]))
                    }) {
                        return Ty::Named(head.into());
                    }
                }
                if let Some(ty) = self.locals.get(head) {
                    return self.member_path(ty.clone(), &tail);
                }
                if tail.is_empty() {
                    if self.doc["stateDefinitions"].get(head).is_some() {
                        return self.infer_reference(&format!("state.{head}"));
                    }
                    if self.doc["propDefinitions"].get(head).is_some()
                        || self.doc["lets"].get(head).is_some()
                    {
                        return self.infer_reference(&format!("props.{head}"));
                    }
                }
                Ty::Unknown
            }
        }
    }
    fn member_path(&self, base: Ty, path: &[&str]) -> Ty {
        path.iter()
            .fold(base, |ty, part| self.member_type(&ty, part))
    }
    fn member_type(&self, base: &Ty, property: &str) -> Ty {
        match base {
            Ty::Named(name) => self.doc["structs"]
                .get(name)
                .and_then(|fields| fields.get(property))
                .and_then(V::as_str)
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
        let value = self.doc["lets"][name].clone();
        let ty = if value.is_null() {
            Ty::Null
        } else {
            self.infer_root(&value, ast_range(&self.doc["propertyRanges"][name]))
        };
        self.visiting.remove(&key);
        self.let_cache.insert(name.into(), ty.clone());
        ty
    }
    fn infer_dictionary(&mut self, section: &str, key: &str, cache_key: &str) -> Ty {
        if let Some(ty) = self.dictionary_cache.get(cache_key) {
            return ty.clone();
        }
        if !self.visiting.insert(cache_key.into()) {
            return Ty::Unknown;
        }
        let value = self.doc[section][key].clone();
        let ty = if value.is_null() {
            Ty::Unknown
        } else {
            self.infer_root(&value, None)
        };
        self.visiting.remove(cache_key);
        self.dictionary_cache.insert(cache_key.into(), ty.clone());
        ty
    }
    fn infer_expression(&mut self, expression: &V, whole: &V, span: Option<Span>) -> Ty {
        match expression["kind"].as_str().unwrap_or("") {
            "conditional" => {
                let condition = &expression["condition"];
                let condition_span = self.child(condition, span);
                let condition_ty = self.infer(condition, condition_span);
                self.check(
                    &condition_ty,
                    &Ty::Bool,
                    "incompatible-condition",
                    condition_span,
                    "Условный оператор",
                );
                let expected = self.expected.last().cloned();
                let mut valid = true;
                let types = ["then", "else"].map(|key| {
                    let value = &expression[key];
                    let child = self.child(value, span);
                    let ty = self.infer(value, child);
                    if let Some(expected) = &expected {
                        valid &= self.check(
                            &ty,
                            expected,
                            "incompatible-binding",
                            child,
                            "Ветвь условного оператора",
                        );
                    }
                    ty
                });
                if valid { union(types) } else { Ty::Unknown }
            }
            "unary" => {
                let argument = &expression["argument"];
                let child = self.child(argument, span);
                let actual = self.infer(argument, child);
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
                    &format!("Оператор {operator}"),
                );
                if actual == Ty::Unknown {
                    Ty::Unknown
                } else if operator == "!" {
                    Ty::Bool
                } else {
                    Ty::Number
                }
            }
            "member" => {
                let object = &expression["object"];
                let object_span = self.child(object, span);
                let base = self.infer(object, object_span);
                let property = expression["property"].as_str().map(str::to_owned);
                let property = if let Some(property) = property {
                    property
                } else {
                    let dynamic = &expression["property"];
                    let dynamic_span = self.child(dynamic, span);
                    let key = self.infer(dynamic, dynamic_span);
                    self.check(
                        &key,
                        &union([Ty::String, Ty::Number]),
                        "incompatible-operand",
                        dynamic_span,
                        "Вычисляемое свойство",
                    );
                    return Ty::Unknown;
                };
                let mut result = if property == "length" && matches!(base, Ty::String | Ty::List(_))
                {
                    Ty::Int
                } else if let Some(object) = object.as_object()
                    && !object.contains_key("expr")
                    && !object.contains_key("expression")
                {
                    object
                        .get(&property)
                        .map(|v| self.infer(v, self.child(v, object_span)))
                        .unwrap_or(Ty::Unknown)
                } else {
                    self.member_type(&base, &property)
                };
                if expression["optional"] == true {
                    result = union([result, Ty::Null]);
                }
                result
            }
            "call" => {
                let name = expression["name"].as_str().unwrap_or("");
                let args = expression["args"].as_array().cloned().unwrap_or_default();
                let mut types = Vec::with_capacity(args.len());
                let mut spans = Vec::with_capacity(args.len());
                for arg in &args {
                    let child = self.child(arg, span);
                    spans.push(child);
                    types.push(self.infer(arg, child));
                }
                self.infer_call(name, &types, &spans)
            }
            "binary" => self.infer_binary(expression, span),
            _ => {
                let _ = whole;
                Ty::Unknown
            }
        }
    }
    fn infer_binary(&mut self, expression: &V, span: Option<Span>) -> Ty {
        let operator = expression["operator"].as_str().unwrap_or("");
        let left = &expression["left"];
        let right = &expression["right"];
        let left_span = self.child(left, span);
        let right_span = self.child(right, span);
        let a = self.infer(left, left_span);
        let b = self.infer(right, right_span);
        match operator {
            "??" => match a {
                Ty::Unknown | Ty::Any => Ty::Unknown,
                Ty::Null => b,
                Ty::Union(_) => union([non_null(a), b]),
                _ => a,
            },
            "&&" | "||" => {
                self.check(
                    &a,
                    &Ty::Bool,
                    "incompatible-operand",
                    left_span,
                    &format!("Оператор {operator}"),
                );
                self.check(
                    &b,
                    &Ty::Bool,
                    "incompatible-operand",
                    right_span,
                    &format!("Оператор {operator}"),
                );
                if a == Ty::Unknown || b == Ty::Unknown {
                    Ty::Unknown
                } else {
                    Ty::Bool
                }
            }
            "==" | "!=" | "===" | "!==" => Ty::Bool,
            "<" | "<=" | ">" | ">=" => {
                let numeric = |t: &Ty| matches!(t, Ty::Number | Ty::Int | Ty::Unknown | Ty::Any);
                let string = |t: &Ty| matches!(t, Ty::String | Ty::Unknown | Ty::Any);
                if !((numeric(&a) && numeric(&b)) || (string(&a) && string(&b))) {
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
                }
            }
            ".." | "..=" => {
                self.check(
                    &a,
                    &Ty::Number,
                    "incompatible-operand",
                    left_span,
                    "Диапазон",
                );
                self.check(
                    &b,
                    &Ty::Number,
                    "incompatible-operand",
                    right_span,
                    "Диапазон",
                );
                Ty::List(Box::new(Ty::Number))
            }
            "+" => {
                if matches!(a, Ty::Unknown | Ty::Any) || matches!(b, Ty::Unknown | Ty::Any) {
                    Ty::Unknown
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
                }
            }
            "-" | "*" | "/" | "%" => {
                self.check(
                    &a,
                    &Ty::Number,
                    "incompatible-operand",
                    left_span,
                    &format!("Оператор {operator}"),
                );
                self.check(
                    &b,
                    &Ty::Number,
                    "incompatible-operand",
                    right_span,
                    &format!("Оператор {operator}"),
                );
                if a == Ty::Unknown || b == Ty::Unknown {
                    Ty::Unknown
                } else {
                    Ty::Number
                }
            }
            _ => Ty::Unknown,
        }
    }
    fn infer_match_value(&mut self, value: &V, span: Option<Span>) -> Ty {
        let subject = &value["match"];
        let subject_span = self.child(subject, span);
        let subject_ty = self.infer(subject, subject_span);
        let expected = self.expected.last().cloned();
        let mut branch_types = Vec::new();
        let mut valid = true;
        for branch in value["branches"].as_array().into_iter().flatten() {
            self.check_pattern(&branch["pattern"], &subject_ty, span);
            if let Some(guard) = branch.get("guard") {
                let guard_span = child_span_after(self.spans, guard, span, subject_span);
                let guard_ty = self.infer(guard, guard_span);
                self.check(
                    &guard_ty,
                    &Ty::Bool,
                    "incompatible-condition",
                    guard_span,
                    "Guard match",
                );
            }
            let result = &branch["value"];
            let result_span = self.child(result, span);
            let ty = self.infer(result, result_span);
            if let Some(expected) = &expected {
                valid &= self.check(
                    &ty,
                    expected,
                    "incompatible-match-branch",
                    result_span,
                    "Ветвь match",
                );
            }
            branch_types.push(ty);
        }
        if valid {
            union(branch_types)
        } else {
            Ty::Unknown
        }
    }
    fn check_pattern(&mut self, pattern: &V, subject: &Ty, parent: Option<Span>) {
        if pattern["expr"] == "_" {
            return;
        }
        let span = self.child(pattern, parent);
        if let Some(alternatives) = pattern["or"].as_array() {
            for alternative in alternatives {
                self.check_pattern(alternative, subject, parent);
            }
            return;
        }
        if pattern.get("comparison").is_some() {
            let value = &pattern["value"];
            let value_span = self.child(value, span);
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
            return;
        }
        if pattern["expression"]["kind"] == "binary"
            && ["..", "..="].contains(&pattern["expression"]["operator"].as_str().unwrap_or(""))
        {
            let range_span = self.child(pattern, span);
            self.infer(pattern, range_span);
            self.check(
                subject,
                &Ty::Number,
                "incompatible-match-pattern",
                range_span,
                "Диапазон в match",
            );
            return;
        }
        if let Some(items) = pattern.as_array() {
            for item in items {
                self.check_pattern(item, subject, span);
            }
            return;
        }
        let actual = self.infer(pattern, span);
        if matches!(subject, Ty::Unknown | Ty::Any) || matches!(actual, Ty::Unknown | Ty::Any) {
            return;
        }
        let alternatives: Vec<&Ty> = match subject {
            Ty::Union(types) => types.iter().collect(),
            other => vec![other],
        };
        let possible = alternatives.into_iter().any(|expected| {
            compatible(&actual, expected) == Compatible::Yes
                || compatible(expected, &actual) == Compatible::Yes
        });
        if !possible {
            self.emit(
                "incompatible-match-pattern",
                subject.display(),
                actual.display(),
                span,
                "Паттерн match",
            );
        }
    }
    fn infer_call(&mut self, name: &str, args: &[Ty], spans: &[Option<Span>]) -> Ty {
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
        let result = (|| match name {
            "String" => {
                if let Some(actual) = args.first()
                    && !self.is_scalar(actual)
                {
                    requirements.push(expected(0, scalar()));
                }
                return Ty::String;
            }
            "Number" => {
                requirements.push(expected(0, union([Ty::Number, Ty::String])));
                return Ty::Number;
            }
            "Bool" => {
                requirements.push(expected(0, Ty::Bool));
                return Ty::Bool;
            }
            "len" => {
                requirements.push(expected(0, union([Ty::String, list_any(), object()])));
                return Ty::Int;
            }
            "isEmpty" => return Ty::Bool,
            "count" => {
                requirements.push(expected(0, list_any()));
                return Ty::Int;
            }
            "any" | "all" => {
                requirements.push(expected(0, Ty::List(Box::new(Ty::Bool))));
                return Ty::Bool;
            }
            "pluck" | "groupBy" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::String)]);
                return list_any();
            }
            "sortBy" | "unique" => {
                requirements.push(expected(0, list_any()));
                if args.len() > 1 {
                    requirements.push(expected(1, Ty::String));
                }
                return args.first().cloned().unwrap_or(Ty::Unknown);
            }
            "alpha" | "lighten" | "darken" | "saturate" | "desaturate" => {
                requirements.extend([expected(0, Ty::Color), expected(1, Ty::Number)]);
                return Ty::Color;
            }
            "invert" => {
                requirements.push(expected(0, Ty::Color));
                return Ty::Color;
            }
            "contrast" => {
                requirements.extend([expected(0, Ty::Color), expected(1, Ty::Color)]);
                return Ty::Number;
            }
            "mix" => {
                requirements.extend([expected(0, Ty::Color), expected(1, Ty::Color)]);
                if args.len() > 2 {
                    requirements.push(expected(2, Ty::Number));
                }
                return Ty::Color;
            }
            "min" | "max" => {
                for index in 0..args.len() {
                    requirements.push(expected(index, Ty::Number));
                }
                return Ty::Number;
            }
            "clamp" | "lerp" => {
                for index in 0..3 {
                    requirements.push(expected(index, Ty::Number));
                }
                return Ty::Number;
            }
            "remap" => {
                for index in 0..5 {
                    requirements.push(expected(index, Ty::Number));
                }
                return Ty::Number;
            }
            "pow" => {
                requirements.extend([expected(0, Ty::Number), expected(1, Ty::Number)]);
                return Ty::Number;
            }
            "sqrt" | "sign" | "abs" | "round" | "floor" | "ceil" => {
                requirements.push(expected(0, Ty::Number));
                return Ty::Number;
            }
            "avg" | "sum" => {
                requirements.push(expected(0, Ty::List(Box::new(Ty::Number))));
                return Ty::Number;
            }
            "trim" | "upper" | "lower" | "capitalize" => {
                requirements.push(expected(0, Ty::String));
                return Ty::String;
            }
            "padStart" | "padEnd" | "truncate" => {
                requirements.extend([expected(0, Ty::String), expected(1, Ty::Number)]);
                if args.len() > 2 {
                    requirements.push(expected(2, Ty::String));
                }
                return Ty::String;
            }
            "repeat" => {
                requirements.extend([expected(0, Ty::String), expected(1, Ty::Number)]);
                return Ty::String;
            }
            "contains" | "includes" | "indexOf" => {
                if let Some(actual) = args.first() {
                    if actual == &Ty::String {
                        requirements.push(expected(1, Ty::String));
                    } else if !matches!(actual, Ty::List(_) | Ty::Unknown | Ty::Any) {
                        requirements.push(expected(0, union([Ty::String, list_any()])));
                    }
                }
                return if name == "indexOf" { Ty::Int } else { Ty::Bool };
            }
            "startsWith" | "endsWith" | "split" => {
                requirements.extend([expected(0, Ty::String), expected(1, Ty::String)]);
                return if name == "split" {
                    Ty::List(Box::new(Ty::String))
                } else {
                    Ty::Bool
                };
            }
            "replace" => {
                for index in 0..3 {
                    requirements.push(expected(index, Ty::String));
                }
                return Ty::String;
            }
            "slice" => {
                requirements.push(expected(0, union([Ty::String, list_any()])));
                requirements.push(expected(1, Ty::Number));
                if args.len() > 2 {
                    requirements.push(expected(2, Ty::Number));
                }
                return args.first().cloned().unwrap_or(Ty::Unknown);
            }
            "join" => {
                requirements.push(expected(0, list_any()));
                if args.len() > 1 {
                    requirements.push(expected(1, Ty::String));
                }
                return Ty::String;
            }
            "reverse" => {
                requirements.push(expected(0, union([Ty::String, list_any()])));
                return args.first().cloned().unwrap_or(Ty::Unknown);
            }
            "take" | "skip" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::Number)]);
                return args.first().cloned().unwrap_or(Ty::Unknown);
            }
            "range" => {
                requirements.extend([expected(0, Ty::Number), expected(1, Ty::Number)]);
                return Ty::List(Box::new(Ty::Number));
            }
            "formatNumber" => {
                requirements.push(expected(0, Ty::Number));
                if args.len() > 1 {
                    requirements.push(expected(1, Ty::Number));
                }
                return Ty::String;
            }
            "find" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::String)]);
                return match args.first() {
                    Some(Ty::List(item)) => union([(**item).clone(), Ty::Null]),
                    _ => Ty::Unknown,
                };
            }
            "compact" | "flatten" => {
                requirements.push(expected(0, list_any()));
                return list_any();
            }
            "chunk" => {
                requirements.extend([expected(0, list_any()), expected(1, Ty::Number)]);
                return match args.first() {
                    Some(Ty::List(item)) => Ty::List(Box::new(Ty::List(item.clone()))),
                    _ => Ty::Unknown,
                };
            }
            "zip" => {
                requirements.extend([expected(0, list_any()), expected(1, list_any())]);
                return list_any();
            }
            "first" | "last" => {
                requirements.push(expected(0, list_any()));
                return match args.first() {
                    Some(Ty::List(item)) => union([(**item).clone(), Ty::Null]),
                    _ => Ty::Unknown,
                };
            }
            "keys" => {
                requirements.push(expected(0, object()));
                return Ty::List(Box::new(Ty::String));
            }
            "values" => {
                requirements.push(expected(0, object()));
                return match args.first() {
                    Some(Ty::Map(item) | Ty::Record(item)) => Ty::List(item.clone()),
                    _ => list_any(),
                };
            }
            "entries" => {
                requirements.push(expected(0, object()));
                return list_any();
            }
            "fromEntries" => {
                requirements.push(expected(0, list_any()));
                return Ty::Record(Box::new(Ty::Unknown));
            }
            "merge" => {
                for index in 0..args.len() {
                    requirements.push(expected(index, object()));
                }
                return Ty::Record(Box::new(Ty::Unknown));
            }
            "pick" | "omit" => {
                requirements.extend([
                    expected(0, object()),
                    expected(1, Ty::List(Box::new(Ty::String))),
                ]);
                return Ty::Record(Box::new(Ty::Unknown));
            }
            "coalesce" => return union(args.iter().cloned().map(non_null)),
            "plural" => {
                requirements.push(expected(0, Ty::Number));
                for index in 1..args.len() {
                    if !self.is_scalar(&args[index]) {
                        requirements.push(expected(index, scalar()));
                    }
                }
                return Ty::String;
            }
            "isValid" => {
                if args.len() == 2 {
                    requirements.push(expected(1, Ty::String));
                } else {
                    requirements.extend([expected(1, Ty::Number), expected(2, Ty::Number)]);
                }
                return Ty::Bool;
            }
            _ => return Ty::Unknown,
        })();
        for (index, expected) in requirements.into_iter().flatten() {
            if let Some(actual) = args.get(index) {
                self.check(
                    actual,
                    &expected,
                    "incompatible-argument",
                    spans.get(index).copied().flatten(),
                    &format!("{name}: аргумент {}", index + 1),
                );
            }
        }
        result
    }
    fn property_type(&self, node: &str, name: &str) -> Option<Ty> {
        if node == self.doc["name"].as_str().unwrap_or("") {
            if let Some(ty) = self.doc["propDefinitions"]
                .get(name)
                .and_then(|d| d["type"].as_str())
            {
                return Some(parse_type(ty));
            }
        }
        self.context.property_type(node, name).map(parse_type)
    }
    fn walk_props(&mut self, node: &V, node_type: &str) {
        if let Some(props) = node["props"].as_object() {
            for (name, value) in props {
                let range = ast_range(&node["propertyRanges"][name])
                    .or_else(|| ast_range(&node["statementRanges"][name]));
                if let Some(expected) = self.property_type(node_type, name) {
                    let actual = self.infer_expected(value, expected.clone(), range);
                    self.check(
                        &actual,
                        &expected,
                        "incompatible-binding",
                        root_span(self.spans, value, range).or(range),
                        &format!("Свойство {node_type}.{name}"),
                    );
                } else {
                    self.infer_root(value, range);
                }
            }
        }
        if let Some(bindings) = node["bindings"].as_object() {
            for (name, path) in bindings {
                let Some(path) = path.as_str() else { continue };
                let actual = self.infer_reference(path);
                if let Some(expected) = self.property_type(node_type, name) {
                    let range = ast_range(&node["statementRanges"][name]);
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
                    .cloned()
                    .unwrap_or_default();
                self.walk_action(action, &args, ast_range(&node["statementRanges"][event]));
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
                self.walk_nodes(&json!([value["patch"].clone()]));
            }
        } else if value.get("match").is_some() {
            self.infer_root(&value["match"], range);
            for branch in value["branches"].as_array().into_iter().flatten() {
                self.walk_slot_value(&branch["value"], range);
            }
        } else if value["type"].is_string() {
            self.walk_nodes(&json!([value.clone()]));
        } else {
            self.infer_root(value, range);
        }
    }
    fn walk_action(&mut self, action: &str, args: &[V], range: Option<Span>) {
        let signature = action
            .strip_prefix("events.")
            .and_then(|name| self.doc["eventDefinitions"].get(name))
            .and_then(V::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|arg| arg["type"].as_str())
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                action.strip_prefix("actions.").and_then(|name| {
                    self.context
                        .actions
                        .get(name)
                        .map(|types| types.iter().map(String::as_str).collect::<Vec<_>>())
                })
            });
        for (index, arg) in args.iter().enumerate() {
            let span = root_span(self.spans, arg, range);
            let actual = self.infer_root(arg, range);
            if let Some(expected) = signature
                .as_deref()
                .and_then(|types| types.get(index))
                .map(|ty| parse_type(ty))
            {
                self.check(
                    &actual,
                    &expected,
                    "incompatible-argument",
                    span,
                    &format!("{action}: аргумент {}", index + 1),
                );
            }
        }
    }
    fn walk_groups(&mut self, groups: &V, node_type: &str) {
        for group in groups.as_array().into_iter().flatten() {
            let subject = &group["match"];
            let subject_ty = self.infer_root(subject, None);
            for branch in group["branches"].as_array().into_iter().flatten() {
                self.check_pattern(
                    &branch["pattern"],
                    &subject_ty,
                    root_span(self.spans, subject, None),
                );
                if let Some(guard) = branch.get("guard") {
                    let ty = self.infer_root(guard, None);
                    self.check(
                        &ty,
                        &Ty::Bool,
                        "incompatible-condition",
                        root_span(self.spans, guard, None),
                        "Guard match",
                    );
                }
                let patch = json!({
                    "props": branch["props"],
                    "propertyRanges": branch["propertyRanges"],
                    "statementRanges": branch["statementRanges"],
                });
                self.walk_props_only(&patch, node_type);
            }
        }
    }
    fn walk_props_only(&mut self, node: &V, node_type: &str) {
        if let Some(props) = node["props"].as_object() {
            for (name, value) in props {
                let range = ast_range(&node["propertyRanges"][name])
                    .or_else(|| ast_range(&node["statementRanges"][name]));
                if let Some(expected) = self.property_type(node_type, name) {
                    let actual = self.infer_expected(value, expected.clone(), range);
                    self.check(
                        &actual,
                        &expected,
                        "incompatible-binding",
                        root_span(self.spans, value, range).or(range),
                        &format!("Свойство {node_type}.{name}"),
                    );
                } else {
                    self.infer_root(value, range);
                }
            }
        }
    }
    fn walk_nodes(&mut self, nodes: &V) {
        for node in nodes.as_array().into_iter().flatten() {
            match node["type"].as_str().unwrap_or("") {
                "Let" => {
                    let ty = self.infer_root(&node["value"], node_range(node));
                    if let Some(name) = node["name"].as_str() {
                        self.locals.insert(name.into(), ty);
                    }
                }
                "If" => {
                    let condition = &node["condition"];
                    let span = root_span(self.spans, condition, node_range(node));
                    let ty = self.infer(condition, span);
                    self.check(&ty, &Ty::Bool, "incompatible-condition", span, "Условие if");
                    self.walk_nodes(&node["children"]);
                    self.walk_nodes(&node["elseChildren"]);
                }
                "For" => {
                    let collection = &node["collection"];
                    let span = root_span(self.spans, collection, node_range(node));
                    let ty = self.infer(collection, span);
                    self.check(
                        &ty,
                        &Ty::List(Box::new(Ty::Any)),
                        "incompatible-binding",
                        span,
                        "Коллекция for",
                    );
                    let saved = self.locals.clone();
                    if let Some(name) = node["item"].as_str() {
                        let item = match ty {
                            Ty::List(item) => *item,
                            _ => Ty::Unknown,
                        };
                        self.locals.insert(name.into(), item);
                    }
                    if let Some(index) = node["index"].as_str() {
                        self.locals.insert(index.into(), Ty::Int);
                    }
                    if let Some(condition) = node.get("where") {
                        let span = root_span(self.spans, condition, node_range(node));
                        let actual = self.infer(condition, span);
                        self.check(
                            &actual,
                            &Ty::Bool,
                            "incompatible-condition",
                            span,
                            "Условие where",
                        );
                    }
                    let key = &node["key"];
                    let key_span = root_span(self.spans, key, node_range(node));
                    let key_ty = self.infer(key, key_span);
                    self.check(
                        &key_ty,
                        &union([Ty::String, Ty::Number]),
                        "incompatible-binding",
                        key_span,
                        "Ключ for",
                    );
                    self.walk_nodes(&node["children"]);
                    self.locals = saved;
                    self.walk_nodes(&node["emptyChildren"]);
                }
                "Match" => {
                    let subject = &node["match"];
                    let subject_span = root_span(self.spans, subject, node_range(node));
                    let subject_ty = self.infer(subject, subject_span);
                    for branch in node["branches"].as_array().into_iter().flatten() {
                        self.check_pattern(&branch["pattern"], &subject_ty, subject_span);
                        if let Some(guard) = branch.get("guard") {
                            let span = root_span(self.spans, guard, node_range(node));
                            let ty = self.infer(guard, span);
                            self.check(
                                &ty,
                                &Ty::Bool,
                                "incompatible-condition",
                                span,
                                "Guard match",
                            );
                        }
                        self.walk_nodes(&branch["children"]);
                    }
                }
                node_type => {
                    self.walk_props(node, node_type);
                    self.walk_nodes(&node["children"]);
                }
            }
        }
    }
    fn infer_declarations(&mut self) {
        if let Some(defaults) = self.doc["defaults"].as_object() {
            let node_type = self.doc["name"].as_str().unwrap_or("").to_owned();
            for (name, value) in defaults {
                let range = ast_range(&self.doc["defaultRanges"][name]);
                if let Some(expected) = self.property_type(&node_type, name) {
                    let actual = self.infer_expected(value, expected.clone(), range);
                    self.check(
                        &actual,
                        &expected,
                        "incompatible-binding",
                        root_span(self.spans, value, range).or(range),
                        &format!("Default {node_type}.{name}"),
                    );
                } else {
                    self.infer_root(value, range);
                }
            }
        }
        if let Some(definitions) = self.doc["stateDefinitions"].as_object() {
            for (name, definition) in definitions {
                let range = ast_range(&self.doc["defaultRanges"][name]);
                let expected = parse_type(definition["type"].as_str().unwrap_or(""));
                let value = &definition["default"];
                let actual = self.infer_expected(value, expected.clone(), range);
                self.check(
                    &actual,
                    &expected,
                    "incompatible-binding",
                    root_span(self.spans, value, range).or(range),
                    &format!("Default state.{name}"),
                );
            }
        }
        for section in ["lets", "tokens", "i18n"] {
            if let Some(values) = self.doc[section].as_object() {
                for value in values.values() {
                    self.infer_root(value, None);
                }
            }
        }
        if let Some(styles) = self.doc["styles"].as_object() {
            for style in styles.values() {
                self.walk_props_only(style, "*");
            }
        }
    }
    fn run(&mut self) {
        self.infer_declarations();
        self.walk_nodes(&self.doc["nodes"]);
        self.walk_groups(
            &self.doc["matches"],
            self.doc["name"].as_str().unwrap_or(""),
        );
        for section in ["slots", "slotDefinitions"] {
            if let Some(values) = self.doc[section].as_object() {
                for value in values.values() {
                    self.walk_nodes(&json!([value]));
                }
            }
        }
        for shortcut in self.doc["shortcuts"].as_array().into_iter().flatten() {
            let args = shortcut["args"].as_array().cloned().unwrap_or_default();
            self.walk_action(
                shortcut["action"].as_str().unwrap_or(""),
                &args,
                ast_range(shortcut),
            );
        }
        self.walk_nodes(&self.doc["entries"]);
        for state in self.doc["states"].as_array().into_iter().flatten() {
            self.walk_nodes(&state["nodes"]);
        }
    }
}
fn unit_type(raw: &str) -> Option<Ty> {
    for (suffixes, ty) in [
        (
            &["px", "%", "rem", "em", "vw", "vh", "ch", "fr"][..],
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
