//! Experimental scalar tree/bytecode comparison, not the public Rush runtime.
//! Both engines use predecoded constants and positional parameters.
use std::rc::Rc;
// Each benchmark target exercises a different part of this shared experiment.
#[allow(dead_code)]
#[path = "closures.rs"]
pub mod closures;
use themoretheless_tokenizer_core::Span;
use themoretheless_tokenizer_rush::{
    CancellationToken, Expr, ExprKind, HostFunction, StmtKind, Value, ValueType, parse,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    Number(f64),
    Bool(bool),
}

#[derive(Debug, PartialEq)]
pub struct Error {
    pub span: Span,
    pub message: String,
}
type Result<T> = std::result::Result<T, Error>;

fn error<T>(span: Span, message: &'static str) -> Result<T> {
    Err(Error {
        span,
        message: message.into(),
    })
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Less,
}
impl Op {
    fn apply(self, left: Scalar, right: Scalar, span: Span) -> Result<Scalar> {
        if matches!(self, Self::Eq) {
            return Ok(Scalar::Bool(left == right));
        }
        let (Scalar::Number(a), Scalar::Number(b)) = (left, right) else {
            return error(span, "Numeric operands required");
        };
        let number = match self {
            Self::Add => a + b,
            Self::Sub => a - b,
            Self::Mul => a * b,
            Self::Div => a / b,
            Self::Rem => a % b,
            Self::Less => return Ok(Scalar::Bool(a < b)),
            Self::Eq => unreachable!(),
        };
        if number.is_finite() {
            Ok(Scalar::Number(number))
        } else {
            error(span, "Non-finite arithmetic result")
        }
    }
}

#[derive(Debug)]
enum Kind {
    Constant(Scalar),
    Local(usize),
    Binary(Op, Box<Node>, Box<Node>),
    If(Box<Node>, Box<Node>, Box<Node>),
    Logical(bool, Box<Node>, Box<Node>),
    Host(Rc<HostFunction>, Vec<Node>, Span),
}
#[derive(Debug)]
struct Node {
    span: Span,
    kind: Kind,
}
impl Node {
    fn lower(
        expr: &Expr<'_>,
        names: &[&str],
        hosts: &[Rc<HostFunction>],
        depth: usize,
    ) -> Result<Self> {
        if depth >= 64 {
            return error(expr.span, "Experimental expression depth exceeded");
        }
        let kind = match &expr.kind {
            ExprKind::Call { callee, arguments } => {
                let ExprKind::Name(name) = &callee.kind else {
                    return error(
                        expr.span,
                        "Experimental calls require a named host function",
                    );
                };
                if names.contains(&name.text) {
                    return error(callee.span, "Experimental parameter is not callable");
                }
                let Some(host) = hosts.iter().find(|host| host.name == name.text) else {
                    return error(callee.span, "Unknown experimental host function");
                };
                Kind::Host(
                    host.clone(),
                    arguments
                        .iter()
                        .map(|arg| Self::lower(arg, names, hosts, depth + 1))
                        .collect::<Result<_>>()?,
                    callee.span,
                )
            }
            ExprKind::Number(text) => {
                let Some(value) = text
                    .replace('_', "")
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                else {
                    return error(expr.span, "Unsupported or non-finite number");
                };
                Kind::Constant(Scalar::Number(value))
            }
            ExprKind::Bool(value) => Kind::Constant(Scalar::Bool(*value)),
            ExprKind::Name(name) => {
                Kind::Local(names.iter().position(|n| *n == name.text).ok_or(Error {
                    span: expr.span,
                    message: "Unknown experimental parameter".into(),
                })?)
            }
            ExprKind::If {
                condition,
                then_value,
                else_value,
            } => Kind::If(
                Box::new(Self::lower(condition, names, hosts, depth + 1)?),
                Box::new(Self::lower(then_value, names, hosts, depth + 1)?),
                Box::new(Self::lower(else_value, names, hosts, depth + 1)?),
            ),
            ExprKind::Binary {
                operator,
                left,
                right,
            } if matches!(*operator, "and" | "&&" | "or" | "||") => Kind::Logical(
                matches!(*operator, "or" | "||"),
                Box::new(Self::lower(left, names, hosts, depth + 1)?),
                Box::new(Self::lower(right, names, hosts, depth + 1)?),
            ),
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                let op = match *operator {
                    "+" => Op::Add,
                    "-" => Op::Sub,
                    "*" => Op::Mul,
                    "/" => Op::Div,
                    "%" => Op::Rem,
                    "==" => Op::Eq,
                    "<" => Op::Less,
                    _ => return error(expr.span, "Unsupported experimental operator"),
                };
                Kind::Binary(
                    op,
                    Box::new(Self::lower(left, names, hosts, depth + 1)?),
                    Box::new(Self::lower(right, names, hosts, depth + 1)?),
                )
            }
            _ => return error(expr.span, "Unsupported experimental expression"),
        };
        Ok(Self {
            span: expr.span,
            kind,
        })
    }
    fn eval(&self, args: &[Scalar], fuel: &mut usize, token: &CancellationToken) -> Result<Scalar> {
        charge(fuel, token, self.span)?;
        match &self.kind {
            Kind::Host(host, arguments, callee_span) => {
                charge(fuel, token, *callee_span)?;
                let mut values = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    values.push(to_value(argument.eval(args, fuel, token)?));
                }
                invoke(host, &values, fuel, token, self.span)
            }
            Kind::Constant(value) => Ok(*value),
            Kind::Local(index) => Ok(args[*index]),
            Kind::Binary(op, left, right) => op.apply(
                left.eval(args, fuel, token)?,
                right.eval(args, fuel, token)?,
                self.span,
            ),
            Kind::If(condition, yes, no) => {
                let Scalar::Bool(condition) = condition.eval(args, fuel, token)? else {
                    return error(self.span, "Condition must be boolean");
                };
                if condition { yes } else { no }.eval(args, fuel, token)
            }
            Kind::Logical(short_value, left, right) => {
                let Scalar::Bool(value) = left.eval(args, fuel, token)? else {
                    return error(self.span, "Boolean operand required");
                };
                if value == *short_value {
                    return Ok(Scalar::Bool(value));
                }
                match right.eval(args, fuel, token)? {
                    result @ Scalar::Bool(_) => Ok(result),
                    _ => error(self.span, "Boolean operand required"),
                }
            }
        }
    }
    fn stack_size(&self) -> usize {
        match &self.kind {
            Kind::Host(_, arguments, _) => arguments
                .iter()
                .enumerate()
                .map(|(i, arg)| i + arg.stack_size())
                .max()
                .unwrap_or(1),
            Kind::Constant(_) | Kind::Local(_) => 1,
            Kind::Binary(_, left, right) => left.stack_size().max(1 + right.stack_size()),
            Kind::If(condition, yes, no) => condition
                .stack_size()
                .max(yes.stack_size())
                .max(no.stack_size()),
            Kind::Logical(_, left, right) => left.stack_size().max(right.stack_size()),
        }
    }
    fn emit(&self, code: &mut Vec<Instruction>, hosts: &mut Vec<Rc<HostFunction>>) {
        code.push(Instruction::Enter(self.span));
        match &self.kind {
            Kind::Host(host, arguments, callee_span) => {
                code.push(Instruction::Enter(*callee_span));
                for argument in arguments {
                    argument.emit(code, hosts);
                }
                let index = hosts.len();
                hosts.push(host.clone());
                code.push(Instruction::Host(index, arguments.len(), self.span));
            }
            Kind::Constant(value) => code.push(Instruction::Constant(*value)),
            Kind::Local(index) => code.push(Instruction::Local(*index)),
            Kind::Binary(op, left, right) => {
                left.emit(code, hosts);
                right.emit(code, hosts);
                code.push(Instruction::Binary(*op, self.span));
            }
            Kind::If(condition, yes, no) => {
                condition.emit(code, hosts);
                let branch = code.len();
                code.push(Instruction::JumpIfFalse(0, self.span));
                yes.emit(code, hosts);
                let end = code.len();
                code.push(Instruction::Jump(0));
                code[branch] = Instruction::JumpIfFalse(code.len(), self.span);
                no.emit(code, hosts);
                code[end] = Instruction::Jump(code.len());
            }
            Kind::Logical(short_value, left, right) => {
                left.emit(code, hosts);
                let branch = code.len();
                code.push(Instruction::ShortCircuit(*short_value, 0, self.span));
                right.emit(code, hosts);
                code.push(Instruction::EnsureBool(self.span));
                code[branch] = Instruction::ShortCircuit(*short_value, code.len(), self.span);
            }
        }
    }
}

fn charge(fuel: &mut usize, token: &CancellationToken, span: Span) -> Result<()> {
    if token.is_cancelled() {
        return error(span, "Execution cancelled");
    }
    if *fuel == 0 {
        return error(span, "Execution limit exceeded");
    }
    *fuel -= 1;
    Ok(())
}

fn to_value(value: Scalar) -> Value<'static> {
    match value {
        Scalar::Number(n) => Value::Number(n),
        Scalar::Bool(b) => Value::Bool(b),
    }
}
fn invoke(
    host: &HostFunction,
    args: &[Value<'_>],
    fuel: &mut usize,
    token: &CancellationToken,
    span: Span,
) -> Result<Scalar> {
    charge(fuel, token, span)?;
    if args.len() != host.parameters.len()
        || !host
            .parameters
            .iter()
            .zip(args)
            .all(|(ty, value)| ty.accepts(value))
    {
        return error(span, "Host function arguments do not match its signature");
    }
    let result = (host.callback)(args, token).map_err(|message| Error { span, message })?;
    if token.is_cancelled() {
        return error(span, "Execution cancelled");
    }
    if !host.result.accepts(&result) {
        return error(span, "Host function returned an invalid value");
    }
    match result {
        Value::Number(n) => Ok(Scalar::Number(n)),
        Value::Bool(b) => Ok(Scalar::Bool(b)),
        _ => error(span, "Experimental host result must be scalar"),
    }
}

#[derive(Debug, Clone, Copy)]
enum Instruction {
    Enter(Span),
    Constant(Scalar),
    Local(usize),
    Binary(Op, Span),
    Jump(usize),
    JumpIfFalse(usize, Span),
    ShortCircuit(bool, usize, Span),
    EnsureBool(Span),
    Host(usize, usize, Span),
}

#[derive(Debug)]
pub struct Tree {
    root: Node,
    arity: usize,
}
impl Tree {
    pub fn compile(source: &str, parameters: &[&str]) -> Result<Self> {
        Self::compile_with_hosts(source, parameters, &[])
    }
    pub fn compile_with_hosts(
        source: &str,
        parameters: &[&str],
        hosts: &[Rc<HostFunction>],
    ) -> Result<Self> {
        let parsed = parse(source);
        for (index, host) in hosts.iter().enumerate() {
            if hosts[..index]
                .iter()
                .any(|previous| previous.name == host.name)
            {
                return error(parsed.module.span, "Duplicate experimental host function");
            }
            if host
                .parameters
                .iter()
                .chain(std::iter::once(&host.result))
                .any(|ty| !matches!(ty, ValueType::Number | ValueType::Bool))
            {
                return error(
                    parsed.module.span,
                    "Experimental host signatures require scalar types",
                );
            }
        }
        if !parsed.is_valid() || parsed.module.items.len() != 1 {
            return error(parsed.module.span, "Expected one valid scalar expression");
        }
        let StmtKind::Expr(expr) = &parsed.module.items[0].kind else {
            return error(parsed.module.span, "Expected scalar expression");
        };
        if parameters
            .iter()
            .enumerate()
            .any(|(i, p)| parameters[..i].contains(p))
        {
            return error(expr.span, "Duplicate experimental parameter");
        }
        Ok(Self {
            root: Node::lower(expr, parameters, hosts, 0)?,
            arity: parameters.len(),
        })
    }
    pub fn run(&self, args: &[Scalar], fuel: usize, token: &CancellationToken) -> Result<Scalar> {
        if args.len() != self.arity {
            return error(self.root.span, "Argument count mismatch");
        }
        self.root.eval(args, &mut { fuel }, token)
    }
    pub fn bytecode(&self) -> Bytecode {
        let mut code = Vec::new();
        let mut hosts = Vec::new();
        self.root.emit(&mut code, &mut hosts);
        // Stack size is derived from trusted lowering, never from external bytecode.
        let stack_size = self.root.stack_size();
        Bytecode {
            code,
            arity: self.arity,
            span: self.root.span,
            stack_size,
            hosts,
        }
    }
}

#[derive(Debug)]
pub struct Bytecode {
    code: Vec<Instruction>,
    arity: usize,
    span: Span,
    stack_size: usize,
    hosts: Vec<Rc<HostFunction>>,
}
impl Bytecode {
    pub fn stack(&self) -> Vec<Scalar> {
        Vec::with_capacity(self.stack_size)
    }
    pub fn run(
        &self,
        args: &[Scalar],
        mut fuel: usize,
        token: &CancellationToken,
        stack: &mut Vec<Scalar>,
    ) -> Result<Scalar> {
        stack.clear();
        if args.len() != self.arity {
            return error(self.span, "Argument count mismatch");
        }
        let result = (|| {
            let mut pc = 0;
            while pc < self.code.len() {
                let instruction = self.code[pc];
                pc += 1;
                match instruction {
                    Instruction::Host(index, count, span) => {
                        let start = stack.len() - count;
                        let values: Vec<_> = stack[start..].iter().copied().map(to_value).collect();
                        let result = invoke(&self.hosts[index], &values, &mut fuel, token, span)?;
                        stack.truncate(start);
                        stack.push(result);
                    }
                    Instruction::Enter(span) => charge(&mut fuel, token, span)?,
                    Instruction::Constant(value) => stack.push(value),
                    Instruction::Local(index) => stack.push(args[index]),
                    Instruction::Binary(op, span) => {
                        let right = stack.pop().expect("compiler stack invariant");
                        let left = stack.pop().expect("compiler stack invariant");
                        stack.push(op.apply(left, right, span)?);
                    }
                    Instruction::Jump(target) => pc = target,
                    Instruction::JumpIfFalse(target, span) => {
                        let Scalar::Bool(condition) =
                            stack.pop().expect("compiler condition invariant")
                        else {
                            return error(span, "Condition must be boolean");
                        };
                        if !condition {
                            pc = target;
                        }
                    }
                    Instruction::ShortCircuit(short_value, target, span) => {
                        let Some(Scalar::Bool(value)) = stack.last() else {
                            return error(span, "Boolean operand required");
                        };
                        if *value == short_value {
                            pc = target;
                        } else {
                            stack.pop();
                        }
                    }
                    Instruction::EnsureBool(span) => {
                        if !matches!(stack.last(), Some(Scalar::Bool(_))) {
                            return error(span, "Boolean operand required");
                        }
                    }
                }
            }
            Ok(stack.pop().expect("compiler result invariant"))
        })();
        stack.clear();
        result
    }
}
