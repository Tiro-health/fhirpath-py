#![cfg(feature = "python")]

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::lexer::Token;
use crate::AstNode;

/// Parse a FHIRPath expression string into an AST dict.
#[pyfunction]
fn parse(py: Python<'_>, expr: &str) -> PyResult<PyObject> {
    let (ast, tokens) = rust_parse_with_tokens(expr)
        .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.0))?;

    // Build root dict: { "children": [top_expression] }
    let root = PyDict::new(py);
    let children = PyList::new(py, [ast_to_pydict(py, &ast, &tokens)?])?;
    root.set_item("children", children)?;
    Ok(root.into())
}

fn ast_to_pydict(py: Python<'_>, node: &AstNode, tokens: &[Token]) -> PyResult<PyObject> {
    let dict = PyDict::new(py);

    // type
    dict.set_item("type", node.node_type)?;

    // byte offsets into the source string
    dict.set_item("start", node.byte_start)?;
    dict.set_item("end", node.byte_end)?;

    // terminalNodeText
    let tnt = PyList::new(
        py,
        node.terminal_node_text
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
    )?;
    dict.set_item("terminalNodeText", tnt)?;

    // text — only for certain node types
    if has_text_field(node.node_type) {
        let text = compute_text(node, tokens);
        dict.set_item("text", text)?;
    }

    // children — only when non-empty
    if !node.children.is_empty() {
        let children: Vec<PyObject> = node
            .children
            .iter()
            .map(|child| ast_to_pydict(py, child, tokens))
            .collect::<PyResult<Vec<_>>>()?;
        let children_list = PyList::new(py, children)?;
        dict.set_item("children", children_list)?;
    }

    Ok(dict.into())
}

fn has_text_field(node_type: &str) -> bool {
    node_type.ends_with("Literal")
        || node_type == "LiteralTerm"
        || node_type == "Identifier"
        || node_type == "TypeSpecifier"
        || node_type == "InvocationExpression"
        || node_type == "TermExpression"
}

fn compute_text(node: &AstNode, tokens: &[Token]) -> String {
    // Concatenate token text for tokens in [token_start..token_end)
    let mut s = String::new();
    for i in node.token_start..node.token_end {
        s.push_str(&tokens[i].text);
    }
    s
}

#[pymodule]
pub fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("IMPLEMENTED", true)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    Ok(())
}

/// Internal helper: parse and also return the token stream (needed for text computation).
fn rust_parse_with_tokens(expr: &str) -> Result<(AstNode, Vec<Token>), crate::ParseError> {
    let tokens = crate::lexer::tokenize(expr).map_err(crate::ParseError)?;
    let mut p = crate::parser::Parser::new(&tokens);
    let ast = p.parse_entire_expression().map_err(crate::ParseError)?;
    Ok((ast, tokens))
}
