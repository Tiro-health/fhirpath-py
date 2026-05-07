use crate::analyze::annotations::{decompose_chain, ChainStepKind};
use crate::analyze::questionnaire_index::QuestionnaireIndex;
use crate::analyze::{Diagnostic, DiagnosticCode, Severity, Span};
use crate::ast::{Expr, Invocation};

/// Collect all linkIds referenced in where(linkId='...') clauses throughout the AST.
/// Returns Vec of (linkId_value, span_of_the_literal).
fn collect_link_id_references(expr: &Expr) -> Vec<(String, Span)> {
    let mut refs = Vec::new();
    collect_link_ids_recursive(expr, &mut refs);
    refs
}

fn collect_link_ids_recursive(expr: &Expr, out: &mut Vec<(String, Span)>) {
    // Try to decompose this node as a chain and extract linkIds from where() steps
    if matches!(
        expr,
        Expr::Invocation { .. } | Expr::Indexer { .. } | Expr::ExternalConstant { .. }
    ) {
        if let Some(steps) = decompose_chain(expr) {
            for step in &steps {
                if let ChainStepKind::Function {
                    name,
                    link_id: Some(id),
                    ..
                } = &step.kind
                {
                    if name == "where" {
                        if let Some(span) = &step.link_id_span {
                            out.push((id.clone(), span.clone()));
                        }
                    }
                }
            }
            // Walk the chain spine and recurse into function arguments and
            // indexer index expressions — those sub-trees aren't visited by
            // `decompose_chain` and may carry their own linkId references
            // (e.g. inside a non-linkId `where()` predicate).
            recurse_into_chain_args(expr, out);
            return;
        }
    }

    // Recurse into structural sub-expressions (operands of Binary, etc.).
    match expr {
        Expr::Binary { lhs, rhs, .. } => {
            collect_link_ids_recursive(lhs, out);
            collect_link_ids_recursive(rhs, out);
        }
        Expr::Polarity { operand, .. } => collect_link_ids_recursive(operand, out),
        Expr::Type { lhs, .. } => collect_link_ids_recursive(lhs, out),
        Expr::Indexer { receiver, index, .. } => {
            collect_link_ids_recursive(receiver, out);
            collect_link_ids_recursive(index, out);
        }
        Expr::Invocation { receiver, call, .. } => {
            if let Some(r) = receiver {
                collect_link_ids_recursive(r, out);
            }
            if let Invocation::Function { args, .. } = call {
                for a in args {
                    collect_link_ids_recursive(a, out);
                }
            }
        }
        Expr::Parenthesized { inner, .. } => collect_link_ids_recursive(inner, out),
        Expr::Literal(_) | Expr::ExternalConstant { .. } => {}
    }
}

/// Walk the chain spine, recursing into function-call arguments and indexer
/// index expressions. The spine itself is consumed by `decompose_chain`; arg
/// subtrees are independent and may contain their own where(linkId=…) clauses.
fn recurse_into_chain_args(expr: &Expr, out: &mut Vec<(String, Span)>) {
    let mut cursor = expr;
    loop {
        let stripped = match cursor {
            Expr::Parenthesized { inner, .. } => inner.as_ref(),
            other => other,
        };
        match stripped {
            Expr::Invocation { receiver, call, .. } => {
                if let Invocation::Function { args, .. } = call {
                    for a in args {
                        collect_link_ids_recursive(a, out);
                    }
                }
                match receiver {
                    Some(r) => cursor = r.as_ref(),
                    None => break,
                }
            }
            Expr::Indexer { receiver, index, .. } => {
                collect_link_ids_recursive(index, out);
                cursor = receiver.as_ref();
            }
            _ => break,
        }
    }
}

/// Internal: validate linkIds by parsing the expression.
/// Called from `analyze_expression` in mod.rs.
pub(crate) fn validate_link_ids_from_expr(
    expr: &str,
    index: &QuestionnaireIndex,
    scope_link_id: Option<&str>,
) -> Result<Vec<Diagnostic>, crate::ParseError> {
    let typed = crate::parse(expr)?;
    Ok(validate_link_ids_from_ast(&typed, index, scope_link_id))
}

fn validate_link_ids_from_ast(
    root: &Expr,
    index: &QuestionnaireIndex,
    scope_link_id: Option<&str>,
) -> Vec<Diagnostic> {
    let refs = collect_link_id_references(root);
    let mut diagnostics = Vec::new();

    for (link_id, span) in &refs {
        if !index.contains(link_id) {
            diagnostics.push(Diagnostic {
                span: span.clone(),
                severity: Severity::Error,
                code: DiagnosticCode::UnknownLinkId,
                message: format!("Unknown linkId '{}'", link_id),
            });
        } else if let Some(ctx) = scope_link_id {
            if !index.is_descendant(ctx, link_id) && link_id != ctx {
                diagnostics.push(Diagnostic {
                    span: span.clone(),
                    severity: Severity::Warning,
                    code: DiagnosticCode::UnreachableLinkId,
                    message: format!(
                        "linkId '{}' is not reachable from scope '{}'",
                        link_id, ctx
                    ),
                });
            }
        }
    }

    diagnostics
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
        let diags = validate_link_ids_from_expr("item.where(linkId='typo').answer.value", &idx, None).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::UnknownLinkId);
        assert!(diags[0].message.contains("typo"));
    }

    #[test]
    fn test_valid_link_id_no_diagnostic() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags =
            validate_link_ids_from_expr("item.where(linkId='choice1').answer.value", &idx, None).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unreachable_link_id() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids_from_expr(
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
        let diags = validate_link_ids_from_expr(
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
        let diags = validate_link_ids_from_expr(
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
        let diags = validate_link_ids_from_expr(expr, &idx, None).unwrap();
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
        let diags = validate_link_ids_from_expr(
            "item.where(linkId='group1').answer.value",
            &idx,
            Some("group1"),
        )
        .unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_nested_link_id_in_non_link_id_where_predicate_is_validated() {
        // The outer chain's first where() collects 'choice1'; the second
        // where() has a non-linkId predicate that itself contains a chain
        // referencing a bogus linkId. That nested reference must be
        // validated — see issue #60.
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids_from_expr(
            "item.where(linkId='choice1').answer.where(item.where(linkId='nonexistent').answer.value = 'yes')",
            &idx,
            None,
        )
        .unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::UnknownLinkId);
        assert!(diags[0].message.contains("nonexistent"));
    }

    #[test]
    fn test_nested_link_id_no_duplicate() {
        // Sanity: a linkId that appears only on the spine isn't picked up
        // twice when we also descend into args.
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids_from_expr(
            "item.where(linkId='nope').answer.where(value = 'x')",
            &idx,
            None,
        )
        .unwrap();
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("nope"));
    }

    #[test]
    fn test_no_where_clauses() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let diags = validate_link_ids_from_expr("Patient.name.given", &idx, None).unwrap();
        assert!(diags.is_empty());
    }
}
