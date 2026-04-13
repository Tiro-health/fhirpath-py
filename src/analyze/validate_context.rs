use crate::analyze::{
    Annotation, AnnotationKind, Diagnostic, DiagnosticCode, Severity, Span,
};
use crate::analyze::annotations::annotate_expression;
use crate::analyze::questionnaire_index::QuestionnaireIndex;

/// Extract the terminal linkId from a context expression's annotations.
fn extract_target_link_id(annotations: &[Annotation]) -> Option<&str> {
    // Prefer ItemReference (normal case for context expressions)
    for ann in annotations {
        if let AnnotationKind::ItemReference { link_ids } = &ann.kind {
            return link_ids.last().map(|s| s.as_str());
        }
    }
    // Fall back to AnswerReference if present
    for ann in annotations {
        if let AnnotationKind::AnswerReference { link_ids, .. } = &ann.kind {
            return link_ids.last().map(|s| s.as_str());
        }
    }
    None
}

/// Validate a templateExtractContext expression.
///
/// Checks:
/// - Target linkId exists in the Questionnaire
/// - Target is a group item (can have children to iterate over)
/// - If parent context provided, target is reachable from parent's target
pub fn validate_context(
    context_expr: &str,
    parent_context_expr: Option<&str>,
    index: &QuestionnaireIndex,
) -> Result<Vec<Diagnostic>, crate::ParseError> {
    let mut diagnostics = Vec::new();

    let annotations = annotate_expression(context_expr)?;

    let Some(target_link_id) = extract_target_link_id(&annotations) else {
        // Expression doesn't navigate to a recognizable item -- nothing to validate
        return Ok(diagnostics);
    };

    // Full expression span for diagnostics
    let expr_span = Span { start: 0, end: context_expr.len() };

    // Check 1: target exists
    if !index.contains(target_link_id) {
        diagnostics.push(Diagnostic {
            span: expr_span,
            severity: Severity::Error,
            code: DiagnosticCode::UnknownLinkId,
            message: format!("Context target linkId '{}' not found in Questionnaire", target_link_id),
        });
        return Ok(diagnostics); // Can't do further checks if target doesn't exist
    }

    // Check 2: target should be a group (for iteration contexts)
    // Only applies to ItemReference annotations (not AnswerReference)
    let is_item_ref = annotations.iter().any(|a| matches!(&a.kind, AnnotationKind::ItemReference { .. }));
    if is_item_ref {
        if let Some(item_type) = index.resolve_item_type(target_link_id) {
            if item_type != "group" {
                diagnostics.push(Diagnostic {
                    span: expr_span.clone(),
                    severity: Severity::Error,
                    code: DiagnosticCode::ContextTargetNotGroup,
                    message: format!(
                        "Context target '{}' is type '{}', expected 'group' for iteration context",
                        target_link_id, item_type
                    ),
                });
            }
        }
    }

    // Check 3: child reachable from parent
    if let Some(parent_expr) = parent_context_expr {
        let parent_annotations = annotate_expression(parent_expr)?;
        if let Some(parent_link_id) = extract_target_link_id(&parent_annotations) {
            if index.contains(parent_link_id)
                && !index.is_descendant(parent_link_id, target_link_id)
                && parent_link_id != target_link_id
            {
                diagnostics.push(Diagnostic {
                    span: expr_span,
                    severity: Severity::Error,
                    code: DiagnosticCode::ContextUnreachableFromParent,
                    message: format!(
                        "Context target '{}' is not reachable from parent context '{}'",
                        target_link_id, parent_link_id
                    ),
                });
            }
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_questionnaire() -> serde_json::Value {
        json!({
            "resourceType": "Questionnaire",
            "item": [
                {
                    "linkId": "group1",
                    "text": "Group One",
                    "type": "group",
                    "item": [
                        {
                            "linkId": "subgroup",
                            "text": "Sub Group",
                            "type": "group",
                            "item": [
                                {
                                    "linkId": "deep-choice",
                                    "text": "Deep",
                                    "type": "choice"
                                }
                            ]
                        },
                        {
                            "linkId": "bool1",
                            "text": "Yes or no",
                            "type": "boolean"
                        }
                    ]
                },
                {
                    "linkId": "group2",
                    "text": "Group Two",
                    "type": "group",
                    "item": [
                        {
                            "linkId": "string1",
                            "text": "Name",
                            "type": "string"
                        }
                    ]
                }
            ]
        })
    }

    #[test]
    fn test_valid_group_context() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_context(
            "%resource.item.where(linkId='group1')",
            None,
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unknown_link_id() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_context(
            "%resource.item.where(linkId='nonexistent')",
            None,
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::UnknownLinkId);
    }

    #[test]
    fn test_non_group_target() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_context(
            "%resource.item.where(linkId='group1').item.where(linkId='bool1')",
            None,
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::ContextTargetNotGroup);
        assert!(diags[0].message.contains("boolean"));
    }

    #[test]
    fn test_child_reachable_from_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_context(
            "item.where(linkId='subgroup')",
            Some("%resource.item.where(linkId='group1')"),
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_child_unreachable_from_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        // group2 is a valid group but not a descendant of group1
        let diags = validate_context(
            "item.where(linkId='group2')",
            Some("%resource.item.where(linkId='group1')"),
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::ContextUnreachableFromParent);
    }

    #[test]
    fn test_nested_group_context() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_context(
            "%resource.item.where(linkId='group1').item.where(linkId='subgroup')",
            None,
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_non_recognizable_expression() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        // An expression that doesn't match QR navigation patterns
        let diags = validate_context("Patient.name", None, &idx).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_same_context_as_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_context(
            "item.where(linkId='group1')",
            Some("%resource.item.where(linkId='group1')"),
            &idx,
        ).unwrap();
        // Same target as parent -- should be allowed (self-reference)
        assert!(diags.is_empty());
    }
}
