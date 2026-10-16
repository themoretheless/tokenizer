use super::{Checker, NameReference};
use crate::{Expr, ExprKind};
use themoretheless_tokenizer_core::Span;

impl<'s> Checker<'s> {
    /// Deepest region level among recorded name references inside `span` that
    /// resolve to bindings declared deeper than `level`; `None` when nothing
    /// escapes.
    pub(super) fn escape_level(&self, span: Span, level: usize) -> Option<usize> {
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
    pub(super) fn is_promote_call(&self, expression: &Expr<'s>) -> bool {
        self.is_unshadowed_call(expression, "promote")
    }

    /// Explicit `move(binding)`: takes the value out of a mutable cell,
    /// leaving it empty. Also an explicit relocation intent for escapes.
    pub(super) fn is_move_call(&self, expression: &Expr<'s>) -> bool {
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
    pub(super) fn move_call(&mut self, span: Span, arguments: &[Expr<'s>]) {
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
    pub(super) fn check_escape(
        &mut self,
        value: &Expr<'s>,
        target_level: usize,
        context: &'static str,
    ) {
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
}
