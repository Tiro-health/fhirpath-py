//! Python-binding-shape JSON projection of the parser AST.
//!
//! This module is the canonical "what Python sees" function in pure Rust.
//! It replicates `bindings::python::ast_to_pydict` exactly, but produces a
//! `serde_json::Value` instead of a `PyObject`. We use it for snapshot tests
//! that prove the FFI dict shape is stable across parser refactors.
//!
//! The shape:
//! ```text
//! {
//!   "type": <node_type str>,
//!   "start": <byte_start usize>,
//!   "end": <byte_end usize>,
//!   "terminalNodeText": [<str>, ...],     // always present, may be empty
//!   "text": <str>,                          // only when has_text_field(node_type)
//!   "children": [<dict>, ...]               // only when non-empty
//! }
//! ```
//! Root is wrapped: `{"children": [<top_expr_dict>]}`.

use serde_json::{json, Map, Value};

use crate::ast::{
    BinOp, Expr, ExternalConstantId, Identifier, Invocation, Literal, QualifiedIdentifier,
    TypeSpecifier, Unit,
};
use crate::lexer::{tokenize, Token};
use crate::parser::Parser;
use crate::{ParseError, Span};

/// Legacy ANTLR-shaped AST node. Produced by the lowering layer
/// (`compat::lower`) from the typed `Expr` tree, then consumed by the
/// Python/WASM bindings, the analyze passes, and `resolve.rs`.
///
/// This is the FFI contract: the dict shape produced by `lower_to_compat_json`
/// is what Python's `fhirpathpy` consumers see, and the snapshot tests
/// gate it byte-for-byte.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AstNode {
    pub node_type: &'static str,
    pub terminal_node_text: Vec<String>,
    pub children: Vec<AstNode>,
    /// Index range [token_start..token_end] into the token vec (for text computation).
    pub token_start: usize,
    pub token_end: usize,
    /// Byte offsets into the source string.
    pub byte_start: usize,
    pub byte_end: usize,
}

impl AstNode {
    /// Construct an empty `AstNode` whose token-range and byte-range start at
    /// the given token index. The `token_end` and `byte_end` are seeded to the
    /// same start position; callers are expected to widen the range as they
    /// build subnodes.
    pub fn new(node_type: &'static str, token_start: usize, tokens: &[Token]) -> Self {
        AstNode {
            node_type,
            terminal_node_text: Vec::new(),
            children: Vec::new(),
            token_start,
            token_end: token_start,
            byte_start: tokens[token_start].byte_start,
            byte_end: tokens[token_start].byte_start,
        }
    }
}

/// Lower an AST node + its tokens into the Python-binding-shape JSON dict.
pub fn lower_to_compat_json(node: &AstNode, tokens: &[Token]) -> Value {
    let mut map = Map::new();
    map.insert("type".to_string(), Value::String(node.node_type.to_string()));
    map.insert("start".to_string(), json!(node.byte_start));
    map.insert("end".to_string(), json!(node.byte_end));

    let tnt: Vec<Value> = node
        .terminal_node_text
        .iter()
        .map(|s| Value::String(s.clone()))
        .collect();
    map.insert("terminalNodeText".to_string(), Value::Array(tnt));

    if has_text_field(node.node_type) {
        map.insert("text".to_string(), Value::String(compute_text(node, tokens)));
    }

    if !node.children.is_empty() {
        let children: Vec<Value> = node
            .children
            .iter()
            .map(|c| lower_to_compat_json(c, tokens))
            .collect();
        map.insert("children".to_string(), Value::Array(children));
    }

    Value::Object(map)
}

/// Parse an expression and produce the wrapped Python-binding-shape root JSON:
/// `{"children": [lower_to_compat_json(&ast, &tokens)]}`.
pub fn parse_to_compat_json(expr: &str) -> Result<Value, ParseError> {
    let tokens = tokenize(expr)?;
    let expr_ast = Parser::new(&tokens).parse_entire_expression()?;
    let ast = lower(&expr_ast, &tokens);
    let mut root = Map::new();
    root.insert(
        "children".to_string(),
        Value::Array(vec![lower_to_compat_json(&ast, &tokens)]),
    );
    Ok(Value::Object(root))
}

// ── Lowering layer: typed Expr → legacy ANTLR-shaped AstNode ─────────────

/// Lower a typed `Expr` (produced by the parser) into the legacy
/// ANTLR-shaped `AstNode` tree consumed by the bindings, the analyze
/// passes, and `resolve.rs`.
pub fn lower(expr: &Expr, tokens: &[Token]) -> AstNode {
    lower_expr(expr, tokens)
}

fn lower_expr(expr: &Expr, tokens: &[Token]) -> AstNode {
    match expr {
        Expr::Binary { op, lhs, rhs, span } => AstNode {
            node_type: bin_op_node_type(*op),
            terminal_node_text: vec![op.keyword().to_string()],
            children: vec![lower_expr(lhs, tokens), lower_expr(rhs, tokens)],
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Expr::Polarity { op, operand, span } => AstNode {
            node_type: "PolarityExpression",
            terminal_node_text: vec![op.keyword().to_string()],
            children: vec![lower_expr(operand, tokens)],
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Expr::Type {
            lhs,
            op,
            type_spec,
            span,
        } => AstNode {
            node_type: "TypeExpression",
            terminal_node_text: vec![op.keyword().to_string()],
            children: vec![lower_expr(lhs, tokens), lower_type_specifier(type_spec)],
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Expr::Indexer {
            receiver,
            index,
            span,
        } => AstNode {
            node_type: "IndexerExpression",
            terminal_node_text: vec!["[".to_string(), "]".to_string()],
            children: vec![lower_expr(receiver, tokens), lower_expr(index, tokens)],
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Expr::Invocation {
            receiver: Some(rcv),
            call,
            span,
        } => AstNode {
            node_type: "InvocationExpression",
            terminal_node_text: vec![".".to_string()],
            children: vec![lower_expr(rcv, tokens), lower_invocation(call, tokens)],
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Expr::Invocation {
            receiver: None,
            call,
            span,
        } => {
            let inner = lower_invocation(call, tokens);
            let inv_term = wrap(span, "InvocationTerm", inner);
            wrap(span, "TermExpression", inv_term)
        }
        Expr::Parenthesized { inner, span } => {
            let par = AstNode {
                node_type: "ParenthesizedTerm",
                terminal_node_text: vec!["(".to_string(), ")".to_string()],
                children: vec![lower_expr(inner, tokens)],
                token_start: span.token_start,
                token_end: span.token_end,
                byte_start: span.byte_start,
                byte_end: span.byte_end,
            };
            wrap(span, "TermExpression", par)
        }
        Expr::Literal(lit) => {
            let span = lit.span();
            let lit_node = lower_literal(lit);
            let lit_term = wrap(span, "LiteralTerm", lit_node);
            wrap(span, "TermExpression", lit_term)
        }
        Expr::ExternalConstant { ident, span } => {
            let id_node = lower_external_constant_id(ident);
            let ec = AstNode {
                node_type: "ExternalConstant",
                terminal_node_text: vec!["%".to_string()],
                children: vec![id_node],
                token_start: span.token_start,
                token_end: span.token_end,
                byte_start: span.byte_start,
                byte_end: span.byte_end,
            };
            let ec_term = wrap(span, "ExternalConstantTerm", ec);
            wrap(span, "TermExpression", ec_term)
        }
    }
}

/// Wrap a single child in a span-matching parent node with the given node_type.
fn wrap(span: &Span, node_type: &'static str, child: AstNode) -> AstNode {
    AstNode {
        node_type,
        terminal_node_text: Vec::new(),
        children: vec![child],
        token_start: span.token_start,
        token_end: span.token_end,
        byte_start: span.byte_start,
        byte_end: span.byte_end,
    }
}

fn lower_invocation(inv: &Invocation, tokens: &[Token]) -> AstNode {
    match inv {
        Invocation::Member { ident } => AstNode {
            node_type: "MemberInvocation",
            terminal_node_text: Vec::new(),
            children: vec![lower_identifier(ident)],
            token_start: ident.span.token_start,
            token_end: ident.span.token_end,
            byte_start: ident.span.byte_start,
            byte_end: ident.span.byte_end,
        },
        Invocation::Function { name, args, span } => {
            let mut functn_children = vec![lower_identifier(name)];
            if !args.is_empty() {
                let first_span = args[0].span();
                let last_span = args.last().unwrap().span();
                let pl_tnt: Vec<String> =
                    (0..args.len().saturating_sub(1)).map(|_| ",".to_string()).collect();
                let pl_children: Vec<AstNode> =
                    args.iter().map(|a| lower_expr(a, tokens)).collect();
                let paramlist = AstNode {
                    node_type: "ParamList",
                    terminal_node_text: pl_tnt,
                    children: pl_children,
                    token_start: first_span.token_start,
                    token_end: last_span.token_end,
                    byte_start: first_span.byte_start,
                    byte_end: last_span.byte_end,
                };
                functn_children.push(paramlist);
            }
            let functn = AstNode {
                node_type: "Functn",
                terminal_node_text: vec!["(".to_string(), ")".to_string()],
                children: functn_children,
                token_start: span.token_start,
                token_end: span.token_end,
                byte_start: span.byte_start,
                byte_end: span.byte_end,
            };
            AstNode {
                node_type: "FunctionInvocation",
                terminal_node_text: Vec::new(),
                children: vec![functn],
                token_start: span.token_start,
                token_end: span.token_end,
                byte_start: span.byte_start,
                byte_end: span.byte_end,
            }
        }
        Invocation::This { span } => leaf("ThisInvocation", "$this", span),
        Invocation::Index { span } => leaf("IndexInvocation", "$index", span),
        Invocation::Total { span } => leaf("TotalInvocation", "$total", span),
    }
}

fn leaf(node_type: &'static str, text: &str, span: &Span) -> AstNode {
    AstNode {
        node_type,
        terminal_node_text: vec![text.to_string()],
        children: Vec::new(),
        token_start: span.token_start,
        token_end: span.token_end,
        byte_start: span.byte_start,
        byte_end: span.byte_end,
    }
}

fn lower_literal(lit: &Literal) -> AstNode {
    match lit {
        Literal::Null { span } => AstNode {
            node_type: "NullLiteral",
            terminal_node_text: vec!["{".to_string(), "}".to_string()],
            children: Vec::new(),
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Literal::Boolean { value, span } => leaf(
            "BooleanLiteral",
            if *value { "true" } else { "false" },
            span,
        ),
        Literal::String { raw, span } => leaf("StringLiteral", raw, span),
        Literal::Number { raw, span } => leaf("NumberLiteral", raw, span),
        Literal::DateTime { raw, span } => leaf("DateTimeLiteral", raw, span),
        Literal::Time { raw, span } => leaf("TimeLiteral", raw, span),
        Literal::Quantity {
            number,
            unit,
            span,
        } => {
            let unit_node = lower_unit(unit);
            let quantity = AstNode {
                node_type: "Quantity",
                terminal_node_text: vec![number.clone()],
                children: vec![unit_node],
                token_start: span.token_start,
                token_end: span.token_end,
                byte_start: span.byte_start,
                byte_end: span.byte_end,
            };
            AstNode {
                node_type: "QuantityLiteral",
                terminal_node_text: Vec::new(),
                children: vec![quantity],
                token_start: span.token_start,
                token_end: span.token_end,
                byte_start: span.byte_start,
                byte_end: span.byte_end,
            }
        }
    }
}

fn lower_unit(unit: &Unit) -> AstNode {
    match unit {
        Unit::String { raw, span } => AstNode {
            node_type: "Unit",
            terminal_node_text: vec![raw.clone()],
            children: Vec::new(),
            token_start: span.token_start,
            token_end: span.token_end,
            byte_start: span.byte_start,
            byte_end: span.byte_end,
        },
        Unit::DateTimePrecision { word, span } => {
            let inner = leaf("DateTimePrecision", word, span);
            wrap(span, "Unit", inner)
        }
        Unit::PluralDateTimePrecision { word, span } => {
            let inner = leaf("PluralDateTimePrecision", word, span);
            wrap(span, "Unit", inner)
        }
    }
}

fn lower_external_constant_id(ext: &ExternalConstantId) -> AstNode {
    match ext {
        ExternalConstantId::Identifier(id) => lower_identifier(id),
        ExternalConstantId::String { raw, span } => leaf("Identifier", raw, span),
    }
}

fn lower_identifier(id: &Identifier) -> AstNode {
    leaf("Identifier", &id.raw, &id.span)
}

fn lower_type_specifier(ts: &TypeSpecifier) -> AstNode {
    let qi = lower_qualified_identifier(&ts.qualified);
    AstNode {
        node_type: "TypeSpecifier",
        terminal_node_text: Vec::new(),
        children: vec![qi],
        token_start: ts.span.token_start,
        token_end: ts.span.token_end,
        byte_start: ts.span.byte_start,
        byte_end: ts.span.byte_end,
    }
}

fn lower_qualified_identifier(qi: &QualifiedIdentifier) -> AstNode {
    let dot_count = qi.parts.len().saturating_sub(1);
    let tnt: Vec<String> = (0..dot_count).map(|_| ".".to_string()).collect();
    let children: Vec<AstNode> = qi.parts.iter().map(lower_identifier).collect();
    AstNode {
        node_type: "QualifiedIdentifier",
        terminal_node_text: tnt,
        children,
        token_start: qi.span.token_start,
        token_end: qi.span.token_end,
        byte_start: qi.span.byte_start,
        byte_end: qi.span.byte_end,
    }
}

fn bin_op_node_type(op: BinOp) -> &'static str {
    match op {
        BinOp::Implies => "ImpliesExpression",
        BinOp::Or | BinOp::Xor => "OrExpression",
        BinOp::And => "AndExpression",
        BinOp::In | BinOp::Contains => "MembershipExpression",
        BinOp::Eq | BinOp::NotEq | BinOp::EquivTilde | BinOp::NotEquivTilde => "EqualityExpression",
        BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => "InequalityExpression",
        BinOp::Union => "UnionExpression",
        BinOp::Plus | BinOp::Minus | BinOp::Concat => "AdditiveExpression",
        BinOp::Mul | BinOp::TrueDiv | BinOp::IntDiv | BinOp::Mod => "MultiplicativeExpression",
    }
}

/// Returns `true` for node types whose dict gets a `"text"` field.
fn has_text_field(node_type: &str) -> bool {
    node_type.ends_with("Literal")
        || node_type == "LiteralTerm"
        || node_type == "Identifier"
        || node_type == "TypeSpecifier"
        || node_type == "InvocationExpression"
        || node_type == "TermExpression"
}

fn compute_text(node: &AstNode, tokens: &[Token]) -> String {
    let mut s = String::new();
    for i in node.token_start..node.token_end {
        s.push_str(&tokens[i].text);
    }
    s
}
