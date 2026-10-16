use super::{Checker, NameReference, PatternSignature, Shape};
use crate::{Expr, ExprKind, MatchArm, ValueType};
use std::collections::HashSet;

impl<'s> Checker<'s> {
    pub(super) fn irrefutable(pattern: &Expr<'s>) -> bool {
        match &pattern.kind {
            ExprKind::Name(_) => true,
            ExprKind::Tuple(items) => items.iter().all(Self::irrefutable),
            ExprKind::Map(fields) => fields.iter().all(|(_, value)| Self::irrefutable(value)),
            _ => false,
        }
    }

    pub(super) fn check_exhaustiveness(&mut self, value: &Expr<'s>, arms: &[MatchArm<'s>]) {
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
        let mut covered = HashSet::new();
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
            self.error(
                value.span,
                "non-exhaustive-match",
                "Match must cover every variant; guarded or refutable payload patterns require a fallback",
            );
        }
    }

    pub(super) fn match_pattern_types(&mut self, pattern: &Expr<'s>, shape: &Shape) {
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

    pub(super) fn match_bindings(&mut self, pattern: &Expr<'s>, start: usize) {
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

    pub(super) fn pattern(&mut self, pattern: &Expr<'s>, start: usize) {
        match &pattern.kind {
            ExprKind::Name(name) if name.text == "_" => {}
            ExprKind::Name(name) => self.declare(name, true, start),
            ExprKind::Tuple(values) => {
                for value in values {
                    self.pattern(value, start);
                }
            }
            ExprKind::Map(entries) => {
                let mut fields = HashSet::new();
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

    pub(super) fn check_pattern_shape(&mut self, pattern: &Expr<'s>, shape: &Shape) {
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

    pub(super) fn pattern_type(&mut self, pattern: &Expr<'s>, ty: &ValueType) {
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
    pub(super) fn pattern_signatures(
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

    pub(super) fn pattern_shape_signatures(
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
}
