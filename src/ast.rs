//! Typed FHIRPath AST.
//!
//! Phase 1 scaffolding — these types compile in isolation and nothing in
//! the parser produces them yet. Phase 2 will rewrite the parser to emit
//! `Expr` directly; Phase 3 will add a lowering layer that walks `Expr`
//! and produces the legacy `AstNode` tree (for the Python-binding-shape
//! dict) so the FFI surface stays byte-for-byte identical during the
//! transition.
//!
//! Notes on `raw` fields:
//! - `Identifier::raw` keeps backticks for delimited identifiers.
//! - `Literal::String::raw` keeps the surrounding single quotes.
//! - `ExternalConstantId::String::raw` keeps the surrounding quotes.
//! - Escape sequences are NOT decoded — the lexer's text is stored verbatim.

use serde::Serialize;

use crate::Span;

#[derive(Debug, Clone, Serialize)]
pub enum Expr {
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Polarity {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Type {
        lhs: Box<Expr>,
        op: TypeOp,
        type_spec: TypeSpecifier,
        span: Span,
    },
    Indexer {
        receiver: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Invocation {
        receiver: Option<Box<Expr>>,
        call: Invocation,
        span: Span,
    },
    Parenthesized {
        inner: Box<Expr>,
        span: Span,
    },
    Literal(Literal),
    ExternalConstant {
        ident: ExternalConstantId,
        span: Span,
    },
}

impl Expr {
    /// Span covering this expression in the source.
    pub fn span(&self) -> &Span {
        match self {
            Expr::Binary { span, .. }
            | Expr::Polarity { span, .. }
            | Expr::Type { span, .. }
            | Expr::Indexer { span, .. }
            | Expr::Invocation { span, .. }
            | Expr::Parenthesized { span, .. }
            | Expr::ExternalConstant { span, .. } => span,
            Expr::Literal(lit) => lit.span(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum Invocation {
    Member {
        ident: Identifier,
    },
    Function {
        name: Identifier,
        args: Vec<Expr>,
        span: Span,
    },
    This {
        span: Span,
    },
    Index {
        span: Span,
    },
    Total {
        span: Span,
    },
}

impl Invocation {
    /// Span covering this invocation in the source.
    pub fn span(&self) -> &Span {
        match self {
            Invocation::Member { ident } => &ident.span,
            Invocation::Function { span, .. }
            | Invocation::This { span }
            | Invocation::Index { span }
            | Invocation::Total { span } => span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum Literal {
    Null { span: Span },
    Boolean { value: bool, span: Span },
    String { raw: String, span: Span },
    Number { raw: String, span: Span },
    Quantity { number: String, unit: Unit, span: Span },
    DateTime { raw: String, span: Span },
    Time { raw: String, span: Span },
}

impl Literal {
    /// Span covering this literal in the source.
    pub fn span(&self) -> &Span {
        match self {
            Literal::Null { span }
            | Literal::Boolean { span, .. }
            | Literal::String { span, .. }
            | Literal::Number { span, .. }
            | Literal::Quantity { span, .. }
            | Literal::DateTime { span, .. }
            | Literal::Time { span, .. } => span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum Unit {
    String { raw: String, span: Span },
    DateTimePrecision { word: String, span: Span },
    PluralDateTimePrecision { word: String, span: Span },
}

impl Unit {
    /// Span covering this unit in the source.
    pub fn span(&self) -> &Span {
        match self {
            Unit::String { span, .. }
            | Unit::DateTimePrecision { span, .. }
            | Unit::PluralDateTimePrecision { span, .. } => span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum ExternalConstantId {
    Identifier(Identifier),
    String { raw: String, span: Span },
}

impl ExternalConstantId {
    /// Span covering this identifier reference in the source.
    pub fn span(&self) -> &Span {
        match self {
            ExternalConstantId::Identifier(id) => &id.span,
            ExternalConstantId::String { span, .. } => span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Identifier {
    pub raw: String,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeSpecifier {
    pub qualified: QualifiedIdentifier,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct QualifiedIdentifier {
    pub parts: Vec<Identifier>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum BinOp {
    Implies,
    Or,
    Xor,
    And,
    In,
    Contains,
    Eq,
    NotEq,
    EquivTilde,
    NotEquivTilde,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Union,
    Plus,
    Minus,
    Concat,
    Mul,
    TrueDiv,
    IntDiv,
    Mod,
}

impl BinOp {
    /// FHIRPath surface form for this operator. Matches the lexeme that the
    /// lexer emits for the corresponding token, so Phase-3 lowering can use
    /// this to populate the legacy `terminal_node_text` slot.
    pub fn keyword(self) -> &'static str {
        match self {
            BinOp::Implies => "implies",
            BinOp::Or => "or",
            BinOp::Xor => "xor",
            BinOp::And => "and",
            BinOp::In => "in",
            BinOp::Contains => "contains",
            BinOp::Eq => "=",
            BinOp::NotEq => "!=",
            BinOp::EquivTilde => "~",
            BinOp::NotEquivTilde => "!~",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LtEq => "<=",
            BinOp::GtEq => ">=",
            BinOp::Union => "|",
            BinOp::Plus => "+",
            BinOp::Minus => "-",
            BinOp::Concat => "&",
            BinOp::Mul => "*",
            BinOp::TrueDiv => "/",
            BinOp::IntDiv => "div",
            BinOp::Mod => "mod",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
}

impl UnaryOp {
    pub fn keyword(self) -> &'static str {
        match self {
            UnaryOp::Plus => "+",
            UnaryOp::Minus => "-",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum TypeOp {
    Is,
    As,
}

impl TypeOp {
    pub fn keyword(self) -> &'static str {
        match self {
            TypeOp::Is => "is",
            TypeOp::As => "as",
        }
    }
}
