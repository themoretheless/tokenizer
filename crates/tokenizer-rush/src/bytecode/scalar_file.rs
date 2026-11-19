//! Eligibility for automatic bytecode execution of standalone scalar files.
//! The general bytecode compiler also accepts constructs whose interpreter
//! semantics (for example, inferred mutable types) it does not yet preserve.

use super::compiler::try_eval_const;
use crate::analysis::FunctionContract;
use crate::{DeclarationRole, Expr, ExprKind, Stmt, StmtKind, ValueType};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scalar {
    Int,
    Number,
    // A bounded integer range yields exactly representable floating values.
    RangeItem,
    Bool,
    Null,
}

impl Scalar {
    fn numeric(self) -> bool {
        matches!(self, Self::Int | Self::Number | Self::RangeItem)
    }
}

#[derive(Clone, Copy)]
struct Binding {
    kind: Scalar,
    mutable: bool,
}

struct Function {
    parameters: Vec<Scalar>,
    result: Scalar,
}

struct Scope<'s> {
    scopes: Vec<HashMap<&'s str, Binding>>,
    functions: HashMap<&'s str, Function>,
    function_result: Option<Scalar>,
    expression_depth: usize,
}

pub(super) fn supports(items: &[Stmt<'_>], contracts: &HashMap<usize, FunctionContract>) -> bool {
    let function_count = items
        .iter()
        .take_while(|stmt| matches!(stmt.kind, StmtKind::Function { .. }))
        .count();
    let (functions, entry) = items.split_at(function_count);
    if !functions.is_empty() && entry.is_empty() {
        // A closing function declaration evaluates to a callable in the interpreter.
        return false;
    }
    let max_depth = crate::ExecutionLimits::new(0).max_depth;
    let mut scope = Scope {
        scopes: Vec::new(),
        functions: HashMap::new(),
        function_result: None,
        // Calls can only reach earlier functions. Budget each frame for its
        // expression, surrounding assignment, and function-body depth.
        expression_depth: (max_depth / (functions.len() + 1)).saturating_sub(3),
    };
    functions.iter().all(|stmt| scope.function(stmt, contracts)) && scope.block(entry, None)
}

fn scalar_type(ty: &ValueType) -> Option<Scalar> {
    match ty {
        ValueType::Int => Some(Scalar::Int),
        ValueType::Number => Some(Scalar::Number),
        ValueType::Bool => Some(Scalar::Bool),
        ValueType::Null => Some(Scalar::Null),
        _ => None,
    }
}

fn matches_contract(actual: Scalar, expected: Scalar) -> bool {
    actual == expected || (actual == Scalar::RangeItem && expected == Scalar::Number)
}

fn block_returns(items: &[Stmt<'_>]) -> bool {
    match items.last().map(|stmt| &stmt.kind) {
        Some(StmtKind::Return(_)) => true,
        Some(StmtKind::If {
            then_block,
            else_block: Some(else_block),
            ..
        }) => block_returns(&then_block.stmts) && block_returns(&else_block.stmts),
        _ => false,
    }
}

impl<'s> Scope<'s> {
    fn function(&mut self, stmt: &Stmt<'s>, contracts: &HashMap<usize, FunctionContract>) -> bool {
        let StmtKind::Function {
            name,
            parameters,
            result,
            body,
            is_async: false,
        } = &stmt.kind
        else {
            return false;
        };
        if matches!(name.text, "range_iter" | "move") {
            return false;
        }
        let contract = contracts.get(&name.span.start);
        let annotation = |ty: Option<&crate::Type<'_>>, inferred: Option<&ValueType>| match ty {
            Some(ty) => scalar_type(&ValueType::parse_annotation(ty).ok()?),
            None => scalar_type(inferred?),
        };
        let Some(result) = annotation(result.as_ref(), contract.and_then(|c| c.1.as_ref())) else {
            return false;
        };
        if !block_returns(&body.stmts) {
            return false;
        }
        let mut bindings = HashMap::new();
        let mut kinds = Vec::with_capacity(parameters.len());
        for (index, parameter) in parameters.iter().enumerate() {
            let ExprKind::Name(binding) = &parameter.pattern.kind else {
                return false;
            };
            let inferred = contract.and_then(|c| c.0.get(index)?.as_ref());
            let Some(kind) = annotation(parameter.ty.as_ref(), inferred) else {
                return false;
            };
            if matches!(binding.text, "_")
                || binding.text == name.text
                || bindings
                    .insert(
                        binding.text,
                        Binding {
                            kind,
                            mutable: false,
                        },
                    )
                    .is_some()
            {
                return false;
            }
            kinds.push(kind);
        }

        // At this point only earlier functions are visible. Scalar variables
        // in the caller have no bearing on which bindings the body can read.
        self.scopes.push(bindings);
        self.function_result = Some(result);
        let supported = body.stmts.iter().all(|stmt| self.statement(stmt));
        self.function_result = None;
        self.scopes.pop();
        if supported {
            self.functions.insert(
                name.text,
                Function {
                    parameters: kinds,
                    result,
                },
            );
        }
        supported
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .copied()
    }

    fn block(&mut self, items: &[Stmt<'s>], range_binding: Option<&'s str>) -> bool {
        let mut scope = HashMap::new();
        if let Some(name) = range_binding {
            scope.insert(
                name,
                Binding {
                    kind: Scalar::RangeItem,
                    mutable: false,
                },
            );
        }
        self.scopes.push(scope);
        let supported = items.iter().all(|stmt| self.statement(stmt));
        self.scopes.pop();
        supported
    }

    fn statement(&mut self, stmt: &Stmt<'s>) -> bool {
        match &stmt.kind {
            StmtKind::Declaration {
                role: DeclarationRole::Binding,
                name,
                constant,
                ty: None,
                constraint: None,
                value,
            } => {
                let Some(kind) = self.expression(value) else {
                    return false;
                };
                self.scopes.last_mut().unwrap().insert(
                    name.text,
                    Binding {
                        kind,
                        mutable: !constant,
                    },
                );
                true
            }
            StmtKind::Show(expr) | StmtKind::Expr(expr) => {
                if let ExprKind::Assign {
                    operator,
                    target,
                    value,
                } = &expr.kind
                {
                    let ExprKind::Name(name) = &target.kind else {
                        return false;
                    };
                    let Some(Binding {
                        kind,
                        mutable: true,
                    }) = self.lookup(name.text)
                    else {
                        return false;
                    };
                    let Some(rhs) = self.expression(value) else {
                        return false;
                    };
                    // The interpreter coerces every assignment to the type
                    // inferred from the initializer. Only use bytecode when
                    // this assignment already produces that same type.
                    match *operator {
                        "=" => kind == rhs,
                        "+=" | "-=" | "*=" | "/=" | "%=" => arithmetic(kind, rhs) == Some(kind),
                        _ => false,
                    }
                } else {
                    self.expression(expr).is_some()
                }
            }
            StmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.expression(condition) == Some(Scalar::Bool)
                    && self.block(&then_block.stmts, None)
                    && else_block
                        .as_ref()
                        .is_none_or(|block| self.block(&block.stmts, None))
            }
            StmtKind::While { condition, body } => {
                self.expression(condition) == Some(Scalar::Bool) && self.block(&body.stmts, None)
            }
            StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                let ExprKind::Call { callee, arguments } = &iterable.kind else {
                    return false;
                };
                matches!(&callee.kind, ExprKind::Name(name)
                    if name.text == "range_iter" && self.lookup(name.text).is_none()
                        && !self.functions.contains_key(name.text))
                    && (2..=3).contains(&arguments.len())
                    && arguments.iter().all(|arg| {
                        self.expression(arg).is_some_and(Scalar::numeric)
                            && try_eval_const(arg)
                                .and_then(|value| value.as_f64())
                                .is_some_and(|n| n.fract() == 0.0 && n.abs() <= (1u64 << 51) as f64)
                    })
                    && self.block(&body.stmts, Some(binding.text))
            }
            StmtKind::Return(value) => self.function_result.is_some_and(|result| {
                match value {
                    Some(expr) => self.expression(expr),
                    None => Some(Scalar::Null),
                }
                .is_some_and(|actual| matches_contract(actual, result))
            }),
            StmtKind::Break | StmtKind::Continue => true,
            _ => false,
        }
    }

    fn expression(&self, expr: &Expr<'s>) -> Option<Scalar> {
        self.expression_with_depth(expr, self.expression_depth)
    }

    fn expression_with_depth(&self, expr: &Expr<'s>, depth: usize) -> Option<Scalar> {
        let depth = depth.checked_sub(1)?;
        let child = |expr| self.expression_with_depth(expr, depth);
        match &expr.kind {
            ExprKind::Number(text) if crate::runtime::is_int_literal(text) => Some(Scalar::Int),
            ExprKind::Number(_) => Some(Scalar::Number),
            ExprKind::Bool(_) => Some(Scalar::Bool),
            ExprKind::Null => Some(Scalar::Null),
            ExprKind::Name(name) => Some(self.lookup(name.text)?.kind),
            ExprKind::Unary { operator, value } => {
                let kind = child(value.as_ref())?;
                match *operator {
                    "+" | "-" if kind.numeric() => Some(kind),
                    "!" | "not" if kind == Scalar::Bool => Some(kind),
                    _ => None,
                }
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let left = child(left.as_ref())?;
                let right = child(right.as_ref())?;
                match *operator {
                    "+" | "-" | "*" | "/" | "%" | "**" => arithmetic(left, right),
                    "==" | "!=" => Some(Scalar::Bool),
                    "<" | "<=" | ">" | ">=" if left.numeric() && right.numeric() => {
                        Some(Scalar::Bool)
                    }
                    "&&" | "and" | "||" | "or" if left == Scalar::Bool && right == Scalar::Bool => {
                        Some(Scalar::Bool)
                    }
                    _ => None,
                }
            }
            ExprKind::Call { callee, arguments } => {
                let ExprKind::Name(name) = &callee.kind else {
                    return None;
                };
                if self.lookup(name.text).is_some() || name.text == "move" {
                    return None;
                }
                let function = self.functions.get(name.text)?;
                (arguments.len() == function.parameters.len()
                    && arguments
                        .iter()
                        .zip(&function.parameters)
                        .all(|(arg, &expected)| {
                            child(arg).is_some_and(|actual| matches_contract(actual, expected))
                        }))
                .then_some(function.result)
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => {
                let kind = child(then_value.as_ref())?;
                (child(condition.as_ref()) == Some(Scalar::Bool)
                    && child(else_value.as_ref()) == Some(kind))
                .then_some(kind)
            }
            _ => None,
        }
    }
}

fn arithmetic(left: Scalar, right: Scalar) -> Option<Scalar> {
    use Scalar::{Int, Number, RangeItem};
    match (left, right) {
        (Int, Int | RangeItem) | (RangeItem, Int) => Some(Int),
        (Number | RangeItem, Number | RangeItem) => Some(Number),
        _ => None,
    }
}
