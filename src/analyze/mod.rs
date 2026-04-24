/// FHIRPath expression analysis in SDC context.
///
/// Provides semantic annotations and diagnostics for any FHIRPath expression
/// that runs against a QuestionnaireResponse — templateExtractContext,
/// templateExtractValue, calculatedExpression, answerExpression, enableWhenExpression, etc.

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueAccessor {
    /// `.answer.value` (bare)
    Value,
    /// `.answer.value.code`
    Code,
    /// `.answer.value.display`
    Display,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

/// How precisely an annotation attributes to its linkId scope.
///
/// Ordered from most to least precise. Transitions only ever demote (never
/// promote) — once a chain loses precision, it stays lost.
///
/// - `Full` — the expression navigates to the complete set of answers/items
///   for the named linkId(s).
/// - `PartialPositional` — a positional selector (`first()`, `[i]`, `take(1)`, …)
///   narrowed the navigation to a subset. The linkId attribution still holds,
///   but the evaluator sees fewer items than a bare `where(linkId=…)` would.
/// - `WidenedScope` — a scope-widening operator (`descendants()`, `children()`,
///   `repeat()`) broke the structural parent-chain. linkIds are still known,
///   but the path to them is no longer deterministic — parent-context
///   reachability checks can't be trusted.
/// - `Unattributable` — an opaque operation (`iif()`, `select()`, non-linkId
///   `where(…)`, unknown function) severed precise attribution. linkIds
///   collected up to that point are preserved but consumers should treat them
///   as hints, not guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    #[default]
    Full,
    PartialPositional,
    WidenedScope,
    Unattributable,
}

impl Attribution {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Attribution::Full)
    }

    /// Rank within the lattice. Higher = more precise.
    fn rank(&self) -> u8 {
        match self {
            Attribution::Full => 3,
            Attribution::PartialPositional => 2,
            Attribution::WidenedScope => 1,
            Attribution::Unattributable => 0,
        }
    }

    /// Degrade `self` toward `floor`, never below.
    /// Returns the lower-ranked of the two.
    pub(crate) fn demote_to(self, floor: Attribution) -> Attribution {
        if floor.rank() < self.rank() {
            floor
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Annotation {
    pub span: Span,
    pub kind: AnnotationKind,
    #[serde(default, skip_serializing_if = "Attribution::is_default")]
    pub attribution: Attribution,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    UnknownLinkId,
    UnreachableLinkId,
    InvalidAccessorForType,
    MissingAccessorForCoding,
    ItemReferenceTargetsLeaf,
    ContextUnreachableFromParent,
    ExpressionNotAttributable,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
}

// ── Unified analysis API ────────────────────────────────────────────────

/// Describes the scope an expression runs within.
#[derive(Debug, Clone, Default)]
pub struct AnalysisContext {
    /// The linkId of the scope this expression runs within.
    /// Used for reachability checks — are the referenced linkIds
    /// descendants of this scope?
    pub scope_link_id: Option<String>,

    /// The parent scope's context expression (raw FHIRPath string).
    /// When provided, validates that item references in this expression
    /// are reachable from the parent scope's target.
    pub parent_context_expr: Option<String>,
}

/// Result of analyzing a FHIRPath expression.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
    let (ann, mut diagnostics) = annotations::annotate_expression_with_diagnostics(expr)?;

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

    // 3. Structural checks on item references — applies to ALL expressions
    diagnostics.extend(validate_context::validate_item_refs(
        expr,
        &ann,
        context.parent_context_expr.as_deref(),
        index,
    )?);

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
    fn test_item_ref_to_leaf_is_warning() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1').item.where(linkId='bool1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ItemReferenceTargetsLeaf));
    }

    #[test]
    fn test_item_ref_to_group_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().all(|d| d.code != DiagnosticCode::ItemReferenceTargetsLeaf));
    }

    #[test]
    fn test_parent_context_unreachable() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group2')",
            &idx,
            &AnalysisContext {
                parent_context_expr: Some("%resource.item.where(linkId='group1')".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ContextUnreachableFromParent));
    }

    #[test]
    fn test_parent_context_reachable() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='subgroup')",
            &idx,
            &AnalysisContext {
                parent_context_expr: Some("%resource.item.where(linkId='group1')".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.iter().all(|d| d.code != DiagnosticCode::ContextUnreachableFromParent));
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
