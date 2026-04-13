/// FHIRPath expression analysis in SDC context.
///
/// Provides semantic annotations and diagnostics for any FHIRPath expression
/// that runs against a QuestionnaireResponse — templateExtractContext,
/// templateExtractValue, calculatedExpression, answerExpression, enableWhenExpression, etc.

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValueAccessor {
    /// `.answer.value` (bare)
    Value,
    /// `.answer.value.code`
    Code,
    /// `.answer.value.display`
    Display,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationKind {
    /// Path navigating to a QR answer value.
    AnswerReference {
        link_ids: Vec<String>,
        accessor: ValueAccessor,
    },
    /// Path navigating to a QR item (no `.answer.value`).
    ItemReference { link_ids: Vec<String> },
    /// A coded value compared against an answer reference.
    CodedValue {
        code: String,
        system: Option<String>,
        context_link_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub span: Span,
    pub kind: AnnotationKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagnosticCode {
    UnknownLinkId,
    UnreachableLinkId,
    InvalidAccessorForType,
    MissingAccessorForCoding,
    ContextTargetNotGroup,
    ContextUnreachableFromParent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
}

// ── Unified analysis API ────────────────────────────────────────────────

/// Describes the role an expression plays, which determines which
/// validations apply. Role-agnostic: works for any SDC expression type.
#[derive(Debug, Clone, Default)]
pub struct AnalysisContext {
    /// The linkId of the scope this expression runs within.
    /// Used for reachability checks — are the referenced linkIds
    /// descendants of this scope?
    ///
    /// Example: if a Composition section has templateExtractContext
    /// targeting 'group1', child expressions should pass
    /// `scope_link_id: Some("group1".into())`.
    pub scope_link_id: Option<String>,

    /// Whether this expression is expected to navigate to an item
    /// (for scoping/iteration), rather than extracting a value.
    ///
    /// When true, the target item must be a `group` type.
    /// Set this for templateExtractContext expressions.
    pub expects_item_target: bool,

    /// The parent scope's context expression (raw FHIRPath string).
    /// When provided, validates that this expression's target is
    /// reachable from the parent scope's target.
    ///
    /// Only meaningful when `expects_item_target` is true.
    pub parent_context_expr: Option<String>,
}

/// Result of analyzing a FHIRPath expression.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpressionAnalysis {
    pub annotations: Vec<Annotation>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Analyze a FHIRPath expression in the context of a Questionnaire.
///
/// Produces semantic annotations (answer references, coded values, item
/// references) and validation diagnostics (unknown linkIds, type mismatches,
/// unreachable items, etc.).
///
/// Works for any SDC expression type — templateExtractContext,
/// templateExtractValue, calculatedExpression, answerExpression,
/// enableWhenExpression, initialExpression, etc.
pub fn analyze_expression(
    expr: &str,
    index: &questionnaire_index::QuestionnaireIndex,
    context: &AnalysisContext,
) -> Result<ExpressionAnalysis, crate::ParseError> {
    let ann = annotations::annotate_expression(expr)?;
    let mut diagnostics = Vec::new();

    // 1. LinkId validation — applies to ALL expressions
    diagnostics.extend(validate_link_ids::validate_link_ids_from_expr(
        expr,
        index,
        context.scope_link_id.as_deref(),
    )?);

    // 2. Value type validation — applies to expressions with answer references
    diagnostics.extend(validate_types::validate_value_types_from_annotations(
        &ann, index,
    ));

    // 3. Context-specific checks — only when expression targets an item for scoping
    if context.expects_item_target {
        diagnostics.extend(validate_context::validate_context_from_annotations(
            expr,
            &ann,
            context.parent_context_expr.as_deref(),
            index,
        )?);
    }

    Ok(ExpressionAnalysis {
        annotations: ann,
        diagnostics,
    })
}

// ── Internal modules ────────────────────────────────────────────────────

pub mod annotations;
pub mod questionnaire_index;

pub(crate) mod validate_context;
pub(crate) mod validate_link_ids;
pub(crate) mod validate_types;

// ── Convenience re-exports ──────────────────────────────────────────────

pub use annotations::annotate_expression;
pub use questionnaire_index::QuestionnaireIndex;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn questionnaire() -> serde_json::Value {
        json!({
            "resourceType": "Questionnaire",
            "item": [
                {
                    "linkId": "group1",
                    "type": "group",
                    "item": [
                        { "linkId": "choice1", "type": "choice" },
                        { "linkId": "bool1", "type": "boolean" },
                        {
                            "linkId": "subgroup",
                            "type": "group",
                            "item": [
                                { "linkId": "deep", "type": "string" }
                            ]
                        }
                    ]
                },
                {
                    "linkId": "group2",
                    "type": "group",
                    "item": [
                        { "linkId": "other", "type": "decimal" }
                    ]
                }
            ]
        })
    }

    #[test]
    fn test_value_expression_gets_linkid_and_type_checks() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='bool1').answer.value.code",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(!result.annotations.is_empty());
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::InvalidAccessorForType));
    }

    #[test]
    fn test_value_expression_unknown_linkid() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='typo').answer.value",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::UnknownLinkId));
    }

    #[test]
    fn test_value_expression_with_scope() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='other').answer.value",
            &idx,
            &AnalysisContext {
                scope_link_id: Some("group1".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::UnreachableLinkId));
    }

    #[test]
    fn test_context_expression_non_group_target() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1').item.where(linkId='bool1')",
            &idx,
            &AnalysisContext {
                expects_item_target: true,
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ContextTargetNotGroup));
    }

    #[test]
    fn test_context_expression_no_context_checks_without_flag() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1').item.where(linkId='bool1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().all(|d| d.code != DiagnosticCode::ContextTargetNotGroup));
    }

    #[test]
    fn test_clean_value_expression() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='choice1').answer.value.code",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.is_empty());
        assert!(!result.annotations.is_empty());
    }

    #[test]
    fn test_calculated_expression_with_scope() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='deep').answer.value",
            &idx,
            &AnalysisContext {
                scope_link_id: Some("subgroup".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_non_qr_expression_passes_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "Patient.name.given",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.annotations.is_empty());
        assert!(result.diagnostics.is_empty());
    }
}
