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
    expected: Option<Ty>,
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
fn contextual_struct_name<'a>(doc: &'a V, expected: Option<&'a Ty>) -> Option<&'a str> {
    match expected? {
        Ty::Named(name) if doc["structs"].get(name.as_str()).is_some() => Some(name.as_str()),
        Ty::Union(ts) => {
            let mut found = None;
            for t in ts {
                if *t == Ty::Null {
                    continue;
                }
                if let Ty::Named(name) = t
                    && doc["structs"].get(name.as_str()).is_some()
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
            let_cache: HashMap::new(),
            dictionary_cache: HashMap::new(),
            visiting: HashSet::new(),
            expected: None,
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
        self.expressions_total += 1;
        let ty = self.infer_inner(value, span);
        if ty.known() {
            self.expressions_typed += 1;
        }
        ty
    }
    fn infer_inner(&mut self, value: &V, span: Option<Span>) -> Ty {
        let contextual = self.expected.take();
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
            return Ty::List(Box::new(union(types)));
        }
        if let Some(raw) = value["expr"].as_str() {
            return self.infer_expr_reference(raw, span);
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
            return Ty::String;
        }
        if value.get("match").is_some() {
            return self.infer_match_value(value, span, contextual);
        }
        if let Some(expression) = value.get("expression") {
            return self.infer_expression(expression, value, span, contextual);
        }
        if let Some(object) = value.as_object() {
            let doc = self.doc;
            if let Some(struct_name) = contextual_struct_name(doc, contextual.as_ref())
                && let Some(fields) = doc["structs"][struct_name].as_object()
            {
                let missing_required = fields.iter().any(|(fname, fty)| {
                    !fty.as_str().unwrap_or("").trim().ends_with('?') && !object.contains_key(fname)
                });
                if !missing_required {
                    let mut valid = true;
                    let expected_keys = {
                        let mut keys = fields.keys().map(String::as_str).collect::<Vec<_>>();
                        keys.sort_unstable();
                        keys.join(" | ")
                    };
                    for (key, item) in object {
                        let child = self.child(item, span).or(span);
                        let field_expected = fields.get(key).and_then(V::as_str).map(parse_type);
                        self.expected = field_expected.clone();
                        let actual = self.infer(item, child);
                        if let Some(expected_ty) = &field_expected {
                            let ok = self.check(
                                &actual,
                                expected_ty,
                                "incompatible-binding",
                                child,
                                format_args!("Поле {struct_name}.{key}"),
                            );
                            valid &= ok;
                            if ok {
                                let constraints = &doc["structConstraints"][struct_name][key];
                                valid &= self.check_static_constraints(
                                    item,
                                    constraints,
                                    child,
                                    &format!("Поле {struct_name}.{key}"),
                                );
                            }
                        } else {
                            self.emit(
                                "unknown-field",
                                expected_keys.clone(),
                                key.clone(),
                                child,
                                &format!("Структура {struct_name}"),
                            );
                            valid = false;
                        }
                    }
                    self.expected = None;
                    return if valid {
                        Ty::Named(struct_name.into())
                    } else {
                        Ty::Unknown
                    };
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
            if let Some(ty) = self.doc["propDefinitions"]
                .get(head)
                .and_then(|d| d["type"].as_str())
            {
                return parse_type(ty);
            }
            if self.doc["lets"].get(head).is_some() {
                return self.infer_let(head);
            }
            if let Some(variants) = self.doc["enums"].get(head).and_then(V::as_array) {
                let expected = variants
                    .iter()
                    .filter_map(V::as_str)
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
                if let Some(variants) = self.doc["enums"].get(head).and_then(V::as_array) {
                    if !rest.contains('.') && variants.iter().any(|v| v.as_str() == Some(rest)) {
                        return Ty::Named(head.into());
                    }
                    let expected = variants
                        .iter()
                        .filter_map(V::as_str)
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
                let doc = self.doc;
                if let Some(fields) = doc["structs"].get(name).and_then(V::as_object)
                    && !fields.contains_key(property)
                {
                    let mut keys = fields.keys().map(String::as_str).collect::<Vec<_>>();
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
                let doc = self.doc;
                for ty in non_null_types {
                    if let Ty::Named(name) = ty
                        && let Some(fields) = doc["structs"].get(name).and_then(V::as_object)
                    {
                        if fields.contains_key(property) {
                            any_has_field = true;
                        }
                        for k in fields.keys() {
                            if !expected_keys.contains(&k.as_str()) {
                                expected_keys.push(k.as_str());
                            }
                        }
                    } else {
                        all_known_structs = false;
                        break;
                    }
                }
                if all_known_structs && !any_has_field {
                    expected_keys.sort_unstable();
                    self.emit(
                        "unknown-field",
                        expected_keys.join(" | "),
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
            Ty::Unknown
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
    ) -> Ty {
        match expression["kind"].as_str().unwrap_or("") {
            "conditional" => {
                let condition = &expression["condition"];
                let condition_span = self.child(condition, span);
                let mut prev_span = condition_span;
                let cond_target = condition_span.or(span);
                let condition_ty = self.infer(condition, cond_target);
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
                    self.expected = expected.clone();
                    let ty = self.infer(value, branch_span);
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
                if valid { union(types) } else { Ty::Unknown }
            }
            "unary" => {
                let argument = &expression["argument"];
                let child = self.child(argument, span).or(span);
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
                    format_args!("Оператор {operator}"),
                );
                if actual == Ty::Unknown {
                    Ty::Unknown
                } else if operator == "!" {
                    Ty::Bool
                } else if actual == Ty::Int {
                    Ty::Int
                } else {
                    Ty::Number
                }
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
                    return result;
                };
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
                result
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
                self.infer_call(name, args, &types, &spans, span)
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
        let left_found = self.child(left, span);
        let right_found = self.child_after(right, span, left_found);
        let left_span = left_found.or(span);
        let right_span = right_found.or(span);
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
                    format_args!("Оператор {operator}"),
                );
                self.check(
                    &b,
                    &Ty::Bool,
                    "incompatible-operand",
                    right_span,
                    format_args!("Оператор {operator}"),
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
                    format_args!("Диапазон"),
                );
                self.check(
                    &b,
                    &Ty::Number,
                    "incompatible-operand",
                    right_span,
                    format_args!("Диапазон"),
                );
                Ty::List(Box::new(Ty::Int))
            }
            "+" => {
                if matches!(a, Ty::Unknown | Ty::Any) || matches!(b, Ty::Unknown | Ty::Any) {
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
                }
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
                if a == Ty::Unknown || b == Ty::Unknown {
                    Ty::Unknown
                } else if operator != "/" && a == Ty::Int && b == Ty::Int {
                    Ty::Int
                } else {
                    Ty::Number
                }
            }
            _ => Ty::Unknown,
        }
    }
    fn infer_match_value(&mut self, value: &V, span: Option<Span>, expected: Option<Ty>) -> Ty {
        let subject = &value["match"];
        let subject_found = self.child(subject, span);
        let subject_span = subject_found.or(span);
        let subject_ty = self.infer(subject, subject_span);
        let branches = value["branches"].as_array().map_or(&[][..], Vec::as_slice);
        let mut branch_types = Vec::new();
        let mut valid = true;
        let mut cursor = subject_found;
        for branch in branches {
            let pat_found = self.check_pattern(&branch["pattern"], &subject_ty, span, cursor);
            if pat_found.is_some() {
                cursor = pat_found;
            }
            if let Some(guard) = branch.get("guard") {
                let guard_found = self.child_after(guard, span, cursor);
                if guard_found.is_some() {
                    cursor = guard_found;
                }
                let guard_span = guard_found.or(span);
                let guard_ty = self.infer(guard, guard_span);
                self.check(
                    &guard_ty,
                    &Ty::Bool,
                    "incompatible-condition",
                    guard_span,
                    format_args!("Guard match"),
                );
            }
            let result = &branch["value"];
            let result_found = self.child_after(result, span, cursor);
            if result_found.is_some() {
                cursor = result_found;
            }
            let result_span = result_found.or(span);
            self.expected = expected.clone();
            let ty = self.infer(result, result_span);
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
        parent: Option<Span>,
        after: Option<Span>,
    ) -> Option<Span> {
        let found = self.child_after(pattern, parent, after);
        if pattern["expr"] == "_" {
            return found;
        }
        let span = found.or(parent);
        if let Some(alternatives) = pattern["or"].as_array() {
            let mut sub_after = None;
            for alternative in alternatives {
                let alt_found = self.check_pattern(alternative, subject, span, sub_after);
                if alt_found.is_some() {
                    sub_after = alt_found;
                }
            }
            return found;
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
            return found;
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
            return found;
        }
        if let Some(items) = pattern.as_array() {
            let item_subject = match subject {
                Ty::List(elem) => elem.as_ref().clone(),
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
                let item_found = self.check_pattern(item, &item_subject, span, sub_after);
                if item_found.is_some() {
                    sub_after = item_found;
                }
            }
            return found;
        }
        let actual = self.infer(pattern, span);
        if matches!(subject, Ty::Unknown | Ty::Any) || matches!(actual, Ty::Unknown | Ty::Any) {
            return found;
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
        found
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
                let Some(variants) = self.doc["enums"].get(name).and_then(V::as_array) else {
                    return false;
                };
                for v in variants.iter().filter_map(V::as_str) {
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
            && let Some(ty) = self.doc["propDefinitions"]
                .get(name)
                .and_then(|d| d["type"].as_str())
        {
            return Some(parse_type(ty));
        }
        self.context.property_type(node, name).map(parse_type)
    }
    fn walk_props(&mut self, node: &V, node_type: &str) {
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
            let branches = value["branches"].as_array().map_or(&[][..], Vec::as_slice);
            let mut cursor = subject_found;
            for branch in branches {
                let pat_found =
                    self.check_pattern(&branch["pattern"], &subject_ty, match_span, cursor);
                if pat_found.is_some() {
                    cursor = pat_found;
                }
                if let Some(guard) = branch.get("guard") {
                    let guard_found = self.child_after(guard, match_span, cursor);
                    if guard_found.is_some() {
                        cursor = guard_found;
                    }
                    let guard_span = guard_found.or(match_span);
                    let guard_ty = self.infer(guard, guard_span);
                    self.check(
                        &guard_ty,
                        &Ty::Bool,
                        "incompatible-condition",
                        guard_span,
                        format_args!("Guard match"),
                    );
                }
                self.walk_slot_value(&branch["value"], match_span);
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
            let branches = group["branches"].as_array().map_or(&[][..], Vec::as_slice);
            let mut cursor = subject_found;
            for branch in branches {
                let pat_found =
                    self.check_pattern(&branch["pattern"], &subject_ty, group_span, cursor);
                if pat_found.is_some() {
                    cursor = pat_found;
                }
                if let Some(guard) = branch.get("guard") {
                    let guard_found = if group_span.is_some() {
                        self.child_after(guard, group_span, cursor)
                    } else {
                        root_span(self.spans, guard, None)
                    };
                    if guard_found.is_some() {
                        cursor = guard_found;
                    }
                    let guard_span = guard_found.or(group_span);
                    let ty = self.infer(guard, guard_span);
                    self.check(
                        &ty,
                        &Ty::Bool,
                        "incompatible-condition",
                        guard_span,
                        format_args!("Guard match"),
                    );
                }
                self.walk_props_fields(
                    &branch["props"],
                    &branch["propertyRanges"],
                    &branch["statementRanges"],
                    node_type,
                );
                if let Some(stmt_ranges) = branch["statementRanges"].as_object() {
                    for r in stmt_ranges.values().filter_map(ast_range) {
                        if cursor.is_none_or(|c| r.to > c.to) {
                            cursor = Some(r);
                        }
                    }
                }
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
                    if ok && node_type == self_name {
                        let constraints = &doc["propDefinitions"][name]["constraints"];
                        self.check_static_constraints(
                            value,
                            constraints,
                            val_span,
                            &format!("Свойство {node_type}.{name}"),
                        );
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
                }
            }
            "If" => {
                let condition = &node["condition"];
                let span =
                    root_span(self.spans, condition, node_range(node)).or_else(|| node_range(node));
                let ty = self.infer(condition, span);
                self.check(
                    &ty,
                    &Ty::Bool,
                    "incompatible-condition",
                    span,
                    format_args!("Условие if"),
                );
                self.walk_nodes(&node["children"]);
                self.walk_nodes(&node["elseChildren"]);
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
                    let span = root_span(self.spans, condition, node_range(node))
                        .or_else(|| node_range(node));
                    let actual = self.infer(condition, span);
                    self.check(
                        &actual,
                        &Ty::Bool,
                        "incompatible-condition",
                        span,
                        format_args!("Условие where"),
                    );
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
                self.locals = saved;
                self.walk_nodes(&node["emptyChildren"]);
            }
            "Match" => {
                let match_span = node_range(node);
                let subject = &node["match"];
                let subject_found = self.child(subject, match_span);
                let subject_span = subject_found.or(match_span);
                let subject_ty = self.infer(subject, subject_span);
                let branches = node["branches"].as_array().map_or(&[][..], Vec::as_slice);
                let mut cursor = subject_found;
                for branch in branches {
                    let pat_found =
                        self.check_pattern(&branch["pattern"], &subject_ty, match_span, cursor);
                    if pat_found.is_some() {
                        cursor = pat_found;
                    }
                    if let Some(guard) = branch.get("guard") {
                        let guard_found = self.child_after(guard, match_span, cursor);
                        if guard_found.is_some() {
                            cursor = guard_found;
                        }
                        let span = guard_found.or(match_span);
                        let ty = self.infer(guard, span);
                        self.check(
                            &ty,
                            &Ty::Bool,
                            "incompatible-condition",
                            span,
                            format_args!("Guard match"),
                        );
                    }
                    self.walk_nodes(&branch["children"]);
                    if let Some(children) = branch["children"].as_array() {
                        for child_node in children.iter().filter_map(node_range) {
                            if cursor.is_none_or(|c| child_node.to > c.to) {
                                cursor = Some(child_node);
                            }
                        }
                    }
                }
                self.check_match_exhaustiveness(&subject_ty, branches, match_span);
            }
            node_type => {
                self.walk_props(node, node_type);
                self.walk_nodes(&node["children"]);
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
                    if ok {
                        let constraints = &doc["propDefinitions"][name]["constraints"];
                        self.check_static_constraints(
                            value,
                            constraints,
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
        self.walk_nodes(&doc["entries"]);
        for state in doc["states"].as_array().into_iter().flatten() {
            self.walk_nodes(&state["nodes"]);
        }
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
