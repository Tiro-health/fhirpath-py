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

use crate::lexer::{tokenize, Token};
use crate::parser::{AstNode, Parser};
use crate::ParseError;

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
    let ast = Parser::new(&tokens).parse_entire_expression()?;
    let mut root = Map::new();
    root.insert(
        "children".to_string(),
        Value::Array(vec![lower_to_compat_json(&ast, &tokens)]),
    );
    Ok(Value::Object(root))
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
