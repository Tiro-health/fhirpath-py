use crate::analyze::annotations::{decompose_chain, ChainStepKind};
use crate::analyze::questionnaire_index::QuestionnaireIndex;
use crate::analyze::{Diagnostic, DiagnosticCode, Severity, Span};
use crate::parser::AstNode;

/// Collect all linkIds referenced in where(linkId='...') clauses throughout the AST.
/// Returns Vec of (linkId_value, span_of_the_literal).
fn collect_link_id_references(node: &AstNode) -> Vec<(String, Span)> {
    let mut refs = Vec::new();
    collect_link_ids_recursive(node, &mut refs);
    refs
}

fn collect_link_ids_recursive(node: &AstNode, out: &mut Vec<(String, Span)>) {
    // Try to decompose this node as a chain and extract linkIds from where() steps
    if node.node_type == "InvocationExpression" || node.node_type == "TermExpression" {
        if let Some(steps) = decompose_chain(node) {
            for step in &steps {
                if let ChainStepKind::Function {
                    name,
                    link_id: Some(id),
                } = &step.kind
                {
                    if name == "where" {
                        if let Some(span) = &step.link_id_span {
                            out.push((id.clone(), span.clone()));
                        }
                    }
                }
            }
            // Skip recursing into children — the chain decomposition from the outermost
            // node already captures all linkIds, and inner InvocationExpression nodes
            // would produce duplicates.
            return;
        }
    }

    // Recurse into children
    for child in &node.children {
        collect_link_ids_recursive(child, out);
    }
}

/// Validate linkIds in FHIRPath where() clauses against a QuestionnaireIndex.
///
/// Checks:
/// - Each linkId exists in the Questionnaire (`UnknownLinkId`)
/// - If `context_link_id` is provided, each linkId is reachable from context (`UnreachableLinkId`)
pub fn validate_link_ids(
    expr: &str,
    index: &QuestionnaireIndex,
    context_link_id: Option<&str>,
) -> Result<Vec<Diagnostic>, crate::ParseError> {
    let tokens = crate::lexer::tokenize(expr).map_err(crate::ParseError)?;
    let mut parser = crate::parser::Parser::new(&tokens);
    let root = parser.parse_entire_expression().map_err(crate::ParseError)?;

    let refs = collect_link_id_references(&root);
    let mut diagnostics = Vec::new();

    for (link_id, span) in &refs {
        if !index.contains(link_id) {
            diagnostics.push(Diagnostic {
                span: span.clone(),
                severity: Severity::Error,
                code: DiagnosticCode::UnknownLinkId,
                message: format!("Unknown linkId '{}'", link_id),
            });
        } else if let Some(ctx) = context_link_id {
            if !index.is_descendant(ctx, link_id) && link_id != ctx {
                diagnostics.push(Diagnostic {
                    span: span.clone(),
                    severity: Severity::Warning,
                    code: DiagnosticCode::UnreachableLinkId,
                    message: format!(
                        "linkId '{}' is not reachable from context '{}'",
                        link_id, ctx
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
                            "linkId": "choice1",
                            "text": "Pick one",
                            "type": "choice"
                        },
                        {
                            "linkId": "bool1",
                            "text": "Yes or no",
                            "type": "boolean"
                        }
                    ]
                },
                {
                    "linkId": "string1",
                    "text": "Free text",
                    "type": "string"
                }
            ]
        })
    }

    #[test]
    fn test_unknown_link_id() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids("item.where(linkId='typo').answer.value", &idx, None).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::UnknownLinkId);
        assert!(diags[0].message.contains("typo"));
    }

    #[test]
    fn test_valid_link_id_no_diagnostic() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags =
            validate_link_ids("item.where(linkId='choice1').answer.value", &idx, None).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unreachable_link_id() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids(
            "item.where(linkId='choice1').answer.value",
            &idx,
            Some("string1"), // choice1 is not a descendant of string1
        )
        .unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::UnreachableLinkId);
    }

    #[test]
    fn test_reachable_link_id_no_diagnostic() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids(
            "item.where(linkId='choice1').answer.value",
            &idx,
            Some("group1"), // choice1 IS a descendant of group1
        )
        .unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_nested_link_ids_both_checked() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids(
            "item.where(linkId='group1').item.where(linkId='nonexistent').answer.value",
            &idx,
            None,
        )
        .unwrap();
        // group1 is valid, nonexistent is not
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::UnknownLinkId);
        assert!(diags[0].message.contains("nonexistent"));
    }

    #[test]
    fn test_diagnostic_span_points_at_literal() {
        let expr = "item.where(linkId='typo').answer.value";
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids(expr, &idx, None).unwrap();
        assert_eq!(diags.len(), 1);
        // The span should cover the string literal 'typo' (including quotes)
        // — verify it's within the where() clause, not the whole expr
        assert!(diags[0].span.start > 0);
        assert!(diags[0].span.end < expr.len());
    }

    #[test]
    fn test_context_link_id_is_self() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        // Referencing the context itself should be ok
        let diags = validate_link_ids(
            "item.where(linkId='group1').answer.value",
            &idx,
            Some("group1"),
        )
        .unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_no_where_clauses() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids("Patient.name.given", &idx, None).unwrap();
        assert!(diags.is_empty());
    }
}
