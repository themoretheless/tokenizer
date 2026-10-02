//! Rush syntax tree. Names and literal spellings borrow the source document.
use themoretheless_tokenizer_core::{Diagnostic, Lexed, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name<'s> {
    pub text: &'s str,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type<'s> {
    pub name: Name<'s>,
    pub path: Vec<Name<'s>>,
    pub arguments: Vec<Type<'s>>,
}

impl Type<'_> {
    pub fn qualified_name(&self) -> std::borrow::Cow<'_, str> {
        if self.path.len() <= 1 {
            std::borrow::Cow::Borrowed(self.name.text)
        } else {
            std::borrow::Cow::Owned(
                self.path
                    .iter()
                    .map(|n| n.text)
                    .collect::<Vec<_>>()
                    .join("."),
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter<'s> {
    pub pattern: Expr<'s>,
    pub ty: Option<Type<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module<'s> {
    pub span: Span,
    pub items: Vec<Stmt<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'s> {
    pub span: Span,
    pub stmts: std::rc::Rc<Vec<Stmt<'s>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt<'s> {
    pub span: Span,
    pub kind: StmtKind<'s>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StmtKind<'s> {
    Struct {
        name: Name<'s>,
        fields: Vec<(Name<'s>, Type<'s>)>,
    },
    Enum {
        name: Name<'s>,
        variants: Vec<(Name<'s>, Vec<Type<'s>>)>,
    },
    Export(Vec<Name<'s>>),
    Import(Name<'s>),
    Destructure {
        pattern: Expr<'s>,
        value: Expr<'s>,
    },
    Function {
        name: Name<'s>,
        parameters: Vec<Parameter<'s>>,
        result: Option<Type<'s>>,
        body: Block<'s>,
    },
    Declaration {
        name: Name<'s>,
        constant: bool,
        ty: Option<Type<'s>>,
        value: Expr<'s>,
    },
    Return(Option<Expr<'s>>),
    Yield(Expr<'s>),
    If {
        condition: Expr<'s>,
        then_block: Block<'s>,
        else_block: Option<Block<'s>>,
    },
    While {
        condition: Expr<'s>,
        body: Block<'s>,
    },
    For {
        binding: Name<'s>,
        iterable: Expr<'s>,
        body: Block<'s>,
    },
    Break,
    Continue,
    Expr(Expr<'s>),
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr<'s> {
    pub span: Span,
    pub kind: ExprKind<'s>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind<'s> {
    Try(Box<Expr<'s>>),
    If {
        condition: Box<Expr<'s>>,
        then_value: Box<Expr<'s>>,
        else_value: Box<Expr<'s>>,
    },
    Lambda {
        parameters: Vec<Expr<'s>>,
        body: Box<Expr<'s>>,
    },
    Name(Name<'s>),
    Number(&'s str),
    String(&'s str),
    Bool(bool),
    Null,
    Unary {
        operator: &'s str,
        value: Box<Expr<'s>>,
    },
    Binary {
        operator: &'s str,
        left: Box<Expr<'s>>,
        right: Box<Expr<'s>>,
    },
    Assign {
        operator: &'s str,
        target: Box<Expr<'s>>,
        value: Box<Expr<'s>>,
    },
    Call {
        callee: Box<Expr<'s>>,
        arguments: Vec<Expr<'s>>,
    },
    Member {
        object: Box<Expr<'s>>,
        field: Name<'s>,
    },
    Index {
        object: Box<Expr<'s>>,
        index: Box<Expr<'s>>,
    },
    List(Vec<Expr<'s>>),
    Tuple(Vec<Expr<'s>>),
    Map(Vec<(Expr<'s>, Expr<'s>)>),
    Pipeline {
        input: Box<Expr<'s>>,
        stages: Vec<Expr<'s>>,
    },
    Match {
        value: Box<Expr<'s>>,
        arms: Vec<MatchArm<'s>>,
    },
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm<'s> {
    pub span: Span,
    pub pattern: Expr<'s>,
    pub guard: Option<Expr<'s>>,
    pub value: Expr<'s>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'s> {
    pub source: &'s str,
    pub lexed: Lexed,
    pub module: Module<'s>,
    pub diagnostics: Vec<Diagnostic>,
    /// Independent of diagnostic storage limits, including a zero limit.
    pub(crate) valid: bool,
    pub(crate) roles: Vec<(Span, &'static str)>,
}

impl Parse<'_> {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.valid
    }
}
