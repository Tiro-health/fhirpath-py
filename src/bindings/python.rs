#![cfg(feature = "python")]

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::analyze::{
    self, AnnotationKind, Attribution, Cardinality, DiagnosticCode, InferredType, Severity,
    ValueAccessor,
};
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

// ── QuestionnaireIndex wrapper ──────────────────────────────────────────

#[pyclass(name = "QuestionnaireIndex")]
struct PyQuestionnaireIndex {
    inner: analyze::QuestionnaireIndex,
}

#[pymethods]
impl PyQuestionnaireIndex {
    #[new]
    fn new(questionnaire_json: &str) -> PyResult<Self> {
        let value: serde_json::Value = serde_json::from_str(questionnaire_json)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(Self {
            inner: analyze::QuestionnaireIndex::build(&value),
        })
    }

    fn resolve_item_text(&self, link_id: &str) -> Option<String> {
        self.inner.resolve_item_text(link_id).map(|s| s.to_string())
    }

    fn resolve_code_display(&self, link_id: &str, system: &str, code: &str) -> Option<String> {
        self.inner
            .resolve_code_display(link_id, system, code)
            .map(|s| s.to_string())
    }

    fn contains(&self, link_id: &str) -> bool {
        self.inner.contains(link_id)
    }

    fn generate_completions(&self, py: Python<'_>, context_expr: &str) -> PyResult<PyObject> {
        let items = analyze::generate_completions(&self.inner, context_expr)
            .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.0))?;
        let result: Vec<PyObject> = items
            .iter()
            .map(|item| completion_item_to_pydict(py, item))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PyList::new(py, result)?.into())
    }
}

// ── Completion helpers ────────────────────────────────────────────────

fn completion_kind_to_str(kind: &analyze::CompletionItemKind) -> &'static str {
    match kind {
        analyze::CompletionItemKind::Value => "value",
        analyze::CompletionItemKind::Code => "code",
        analyze::CompletionItemKind::Display => "display",
    }
}

fn completion_item_to_pydict(py: Python<'_>, item: &analyze::CompletionItem) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    dict.set_item("label", item.label.as_str())?;
    match &item.detail {
        Some(d) => dict.set_item("detail", d.as_str())?,
        None => dict.set_item("detail", py.None())?,
    }
    dict.set_item("insert_text", item.insert_text.as_str())?;
    dict.set_item("filter_text", item.filter_text.as_str())?;
    dict.set_item("sort_text", item.sort_text.as_str())?;
    dict.set_item("kind", completion_kind_to_str(&item.kind))?;
    Ok(dict.into())
}

// ── Annotation / analysis helpers ──────────────────────────────────────

fn accessor_to_str(accessor: &ValueAccessor) -> &'static str {
    match accessor {
        ValueAccessor::Value => "value",
        ValueAccessor::Code => "code",
        ValueAccessor::Display => "display",
    }
}

fn severity_to_str(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn diagnostic_code_to_str(code: &DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::UnknownLinkId => "unknown_link_id",
        DiagnosticCode::UnreachableLinkId => "unreachable_link_id",
        DiagnosticCode::InvalidAccessorForType => "invalid_accessor_for_type",
        DiagnosticCode::MissingAccessorForCoding => "missing_accessor_for_coding",
        DiagnosticCode::ItemReferenceTargetsLeaf => "item_reference_targets_leaf",
        DiagnosticCode::ContextUnreachableFromParent => "context_unreachable_from_parent",
        DiagnosticCode::ExpressionNotAttributable => "expression_not_attributable",
        DiagnosticCode::ExpressionTypeMismatch => "expression_type_mismatch",
        DiagnosticCode::ExpressionCardinalityMismatch => "expression_cardinality_mismatch",
    }
}

fn annotation_to_pydict(py: Python<'_>, ann: &analyze::Annotation) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    dict.set_item("start", ann.span.start)?;
    dict.set_item("end", ann.span.end)?;
    match &ann.kind {
        AnnotationKind::AnswerReference { link_ids, accessor } => {
            dict.set_item("kind", "answer_reference")?;
            let ids = PyList::new(py, link_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
            dict.set_item("link_ids", ids)?;
            dict.set_item("accessor", accessor_to_str(accessor))?;
        }
        AnnotationKind::ItemReference { link_ids } => {
            dict.set_item("kind", "item_reference")?;
            let ids = PyList::new(py, link_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>())?;
            dict.set_item("link_ids", ids)?;
        }
        AnnotationKind::CodedValue {
            code,
            system,
            context_link_id,
        } => {
            dict.set_item("kind", "coded_value")?;
            dict.set_item("code", code.as_str())?;
            match system {
                Some(s) => dict.set_item("system", s.as_str())?,
                None => dict.set_item("system", py.None())?,
            }
            dict.set_item("context_link_id", context_link_id.as_str())?;
        }
    }
    // Only emit `attribution` when non-default, preserving v3.0.0 dict shapes
    // for callers that don't care about positional selectors.
    if !ann.attribution.is_default() {
        dict.set_item("attribution", attribution_to_str(&ann.attribution))?;
    }
    Ok(dict.into())
}

fn attribution_to_str(attribution: &Attribution) -> &'static str {
    match attribution {
        Attribution::Full => "full",
        Attribution::PartialPositional => "partial_positional",
        Attribution::WidenedScope => "widened_scope",
        Attribution::Unattributable => "unattributable",
    }
}

fn diagnostic_to_pydict(py: Python<'_>, diag: &analyze::Diagnostic) -> PyResult<PyObject> {
    let dict = PyDict::new(py);
    dict.set_item("severity", severity_to_str(&diag.severity))?;
    dict.set_item("code", diagnostic_code_to_str(&diag.code))?;
    dict.set_item("message", diag.message.as_str())?;
    dict.set_item("start", diag.span.start)?;
    dict.set_item("end", diag.span.end)?;
    Ok(dict.into())
}

// ── Python-facing functions ────────────────────────────────────────────

/// Annotate a FHIRPath expression, returning a list of annotation dicts.
#[pyfunction]
fn annotate_expression(py: Python<'_>, expr: &str) -> PyResult<PyObject> {
    let annotations = analyze::annotate_expression(expr)
        .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.0))?;
    let result: Vec<PyObject> = annotations
        .iter()
        .map(|a| annotation_to_pydict(py, a))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, result)?.into())
}

/// Resolve `%context` references in a FHIRPath expression at the AST level.
///
/// Parses both expressions, replaces every `%context` reference in `expr`
/// with the parsed `base_expr` AST, and returns the serialized result.
/// Returns `expr` unchanged when no `%context` reference exists.
/// Raises `SyntaxError` if either expression fails to parse.
#[pyfunction]
fn resolve_context(expr: &str, base_expr: &str) -> PyResult<String> {
    crate::resolve::resolve_context(expr, base_expr)
        .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.0))
}

/// Map an `InferredType` snake_case string to the enum.
fn parse_inferred_type(s: &str) -> Option<InferredType> {
    match s {
        "boolean" => Some(InferredType::Boolean),
        "string" => Some(InferredType::String),
        "integer" => Some(InferredType::Integer),
        "decimal" => Some(InferredType::Decimal),
        "date" => Some(InferredType::Date),
        "date_time" | "datetime" => Some(InferredType::DateTime),
        "time" => Some(InferredType::Time),
        "quantity" => Some(InferredType::Quantity),
        "coding" => Some(InferredType::Coding),
        "unknown" => Some(InferredType::Unknown),
        _ => None,
    }
}

/// Map a `Cardinality` snake_case string to the enum.
fn parse_cardinality(s: &str) -> Option<Cardinality> {
    match s {
        "singleton" => Some(Cardinality::Singleton),
        "collection" => Some(Cardinality::Collection),
        "unknown" => Some(Cardinality::Unknown),
        _ => None,
    }
}

/// Analyze a FHIRPath expression against a QuestionnaireIndex, returning
/// annotations and diagnostics.
#[pyfunction]
#[pyo3(signature = (expr, index, context_link_id=None, parent_context_expr=None, expected_result_type=None, expected_cardinality=None))]
fn analyze_expression(
    py: Python<'_>,
    expr: &str,
    index: &PyQuestionnaireIndex,
    context_link_id: Option<&str>,
    parent_context_expr: Option<&str>,
    expected_result_type: Option<&str>,
    expected_cardinality: Option<&str>,
) -> PyResult<PyObject> {
    let expected = match expected_result_type {
        Some(s) => Some(parse_inferred_type(s).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "unknown expected_result_type: {}",
                s
            ))
        })?),
        None => None,
    };
    let expected_card = match expected_cardinality {
        Some(s) => Some(parse_cardinality(s).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "unknown expected_cardinality: {}",
                s
            ))
        })?),
        None => None,
    };
    let context = analyze::AnalysisContext {
        scope_link_id: context_link_id.map(|s| s.to_string()),
        parent_context_expr: parent_context_expr.map(|s| s.to_string()),
        expected_result_type: expected,
        expected_cardinality: expected_card,
    };
    let result = analyze::analyze_expression(expr, &index.inner, &context)
        .map_err(|e| pyo3::exceptions::PySyntaxError::new_err(e.0))?;

    let annotations: Vec<PyObject> = result
        .annotations
        .iter()
        .map(|a| annotation_to_pydict(py, a))
        .collect::<PyResult<Vec<_>>>()?;
    let diagnostics: Vec<PyObject> = result
        .diagnostics
        .iter()
        .map(|d| diagnostic_to_pydict(py, d))
        .collect::<PyResult<Vec<_>>>()?;

    let dict = PyDict::new(py);
    dict.set_item("annotations", PyList::new(py, annotations)?)?;
    dict.set_item("diagnostics", PyList::new(py, diagnostics)?)?;
    Ok(dict.into())
}

#[pymodule]
pub fn _rust(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("IMPLEMENTED", true)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_class::<PyQuestionnaireIndex>()?;
    m.add_function(wrap_pyfunction!(annotate_expression, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_expression, m)?)?;
    m.add_function(wrap_pyfunction!(resolve_context, m)?)?;
    Ok(())
}

/// Internal helper: parse and also return the token stream (needed for text computation).
fn rust_parse_with_tokens(expr: &str) -> Result<(AstNode, Vec<Token>), crate::ParseError> {
    let tokens = crate::lexer::tokenize(expr).map_err(crate::ParseError)?;
    let mut p = crate::parser::Parser::new(&tokens);
    let ast = p.parse_entire_expression().map_err(crate::ParseError)?;
    Ok((ast, tokens))
}
