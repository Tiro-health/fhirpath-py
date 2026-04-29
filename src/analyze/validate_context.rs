use crate::analyze::{
    Annotation, AnnotationKind, Attribution, Diagnostic, DiagnosticCode, Severity, Span,
};
use crate::analyze::annotations::annotate_expression;
use crate::analyze::questionnaire_index::QuestionnaireIndex;

/// Extract the annotation we'd use to attribute the expression as a whole
/// (prefers ItemReference, falls back to AnswerReference).
fn extract_target_annotation(annotations: &[Annotation]) -> Option<&Annotation> {
    for ann in annotations {
        if let AnnotationKind::ItemReference { .. } = &ann.kind {
            return Some(ann);
        }
    }
    for ann in annotations {
        if let AnnotationKind::AnswerReference { .. } = &ann.kind {
            return Some(ann);
        }
    }
    None
}

fn annotation_link_ids(ann: &Annotation) -> &[String] {
    match &ann.kind {
        AnnotationKind::ItemReference { link_ids } => link_ids,
        AnnotationKind::AnswerReference { link_ids, .. } => link_ids,
        AnnotationKind::CodedValue { .. } => &[],
    }
}

fn is_trusted_for_reachability(attribution: Attribution) -> bool {
    matches!(
        attribution,
        Attribution::Full | Attribution::PartialPositional
    )
}

/// Validate structural properties of item references in any expression.
///
/// Checks (applied to ALL expressions, not just context expressions):
/// - ItemReference targets a leaf item (no children) → warning
///
/// Additional check when `parent_context_expr` is provided:
/// - Target is reachable from parent scope → error if not
pub(crate) fn validate_item_refs(
    expr: &str,
    annotations: &[Annotation],
    parent_context_expr: Option<&str>,
    index: &QuestionnaireIndex,
) -> Result<Vec<Diagnostic>, crate::ParseError> {
    let mut diagnostics = Vec::new();

    for ann in annotations {
        let AnnotationKind::ItemReference { link_ids } = &ann.kind else {
            continue;
        };
        let Some(target_link_id) = link_ids.last().map(|s| s.as_str()) else {
            continue;
        };
        if !index.contains(target_link_id) {
            continue; // unknown linkId is reported by validate_link_ids
        }

        // An ItemReference to a leaf item is likely unintended — you're
        // scoping to an item that has no children to iterate over.
        if let Some(info) = index.get(target_link_id) {
            if info.children.is_empty() {
                diagnostics.push(Diagnostic {
                    span: ann.span.clone(),
                    severity: Severity::Warning,
                    code: DiagnosticCode::ItemReferenceTargetsLeaf,
                    message: format!(
                        "Item reference targets '{}' (type '{}') which has no child items",
                        target_link_id, info.item_type,
                    ),
                });
            }
        }
    }

    // Parent context reachability — check if this expression's item target
    // is reachable from the parent scope. Skip when either side's attribution
    // is degraded below PartialPositional: we'd be making claims about paths
    // we no longer model precisely.
    if let Some(parent_expr) = parent_context_expr {
        let Some(target_ann) = extract_target_annotation(annotations) else {
            return Ok(diagnostics);
        };
        if !is_trusted_for_reachability(target_ann.attribution) {
            return Ok(diagnostics);
        }
        let Some(target_link_id) = annotation_link_ids(target_ann).last() else {
            return Ok(diagnostics);
        };
        if !index.contains(target_link_id) {
            return Ok(diagnostics);
        }
        let parent_annotations = annotate_expression(parent_expr)?;
        let Some(parent_ann) = extract_target_annotation(&parent_annotations) else {
            return Ok(diagnostics);
        };
        if !is_trusted_for_reachability(parent_ann.attribution) {
            return Ok(diagnostics);
        }
        if let Some(parent_link_id) = annotation_link_ids(parent_ann).last() {
            if index.contains(parent_link_id)
                && !index.is_descendant(parent_link_id, target_link_id)
                && parent_link_id != target_link_id
            {
                diagnostics.push(Diagnostic {
                    span: Span { start: 0, end: expr.len() },
                    severity: Severity::Error,
                    code: DiagnosticCode::ContextUnreachableFromParent,
                    message: format!(
                        "Target '{}' is not reachable from parent context '{}'",
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
    use crate::analyze::{analyze_expression, AnalysisContext, DiagnosticCode};
    use crate::analyze::questionnaire_index::QuestionnaireIndex;
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

    #[test]
    fn test_item_ref_to_group_is_clean() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "%resource.item.where(linkId='group1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().all(|d| d.code != DiagnosticCode::ItemReferenceTargetsLeaf));
    }

    #[test]
    fn test_item_ref_to_leaf_warns() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "%resource.item.where(linkId='group1').item.where(linkId='bool1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ItemReferenceTargetsLeaf));
    }

    #[test]
    fn test_child_reachable_from_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
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
    fn test_child_unreachable_from_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
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
    fn test_same_context_as_parent() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1')",
            &idx,
            &AnalysisContext {
                parent_context_expr: Some("%resource.item.where(linkId='group1')".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.iter().all(|d| d.code != DiagnosticCode::ContextUnreachableFromParent));
    }

    #[test]
    fn test_non_recognizable_expression() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let result = analyze_expression("Patient.name", &idx, &AnalysisContext::default()).unwrap();
        assert!(result.diagnostics.is_empty());
    }
}
