//! Two-stage immutable scalar factories; invocation is driven by the Rust harness.
use super::*;

#[derive(Debug, Clone)]
enum Body {
    Tree(Rc<Tree>),
    Bytecode(Rc<Bytecode>),
}
#[derive(Debug)]
pub struct Factory {
    body: Body,
    captured_slots: Rc<[usize]>,
    outer_arity: usize,
    inner_arity: usize,
    span: Span,
}
#[derive(Debug, Clone)]
pub struct Closure {
    body: Body,
    captured: Rc<[Scalar]>,
    arity: usize,
    span: Span,
}

fn names<'a>(parameters: &[Expr<'a>]) -> Result<Vec<&'a str>> {
    let mut result = Vec::new();
    for parameter in parameters {
        let ExprKind::Name(name) = &parameter.kind else {
            return error(
                parameter.span,
                "Experimental closure requires named parameters",
            );
        };
        if result.contains(&name.text) {
            return error(parameter.span, "Duplicate experimental parameter");
        }
        result.push(name.text);
    }
    Ok(result)
}
fn referenced(node: &Node, used: &mut [bool]) {
    match &node.kind {
        Kind::Local(index) => used[*index] = true,
        Kind::Constant(_) => (),
        Kind::Binary(_, a, b) | Kind::Logical(_, a, b) => {
            referenced(a, used);
            referenced(b, used);
        }
        Kind::If(a, b, c) => {
            referenced(a, used);
            referenced(b, used);
            referenced(c, used);
        }
        Kind::Host(_, args, _) => {
            for arg in args {
                referenced(arg, used);
            }
        }
    }
}
impl Factory {
    pub fn compile(source: &str) -> Result<Self> {
        let parsed = parse(source);
        if !parsed.is_valid() || parsed.module.items.len() != 1 {
            return error(parsed.module.span, "Expected one closure factory");
        }
        let StmtKind::Expr(Expr {
            kind:
                ExprKind::Lambda {
                    parameters: outer,
                    body,
                },
            ..
        }) = &parsed.module.items[0].kind
        else {
            return error(parsed.module.span, "Expected outer lambda");
        };
        let ExprKind::Lambda {
            parameters: inner,
            body,
        } = &body.kind
        else {
            return error(body.span, "Expected returned lambda");
        };
        let outer_names = names(outer)?;
        let inner_names = names(inner)?;
        let candidates: Vec<_> = outer_names
            .iter()
            .enumerate()
            .filter(|(_, name)| !inner_names.contains(name))
            .collect();
        let mut bindings = inner_names.clone();
        bindings.extend(candidates.iter().map(|(_, name)| **name));
        let preliminary = Node::lower(body, &bindings, &[], 0)?;
        let mut used = vec![false; bindings.len()];
        referenced(&preliminary, &mut used);
        let captured_slots: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| used[inner_names.len() + i])
            .map(|(_, (slot, _))| *slot)
            .collect();
        let mut bindings = inner_names.clone();
        bindings.extend(captured_slots.iter().map(|slot| outer_names[*slot]));
        let tree = Tree {
            root: Node::lower(body, &bindings, &[], 0)?,
            arity: bindings.len(),
        };
        Ok(Self {
            body: Body::Tree(Rc::new(tree)),
            captured_slots: captured_slots.into(),
            outer_arity: outer_names.len(),
            inner_arity: inner_names.len(),
            span: parsed.module.span,
        })
    }
    pub fn bytecode(&self) -> Self {
        let body = match &self.body {
            Body::Tree(tree) => Body::Bytecode(Rc::new(tree.bytecode())),
            body => body.clone(),
        };
        Self {
            body,
            captured_slots: self.captured_slots.clone(),
            outer_arity: self.outer_arity,
            inner_arity: self.inner_arity,
            span: self.span,
        }
    }
    pub fn bind(&self, args: &[Scalar]) -> Result<Closure> {
        if args.len() != self.outer_arity {
            return error(self.span, "Argument count mismatch");
        }
        let captured: Vec<_> = self.captured_slots.iter().map(|slot| args[*slot]).collect();
        Ok(Closure {
            body: self.body.clone(),
            captured: captured.into(),
            arity: self.inner_arity,
            span: self.span,
        })
    }
}
impl Closure {
    pub fn captured_count(&self) -> usize {
        self.captured.len()
    }
    pub fn stack(&self) -> Vec<Scalar> {
        match &self.body {
            Body::Tree(_) => Vec::new(),
            Body::Bytecode(vm) => vm.stack(),
        }
    }
    pub fn run(
        &self,
        args: &[Scalar],
        fuel: usize,
        token: &CancellationToken,
        stack: &mut Vec<Scalar>,
    ) -> Result<Scalar> {
        stack.clear();
        if args.len() != self.arity {
            return error(self.span, "Argument count mismatch");
        }
        // Identical argument/capture assembly for both backends, included in timing.
        let mut frame = Vec::with_capacity(args.len() + self.captured.len());
        frame.extend_from_slice(args);
        frame.extend_from_slice(&self.captured);
        match &self.body {
            Body::Tree(tree) => tree.run(&frame, fuel, token),
            Body::Bytecode(vm) => vm.run(&frame, fuel, token, stack),
        }
    }
}
