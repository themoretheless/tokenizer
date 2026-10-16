use super::{Checker, Shape};
use crate::{Expr, ExprKind, Stmt, StmtKind, ValueType};
use std::collections::{HashMap, HashSet};
use themoretheless_tokenizer_core::Span;

// Only exits that can reach a following statement matter for missing-return.
// Return and continue stop the current block; loops consume their own breaks.
#[derive(Clone, Copy, Default)]
pub(super) struct BlockExits {
    pub(super) next: bool,
    pub(super) breaks: bool,
}

impl BlockExits {
    pub(super) fn inspect(statements: &[Stmt<'_>]) -> Self {
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
    pub(super) fn constrain(
        &mut self,
        expression: &Expr<'s>,
        expected: Option<ValueType>,
        parameters: &HashSet<usize>,
    ) {
        match &expression.kind {
            ExprKind::Name(name) => {
                if let Some(binding) = self.scopes.iter().rev().find_map(|s| s.get(name.text))
                    && parameters.contains(&binding.1.start)
                    && let Some(expected) = expected
                {
                    let id = binding.1.start;
                    if let Some(actual) = self.binding_types.get(&id) {
                        if super::shape::disjoint(actual, &expected) {
                            self.error(
                                name.span,
                                "parameter-type",
                                "Parameter has incompatible type requirements; add an annotation",
                            );
                        } else if actual == &ValueType::Number && expected == ValueType::Int {
                            self.binding_types.insert(id, ValueType::Int);
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
                let bitwise = matches!(*operator, "&" | "^" | "<<" | ">>");
                let comparable = matches!(*operator, "+" | "-" | "%" | "<" | ">" | "<=" | ">=");
                let context = if boolean {
                    Some(ValueType::Bool)
                } else if bitwise {
                    Some(ValueType::Int)
                } else if matches!(*operator, "+" | "-" | "*" | "/" | "%") {
                    expected
                } else {
                    None
                };
                let inferred = |shape: &Shape| {
                    if let Shape::Typed(
                        ty @ (ValueType::Int | ValueType::Number | ValueType::String),
                    ) = shape
                    {
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
                } else if *operator == "~" {
                    Some(ValueType::Int)
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
            ExprKind::Assign {
                operator,
                target,
                value,
            } => {
                let target_expected = match self.shape(target) {
                    Shape::Typed(ty) => Some(ty),
                    _ => expected,
                };
                if *operator == "=" {
                    self.constrain(target, None, parameters);
                    self.constrain(value, target_expected, parameters);
                } else {
                    let operation = match *operator {
                        "+=" => "+",
                        "-=" => "-",
                        "*=" => "*",
                        "/=" => "/",
                        "%=" => "%",
                        "<<=" => "<<",
                        ">>=" => ">>",
                        "&=" => "&",
                        "^=" => "^",
                        _ => "",
                    };
                    let binary = Expr {
                        span: expression.span,
                        kind: ExprKind::Binary {
                            operator: operation,
                            left: target.clone(),
                            right: value.clone(),
                        },
                    };
                    self.constrain(&binary, target_expected, parameters);
                }
            }
            _ => {}
        }
    }

    pub(super) fn constrain_body(
        &mut self,
        statements: &[Stmt<'s>],
        expected: Option<ValueType>,
        parameters: &HashSet<usize>,
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

    pub(super) fn return_candidates(&self, statements: &[Stmt<'s>], output: &mut Vec<Shape>) {
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

    pub(super) fn require_contract(&mut self, shape: &Shape, span: Span) {
        if let Shape::Function(parameters, result, _) = shape
            && (parameters.iter().any(Option::is_none) || !result.has_contract())
        {
            self.error(
                span,
                "dynamic-contract",
                "Function contract cannot be inferred completely; annotate dynamic parameters and return type",
            );
        }
    }
}
