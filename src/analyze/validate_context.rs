use crate::analyze::{
    Annotation, AnnotationKind, Diagnostic, DiagnosticCode, Severity, Span,
};
use crate::analyze::annotations::annotate_expression;
use crate::analyze::questionnaire_index::QuestionnaireIndex;

/// Extract the terminal linkId from annotations (prefers ItemReference).
fn extract_target_link_id(annotations: &[Annotation]) -> Option<&str> {
    for ann in annotations {
        if let AnnotationKind::ItemReference { link_ids } = &ann.kind {
            return link_ids.last().map(|s| s.as_str());
        }
    }
    for ann in annotations {
        if let AnnotationKind::AnswerReference { link_ids, .. } = &ann.kind {
            return link_ids.last().map(|s| s.as_str());
        }
    }
    None
}

/// Internal: validate context-specific constraints from pre-computed annotations.
/// Called from `analyze_expression` when `expects_item_target` is true.
pub(crate) fn validate_context_from_annotations(
    expr: &str,
    annotations: &[Annotation],
    parent_context_expr: Option<&str>,
    index: &QuestionnaireIndex,
) -> Result<Vec<Diagnostic>, crate::ParseError> {
    let mut diagnostics = Vec::new();

    let Some(target_link_id) = extract_target_link_id(annotations) else {
        return Ok(diagnostics);
    };

    let expr_span = Span { start: 0, end: expr.len() };

    if !index.contains(target_link_id) {
        diagnostics.push(Diagnostic {
            span: expr_span,
            severity: Severity::Error,
            code: DiagnosticCode::UnknownLinkId,
            message: format!("Context target linkId '{}' not found in Questionnaire", target_link_id),
        });
        return Ok(diagnostics);
    }

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
    use crate::analyze::{analyze_expression, AnalysisContext};
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
                                { "linkId": "deep-choice", "text": "Deep", "type": "choice" }
                            ]
                        },
                        { "linkId": "bool1", "text": "Yes or no", "type": "boolean" }
                    ]
                },
                {
                    "linkId": "group2",
                    "text": "Group Two",
                    "type": "group",
                    "item": [
                        { "linkId": "string1", "text": "Name", "type": "string" }
                    ]
                }
            ]
        })
    }

    fn context_opts(parent: Option<&str>) -> AnalysisContext {
        AnalysisContext {
            expects_item_target: true,
            parent_context_expr: parent.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_valid_group_context() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "%resource.item.where(linkId='group1')",
            &idx,
            &context_opts(None),
        ).unwrap();
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_non_group_target() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "%resource.item.where(linkId='group1').item.where(linkId='bool1')",
            &idx,
            &context_opts(None),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ContextTargetNotGroup));
    }

    #[test]
    fn test_child_reachable_from_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "item.where(linkId='subgroup')",
            &idx,
            &context_opts(Some("%resource.item.where(linkId='group1')")),
        ).unwrap();
        let context_diags: Vec<_> = result.diagnostics.iter()
            .filter(|d| matches!(d.code, DiagnosticCode::ContextUnreachableFromParent))
            .collect();
        assert!(context_diags.is_empty());
    }

    #[test]
    fn test_child_unreachable_from_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group2')",
            &idx,
            &context_opts(Some("%resource.item.where(linkId='group1')")),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ContextUnreachableFromParent));
    }

    #[test]
    fn test_nested_group_context() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "%resource.item.where(linkId='group1').item.where(linkId='subgroup')",
            &idx,
            &context_opts(None),
        ).unwrap();
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_same_context_as_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1')",
            &idx,
            &context_opts(Some("%resource.item.where(linkId='group1')")),
        ).unwrap();
        let context_diags: Vec<_> = result.diagnostics.iter()
            .filter(|d| matches!(d.code, DiagnosticCode::ContextUnreachableFromParent | DiagnosticCode::ContextTargetNotGroup))
            .collect();
        assert!(context_diags.is_empty());
    }
}
