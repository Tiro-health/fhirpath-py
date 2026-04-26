#![cfg(feature = "wasm")]

use wasm_bindgen::prelude::*;

use crate::analyze;
use crate::utf16::byte_to_utf16_offset;

/// Translate a span's byte offsets into UTF-16 code-unit offsets so JS hosts
/// can use them directly for `String.slice`, CodeMirror diagnostics, etc.
fn convert_span(span: &mut analyze::Span, expr: &str) {
    span.start = byte_to_utf16_offset(expr, span.start);
    span.end = byte_to_utf16_offset(expr, span.end);
}

fn convert_annotation_spans(annotations: &mut [analyze::Annotation], expr: &str) {
    for ann in annotations.iter_mut() {
        convert_span(&mut ann.span, expr);
    }
}

fn convert_diagnostic_spans(diagnostics: &mut [analyze::Diagnostic], expr: &str) {
    for diag in diagnostics.iter_mut() {
        convert_span(&mut diag.span, expr);
    }
}

/// Parse a FHIRPath expression string into an AST.
///
/// Returns the AST as a JavaScript object.
#[wasm_bindgen]
pub fn parse(expr: &str) -> Result<JsValue, JsError> {
    let ast = crate::parse(expr).map_err(|e| JsError::new(&e.0))?;
    serde_wasm_bindgen::to_value(&ast).map_err(|e| JsError::new(&e.to_string()))
}

/// Annotate a FHIRPath expression, extracting answer references,
/// item references, and coded values.
///
/// Returns `Annotation[]` as a JavaScript value. Span offsets are
/// UTF-16 code units, suitable for `String.prototype.slice` and
/// CodeMirror/Monaco position math.
#[wasm_bindgen]
pub fn annotate_expression(expr: &str) -> Result<JsValue, JsError> {
    let mut annotations =
        analyze::annotate_expression(expr).map_err(|e| JsError::new(&e.to_string()))?;
    convert_annotation_spans(&mut annotations, expr);
    serde_wasm_bindgen::to_value(&annotations).map_err(|e| JsError::new(&e.to_string()))
}

/// A Questionnaire index for use in expression analysis.
///
/// Build one from a FHIR Questionnaire JSON string, then pass it
/// to `analyze_expression` for semantic validation.
#[wasm_bindgen]
pub struct QuestionnaireIndex {
    inner: analyze::QuestionnaireIndex,
}

#[wasm_bindgen]
impl QuestionnaireIndex {
    /// Build a `QuestionnaireIndex` from a FHIR Questionnaire JSON string.
    #[wasm_bindgen(constructor)]
    pub fn new(questionnaire_json: &str) -> Result<QuestionnaireIndex, JsError> {
        let value: serde_json::Value = serde_json::from_str(questionnaire_json)
            .map_err(|e| JsError::new(&format!("Invalid JSON: {e}")))?;
        Ok(QuestionnaireIndex {
            inner: analyze::QuestionnaireIndex::build(&value),
        })
    }

    /// Generate completion items for autocomplete given a context expression.
    pub fn generate_completions(&self, context_expr: &str) -> Result<JsValue, JsError> {
        let items = analyze::generate_completions(&self.inner, context_expr)
            .map_err(|e| JsError::new(&e.to_string()))?;
        serde_wasm_bindgen::to_value(&items).map_err(|e| JsError::new(&e.to_string()))
    }
}

/// Resolve `%context` references in a FHIRPath expression at the AST level.
///
/// Parses both expressions, replaces every `%context` reference in `expr`
/// with the parsed `base_expr` AST, and returns the serialized result.
/// Returns `expr` unchanged when no `%context` reference exists.
#[wasm_bindgen]
pub fn resolve_context(expr: &str, base_expr: &str) -> Result<String, JsError> {
    crate::resolve::resolve_context(expr, base_expr).map_err(|e| JsError::new(&e.0))
}

/// Analyze a FHIRPath expression in the context of a Questionnaire.
///
/// Returns `{ annotations: Annotation[], diagnostics: Diagnostic[] }`.
/// Span offsets are UTF-16 code units, suitable for
/// `String.prototype.slice` and CodeMirror/Monaco position math.
///
/// - `expr` -- the FHIRPath expression string
/// - `index` -- a `QuestionnaireIndex` built from the Questionnaire
/// - `scope_link_id` -- optional linkId of the item scope (for reachability checks)
/// - `parent_context_expr` -- optional parent context expression (raw FHIRPath)
#[wasm_bindgen]
pub fn analyze_expression(
    expr: &str,
    index: &QuestionnaireIndex,
    scope_link_id: Option<String>,
    parent_context_expr: Option<String>,
) -> Result<JsValue, JsError> {
    let context = analyze::AnalysisContext {
        scope_link_id,
        parent_context_expr,
    };
    let mut result = analyze::analyze_expression(expr, &index.inner, &context)
        .map_err(|e| JsError::new(&e.to_string()))?;
    convert_annotation_spans(&mut result.annotations, expr);
    convert_diagnostic_spans(&mut result.diagnostics, expr);
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
}
