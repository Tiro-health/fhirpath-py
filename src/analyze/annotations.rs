/// Annotation extraction from FHIRPath AST nodes.
///
/// Pass 1 identifies answer/item references by matching the QR navigation pattern
/// (e.g. `item.where(linkId='x').answer.value.code`).
/// Pass 2 identifies coded values in equality/equivalence comparisons against those references.

use crate::parser::AstNode;

use super::{Annotation, AnnotationKind, Span, ValueAccessor};

// ── Helper types ────────────────────────────────────────────────────────

pub(crate) enum ChainStepKind {
    Identifier(String),
    Function { name: String, link_id: Option<String> },
    External,
}

pub(crate) struct ChainStep {
    pub(crate) kind: ChainStepKind,
    pub(crate) link_id_span: Option<Span>,
}

// ── Helper functions ────────────────────────────────────────────────────

/// If `node` is an Identifier, return its name.
pub(crate) fn get_identifier_name(node: &AstNode) -> Option<&str> {
    if node.node_type == "Identifier" {
        node.terminal_node_text.first().map(|s| s.as_str())
    } else {
        None
    }
}

/// Navigate through TermExpression -> LiteralTerm -> StringLiteral to extract the string value.
/// Strips the surrounding quote characters.
fn extract_string_value(node: &AstNode) -> Option<String> {
    let mut current = node;

    // Walk through TermExpression -> LiteralTerm -> StringLiteral
    if current.node_type == "TermExpression" {
        current = current.children.first()?;
    }
    if current.node_type == "LiteralTerm" {
        current = current.children.first()?;
    }
    if current.node_type == "StringLiteral" {
        let raw = current.terminal_node_text.first()?;
        if raw.len() >= 2 {
            return Some(raw[1..raw.len() - 1].to_string());
        }
    }
    None
}

/// Extract a linkId from a `where(linkId='...')` call.
/// `functn` is the `Functn` node inside a `FunctionInvocation`.
/// Returns the linkId string value and the byte span of the string literal.
pub(crate) fn extract_link_id_from_where(functn: &AstNode) -> Option<(String, Span)> {
    // Functn.children: [Identifier("where"), ParamList]
    let name_node = functn.children.first()?;
    if get_identifier_name(name_node)? != "where" {
        return None;
    }

    let param_list = functn.children.get(1)?;
    if param_list.node_type != "ParamList" {
        return None;
    }

    let equality = param_list.children.first()?;
    if equality.node_type != "EqualityExpression" {
        return None;
    }

    // EqualityExpression.children: [left, right]
    let left = equality.children.first()?;
    let right = equality.children.get(1)?;

    // left should be TermExpression -> InvocationTerm -> MemberInvocation -> Identifier("linkId")
    let left_name = extract_member_identifier_name(left)?;
    if left_name != "linkId" {
        return None;
    }

    // Navigate to the StringLiteral node to get its byte span
    let string_node = find_string_literal_node(right)?;
    let value = extract_string_value(right)?;
    Some((value, Span { start: string_node.byte_start, end: string_node.byte_end }))
}

/// Navigate through TermExpression -> LiteralTerm -> StringLiteral to find the StringLiteral node.
fn find_string_literal_node(node: &AstNode) -> Option<&AstNode> {
    let mut current = node;
    if current.node_type == "TermExpression" {
        current = current.children.first()?;
    }
    if current.node_type == "LiteralTerm" {
        current = current.children.first()?;
    }
    if current.node_type == "StringLiteral" {
        Some(current)
    } else {
        None
    }
}

/// Navigate TermExpression -> InvocationTerm -> MemberInvocation -> Identifier to get the name.
fn extract_member_identifier_name(node: &AstNode) -> Option<&str> {
    let mut current = node;
    if current.node_type == "TermExpression" {
        current = current.children.first()?;
    }
    if current.node_type == "InvocationTerm" {
        current = current.children.first()?;
    }
    if current.node_type == "MemberInvocation" {
        current = current.children.first()?;
    }
    get_identifier_name(current)
}

/// Recursively flatten an InvocationExpression tree into a chain of steps.
pub(crate) fn decompose_chain(node: &AstNode) -> Option<Vec<ChainStep>> {
    match node.node_type {
        "TermExpression" => {
            let inner = node.children.first()?;
            match inner.node_type {
                "InvocationTerm" => {
                    let member = inner.children.first()?;
                    if member.node_type == "MemberInvocation" {
                        let ident = member.children.first()?;
                        let name = get_identifier_name(ident)?.to_string();
                        Some(vec![ChainStep {
                            kind: ChainStepKind::Identifier(name),
                            link_id_span: None,
                        }])
                    } else {
                        None
                    }
                }
                "ExternalConstantTerm" => {
                    let ext_const = inner.children.first()?;
                    if ext_const.node_type != "ExternalConstant" {
                        return None;
                    }
                    let ident = ext_const.children.first()?;
                    let _name = get_identifier_name(ident)?;
                    Some(vec![ChainStep {
                        kind: ChainStepKind::External,
                        link_id_span: None,
                    }])
                }
                _ => None,
            }
        }
        "InvocationExpression" => {
            let receiver = node.children.first()?;
            let member = node.children.get(1)?;

            let mut steps = decompose_chain(receiver)?;

            match member.node_type {
                "MemberInvocation" => {
                    let ident = member.children.first()?;
                    let name = get_identifier_name(ident)?.to_string();
                    steps.push(ChainStep {
                        kind: ChainStepKind::Identifier(name),
                        link_id_span: None,
                    });
                }
                "FunctionInvocation" => {
                    let functn = member.children.first()?;
                    if functn.node_type != "Functn" {
                        return None;
                    }
                    let func_ident = functn.children.first()?;
                    let func_name = get_identifier_name(func_ident)?.to_string();
                    let (link_id, link_id_span) = if func_name == "where" {
                        match extract_link_id_from_where(functn) {
                            Some((id, span)) => (Some(id), Some(span)),
                            None => (None, None),
                        }
                    } else {
                        (None, None)
                    };
                    steps.push(ChainStep {
                        kind: ChainStepKind::Function {
                            name: func_name,
                            link_id,
                        },
                        link_id_span,
                    });
                }
                _ => return None,
            }

            Some(steps)
        }
        _ => None,
    }
}

// ── QR state machine ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum QRState {
    Start,
    Item,
    ItemFiltered,
    Answer,
    Value,
    Prop(ValueAccessor),
    Rejected,
}

enum MatchKind {
    AnswerRef(ValueAccessor),
    ItemRef,
}

struct MatchResult {
    link_ids: Vec<String>,
    kind: MatchKind,
}

/// Run the QR navigation state machine over a chain of steps.
fn match_qr_path(steps: &[ChainStep]) -> Option<MatchResult> {
    let mut state = QRState::Start;
    let mut link_ids: Vec<String> = Vec::new();

    for step in steps {
        state = match (&state, &step.kind) {
            // Start + External -> Start (skip %context, %resource, etc.)
            (QRState::Start, ChainStepKind::External) => QRState::Start,

            // Start + "item" -> Item
            (QRState::Start, ChainStepKind::Identifier(name)) if name == "item" => QRState::Item,

            // Item + where(linkId=...) -> ItemFiltered
            (
                QRState::Item,
                ChainStepKind::Function {
                    name,
                    link_id: Some(id),
                },
            ) if name == "where" => {
                link_ids.push(id.clone());
                QRState::ItemFiltered
            }

            // ItemFiltered + "answer" -> Answer
            (QRState::ItemFiltered, ChainStepKind::Identifier(name)) if name == "answer" => {
                QRState::Answer
            }

            // ItemFiltered + "item" -> Item (nested navigation)
            (QRState::ItemFiltered, ChainStepKind::Identifier(name)) if name == "item" => {
                QRState::Item
            }

            // Answer + "item" -> Item (nested navigation through answer)
            (QRState::Answer, ChainStepKind::Identifier(name)) if name == "item" => {
                QRState::Item
            }

            // Answer + "value" -> Value
            (QRState::Answer, ChainStepKind::Identifier(name)) if name == "value" => {
                QRState::Value
            }

            // Value + "code" -> Prop(Code)
            (QRState::Value, ChainStepKind::Identifier(name)) if name == "code" => {
                QRState::Prop(ValueAccessor::Code)
            }

            // Value + "display" -> Prop(Display)
            (QRState::Value, ChainStepKind::Identifier(name)) if name == "display" => {
                QRState::Prop(ValueAccessor::Display)
            }

            // Everything else -> Rejected
            _ => QRState::Rejected,
        };

        if matches!(state, QRState::Rejected) {
            return None;
        }
    }

    match state {
        QRState::Value => Some(MatchResult {
            link_ids,
            kind: MatchKind::AnswerRef(ValueAccessor::Value),
        }),
        QRState::Prop(accessor) => Some(MatchResult {
            link_ids,
            kind: MatchKind::AnswerRef(accessor),
        }),
        QRState::ItemFiltered if !link_ids.is_empty() => Some(MatchResult {
            link_ids,
            kind: MatchKind::ItemRef,
        }),
        _ => None,
    }
}

// ── AST walkers ─────────────────────────────────────────────────────────

/// Pass 1: Find answer and item references by DFS over the AST.
fn find_answer_refs(node: &AstNode, out: &mut Vec<Annotation>) {
    // Try to match this node as a QR navigation chain
    if node.node_type == "InvocationExpression" || node.node_type == "TermExpression" {
        if let Some(steps) = decompose_chain(node) {
            if let Some(result) = match_qr_path(&steps) {
                let span = Span {
                    start: node.byte_start,
                    end: node.byte_end,
                };
                let kind = match result.kind {
                    MatchKind::AnswerRef(accessor) => AnnotationKind::AnswerReference {
                        link_ids: result.link_ids,
                        accessor,
                    },
                    MatchKind::ItemRef => AnnotationKind::ItemReference {
                        link_ids: result.link_ids,
                    },
                };
                out.push(Annotation { span, kind });
                return; // Don't recurse into children of a matched node
            }
        }
    }

    // Recurse into children
    for child in &node.children {
        find_answer_refs(child, out);
    }
}

/// Check if a node's byte range overlaps with an annotation span.
fn overlaps_annotation(node: &AstNode, ann: &Annotation) -> bool {
    node.byte_start < ann.span.end && node.byte_end > ann.span.start
}

/// Find the last linkId from an answer reference annotation.
fn last_link_id(ann: &Annotation) -> Option<&str> {
    match &ann.kind {
        AnnotationKind::AnswerReference { link_ids, .. } => link_ids.last().map(|s| s.as_str()),
        _ => None,
    }
}

/// Try to extract a `%factory.Coding('system', 'code')` invocation from a node.
fn try_extract_factory_coding(node: &AstNode) -> Option<(String, Option<String>, Span)> {
    // node should be an InvocationExpression
    if node.node_type != "InvocationExpression" {
        return None;
    }

    let receiver = node.children.first()?;
    let member = node.children.get(1)?;

    // receiver should be TermExpression -> ExternalConstantTerm -> ExternalConstant -> Identifier("factory")
    if receiver.node_type != "TermExpression" {
        return None;
    }
    let ext_term = receiver.children.first()?;
    if ext_term.node_type != "ExternalConstantTerm" {
        return None;
    }
    let ext_const = ext_term.children.first()?;
    if ext_const.node_type != "ExternalConstant" {
        return None;
    }
    let factory_ident = ext_const.children.first()?;
    if get_identifier_name(factory_ident)? != "factory" {
        return None;
    }

    // member should be FunctionInvocation -> Functn with name "Coding"
    if member.node_type != "FunctionInvocation" {
        return None;
    }
    let functn = member.children.first()?;
    if functn.node_type != "Functn" {
        return None;
    }
    let func_ident = functn.children.first()?;
    if get_identifier_name(func_ident)? != "Coding" {
        return None;
    }

    // ParamList with >= 2 children
    let param_list = functn.children.get(1)?;
    if param_list.node_type != "ParamList" || param_list.children.len() < 2 {
        return None;
    }

    let system = extract_string_value(param_list.children.first()?)?;
    let code = extract_string_value(param_list.children.get(1)?)?;

    Some((
        code,
        Some(system),
        Span {
            start: node.byte_start,
            end: node.byte_end,
        },
    ))
}

/// Try to extract a string literal value and span from a node by searching its subtree.
fn try_extract_string_literal(node: &AstNode) -> Option<(String, Span)> {
    if let Some(val) = extract_string_value(node) {
        return Some((
            val,
            Span {
                start: node.byte_start,
                end: node.byte_end,
            },
        ));
    }
    // Recurse into children
    for child in &node.children {
        if let Some(result) = try_extract_string_literal(child) {
            return Some(result);
        }
    }
    None
}

/// Try to extract a factory coding from a node by searching its subtree.
fn try_extract_factory_coding_recursive(node: &AstNode) -> Option<(String, Option<String>, Span)> {
    if let Some(result) = try_extract_factory_coding(node) {
        return Some(result);
    }
    for child in &node.children {
        if let Some(result) = try_extract_factory_coding_recursive(child) {
            return Some(result);
        }
    }
    None
}

/// Check if a node is fully contained within any answer ref annotation.
fn contained_in_answer_ref(node: &AstNode, answer_refs: &[Annotation]) -> bool {
    answer_refs.iter().any(|a| node.byte_start >= a.span.start && node.byte_end <= a.span.end)
}

/// Pass 2: Find coded values by scanning for equality/equivalence expressions that compare
/// an answer reference against a code literal or factory Coding.
fn find_coded_values(node: &AstNode, answer_refs: &[Annotation], out: &mut Vec<Annotation>) {
    // Skip nodes fully contained within an answer ref (e.g. the linkId='x' inside where())
    if contained_in_answer_ref(node, answer_refs) {
        return;
    }

    if node.node_type == "EqualityExpression" && node.children.len() == 2 {
        let left = &node.children[0];
        let right = &node.children[1];

        // Find which side overlaps an answer ref
        let (answer_ref, code_side) =
            if let Some(ann) = answer_refs.iter().find(|a| overlaps_annotation(left, a)) {
                (ann, right)
            } else if let Some(ann) = answer_refs.iter().find(|a| overlaps_annotation(right, a)) {
                (ann, left)
            } else {
                // No answer ref found, recurse into children
                for child in &node.children {
                    find_coded_values(child, answer_refs, out);
                }
                return;
            };

        let context_link_id = match last_link_id(answer_ref) {
            Some(id) => id.to_string(),
            None => {
                for child in &node.children {
                    find_coded_values(child, answer_refs, out);
                }
                return;
            }
        };

        // Try factory coding first, then string literal
        if let Some((code, system, span)) = try_extract_factory_coding_recursive(code_side) {
            out.push(Annotation {
                span,
                kind: AnnotationKind::CodedValue {
                    code,
                    system,
                    context_link_id,
                },
            });
            return;
        }

        if let Some((value, span)) = try_extract_string_literal(code_side) {
            out.push(Annotation {
                span,
                kind: AnnotationKind::CodedValue {
                    code: value,
                    system: None,
                    context_link_id,
                },
            });
            return;
        }
    }

    // Recurse into children
    for child in &node.children {
        find_coded_values(child, answer_refs, out);
    }
}

// ── Public API ──────────────────────────────────────────────────────────

/// Annotate a FHIRPath expression string, extracting answer references,
/// item references, and coded values.
pub fn annotate_expression(expr: &str) -> Result<Vec<Annotation>, crate::ParseError> {
    let tokens = crate::lexer::tokenize(expr).map_err(crate::ParseError)?;
    let mut p = crate::parser::Parser::new(&tokens);
    let root = p.parse_entire_expression().map_err(crate::ParseError)?;

    let mut answer_refs = Vec::new();
    find_answer_refs(&root, &mut answer_refs);

    let mut coded_values = Vec::new();
    find_coded_values(&root, &answer_refs, &mut coded_values);

    let mut all: Vec<Annotation> = answer_refs.into_iter().chain(coded_values).collect();
    all.sort_by_key(|a| a.span.start);
    Ok(all)
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::{AnnotationKind, ValueAccessor};

    #[test]
    fn test_answer_reference_code() {
        let r = annotate_expression("item.where(linkId='sedatie').answer.value.code").unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::AnswerReference { link_ids, accessor }
            if link_ids == &["sedatie"] && *accessor == ValueAccessor::Code));
    }

    #[test]
    fn test_answer_reference_display() {
        let r = annotate_expression("item.where(linkId='x').answer.value.display").unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::AnswerReference { accessor, .. }
            if *accessor == ValueAccessor::Display));
    }

    #[test]
    fn test_answer_reference_bare_value() {
        let r = annotate_expression("item.where(linkId='x').answer.value").unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::AnswerReference { accessor, .. }
            if *accessor == ValueAccessor::Value));
    }

    #[test]
    fn test_nested_item_navigation() {
        let r = annotate_expression(
            "item.where(linkId='a').item.where(linkId='b').answer.value.code",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::AnswerReference { link_ids, accessor }
            if link_ids == &["a", "b"] && *accessor == ValueAccessor::Code));
    }

    #[test]
    fn test_nested_answer_item_navigation() {
        let r = annotate_expression(
            "%context.item.where(linkId='verwijzer').answer.item.where(linkId='verwijzend-ziekenhuis').answer.value.display",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::AnswerReference { link_ids, accessor }
            if link_ids == &["verwijzer", "verwijzend-ziekenhuis"] && *accessor == ValueAccessor::Display));
    }

    #[test]
    fn test_item_reference() {
        let r = annotate_expression("item.where(linkId='group1')").unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::ItemReference { link_ids }
            if link_ids == &["group1"]));
    }

    #[test]
    fn test_string_literal_coded_value() {
        let r = annotate_expression("item.where(linkId='x').answer.value.code = 'yes'").unwrap();
        assert_eq!(r.len(), 2);
        assert!(r
            .iter()
            .any(|a| matches!(&a.kind, AnnotationKind::AnswerReference { .. })));
        assert!(r.iter().any(|a| matches!(&a.kind, AnnotationKind::CodedValue { code, system, context_link_id }
            if code == "yes" && system.is_none() && context_link_id == "x")));
    }

    #[test]
    fn test_factory_coding() {
        let r = annotate_expression(
            "item.where(linkId='x').answer.value ~ %factory.Coding('http://snomed.info/sct', '373067005')",
        )
        .unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|a| matches!(&a.kind, AnnotationKind::CodedValue { code, system, context_link_id }
            if code == "373067005" && system.as_deref() == Some("http://snomed.info/sct") && context_link_id == "x")));
    }

    #[test]
    fn test_external_constant_prefix() {
        let r =
            annotate_expression("%context.item.where(linkId='x').answer.value.code").unwrap();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0].kind, AnnotationKind::AnswerReference { link_ids, .. }
            if link_ids == &["x"]));
    }

    #[test]
    fn test_non_matching_expression() {
        let r = annotate_expression("Patient.name.given").unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_byte_offsets_cover_expression() {
        let expr = "item.where(linkId='x').answer.value.code";
        let r = annotate_expression(expr).unwrap();
        assert_eq!(r[0].span.start, 0);
        assert_eq!(r[0].span.end, expr.len());
    }
}
