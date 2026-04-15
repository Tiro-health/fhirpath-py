#![cfg(feature = "wasm")]

use wasm_bindgen::prelude::*;

use crate::analyze;

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
/// Returns `Annotation[]` as a JavaScript value.
#[wasm_bindgen]
pub fn annotate_expression(expr: &str) -> Result<JsValue, JsError> {
    let annotations =
        analyze::annotate_expression(expr).map_err(|e| JsError::new(&e.to_string()))?;
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

/// Analyze a FHIRPath expression in the context of a Questionnaire.
///
/// Returns `{ annotations: Annotation[], diagnostics: Diagnostic[] }`.
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
    let result = analyze::analyze_expression(expr, &index.inner, &context)
        .map_err(|e| JsError::new(&e.to_string()))?;
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsError::new(&e.to_string()))
}
