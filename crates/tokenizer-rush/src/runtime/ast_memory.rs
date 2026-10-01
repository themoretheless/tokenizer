//! Admission for immutable syntax copies. Reserve before cloning Vec/Box storage.
use super::*;
use std::mem::size_of;
pub(super) fn expressions(items: &[Expr<'_>]) -> usize {
    size_of_val(items) + items.iter().map(expression).sum::<usize>()
}
pub(super) fn expression(e: &Expr<'_>) -> usize {
    let boxed = |e: &Expr<'_>| size_of::<Expr<'_>>() + expression(e);
    match &e.kind {
        ExprKind::If {
            condition,
            then_value,
            else_value,
        } => boxed(condition) + boxed(then_value) + boxed(else_value),
        ExprKind::Lambda { parameters, body } => expressions(parameters) + boxed(body),
        ExprKind::Unary { value, .. } => boxed(value),
        ExprKind::Binary { left, right, .. } => boxed(left) + boxed(right),
        ExprKind::Assign { target, value, .. } => boxed(target) + boxed(value),
        ExprKind::Call { callee, arguments } => boxed(callee) + expressions(arguments),
        ExprKind::Member { object, .. } => boxed(object),
        ExprKind::Index { object, index } => boxed(object) + boxed(index),
        ExprKind::List(v) | ExprKind::Tuple(v) => expressions(v),
        ExprKind::Map(v) => {
            size_of_val(v.as_slice())
                + v.iter()
                    .map(|(a, b)| expression(a) + expression(b))
                    .sum::<usize>()
        }
        ExprKind::Pipeline { input, stages } => boxed(input) + expressions(stages),
        ExprKind::Match { value, arms } => {
            boxed(value)
                + size_of_val(arms.as_slice())
                + arms
                    .iter()
                    .map(|a| {
                        expression(&a.pattern)
                            + a.guard.as_ref().map_or(0, expression)
                            + expression(&a.value)
                    })
                    .sum::<usize>()
        }
        _ => 0,
    }
}
fn syntax_type(ty: &crate::Type<'_>) -> usize {
    size_of_val(ty.arguments.as_slice()) + ty.arguments.iter().map(syntax_type).sum::<usize>()
}
pub(super) fn annotation(ty: &crate::Type<'_>) -> usize {
    let nested = ty.arguments.iter().map(annotation).sum::<usize>();
    nested
        + match ty.name.text {
            "Option" | "list" => size_of::<ValueType>(),
            "Result" => 2 * size_of::<ValueType>(),
            "tuple" => ty.arguments.len() * size_of::<ValueType>(),
            _ => 0,
        }
}
pub(super) fn block(b: &Block<'_>) -> usize {
    statements(&b.stmts)
}
pub(super) fn statements(v: &[Stmt<'_>]) -> usize {
    size_of_val(v)
        + v.iter()
            .map(|s| match &s.kind {
                StmtKind::Destructure { pattern, value } => expression(pattern) + expression(value),
                StmtKind::Function {
                    parameters,
                    result,
                    body,
                    ..
                } => {
                    size_of_val(parameters.as_slice())
                        + parameters
                            .iter()
                            .map(|p| expression(&p.pattern) + p.ty.as_ref().map_or(0, syntax_type))
                            .sum::<usize>()
                        + result.as_ref().map_or(0, syntax_type)
                        + block(body)
                }
                StmtKind::Declaration { ty, value, .. } => {
                    ty.as_ref().map_or(0, syntax_type) + expression(value)
                }
                StmtKind::Return(e) => e.as_ref().map_or(0, expression),
                StmtKind::Expr(e) | StmtKind::Yield(e) => expression(e),
                StmtKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    expression(condition) + block(then_block) + else_block.as_ref().map_or(0, block)
                }
                StmtKind::While { condition, body } => expression(condition) + block(body),
                StmtKind::For { iterable, body, .. } => expression(iterable) + block(body),
                _ => 0,
            })
            .sum::<usize>()
}
#[derive(Debug)]
pub(super) struct RuntimeType {
    value: ValueType,
    _storage: memory::Reservation,
}
impl PartialEq for RuntimeType {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl std::ops::Deref for RuntimeType {
    type Target = ValueType;
    fn deref(&self) -> &ValueType {
        &self.value
    }
}
impl RuntimeType {
    pub(super) fn new(value: ValueType, storage: memory::Reservation) -> Self {
        Self {
            value,
            _storage: storage,
        }
    }
}
#[derive(Debug)]
pub(super) struct StoredModule<'s> {
    pub(super) module: crate::Module<'s>,
    pub(super) _storage: memory::Reservation,
}
impl<'s> std::ops::Deref for StoredModule<'s> {
    type Target = crate::Module<'s>;
    fn deref(&self) -> &Self::Target {
        &self.module
    }
}
