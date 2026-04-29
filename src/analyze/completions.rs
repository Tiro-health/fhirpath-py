use crate::analyze::annotations::{decompose_chain, ChainStepKind};
use crate::analyze::questionnaire_index::QuestionnaireIndex;

// ── Public types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionItemKind {
    Value,
    Code,
    Display,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub insert_text: String,
    pub filter_text: String,
    pub sort_text: String,
    pub kind: CompletionItemKind,
    pub link_id: String,
    pub item_type: String,
    /// `true` if accepting this suggestion crosses a repeating boundary that
    /// the user's typed prefix has not already crossed — i.e. any item
    /// strictly between the context anchor and the target leaf (or the leaf
    /// itself) has `repeats: true`. Items already in the prefix, including
    /// the anchor itself, are excluded. UI hosts can use this to warn that
    /// the chain may collapse multiple instances into a single list.
    pub traverses_repeating: bool,
}

// ── Context resolution ─────────────────────────────────────────────────

enum ContextPosition {
    /// Top of the QuestionnaireResponse — the user typed just `%resource`
    /// (or `%context`) and wants to start drilling into items.
    Root,
    ItemFiltered(String),
    Answer(String),
}

fn resolve_context(expr: &str) -> Result<Option<ContextPosition>, crate::ParseError> {
    let tokens = crate::lexer::tokenize(expr).map_err(crate::ParseError)?;
    let mut p = crate::parser::Parser::new(&tokens);
    let root = p.parse_entire_expression().map_err(crate::ParseError)?;

    let steps = match decompose_chain(&root) {
        Some(s) => s,
        None => return Ok(None),
    };

    enum State {
        Start,
        /// Saw `%resource` / `%context` at the head of the chain with nothing
        /// after it yet. Equivalent to Start for navigation, but distinguished
        /// so a chain that ends here can emit root-level suggestions.
        AtRoot,
        Item,
        ItemFiltered(String),
        Answer(String),
    }

    let mut state = State::Start;

    for step in &steps {
        state = match (&state, &step.kind) {
            // Only %resource and %context are transparent QR-aliases — see
            // annotations.rs for the same restriction.
            (State::Start, ChainStepKind::External(name))
                if name == "resource" || name == "context" =>
            {
                State::AtRoot
            }
            (State::Start | State::AtRoot, ChainStepKind::Identifier(name)) if name == "item" => {
                State::Item
            }
            (
                State::Item,
                ChainStepKind::Function {
                    name,
                    link_id: Some(id),
                    ..
                },
            ) if name == "where" => State::ItemFiltered(id.clone()),
            (State::ItemFiltered(_), ChainStepKind::Identifier(name)) if name == "item" => {
                State::Item
            }
            (State::ItemFiltered(id), ChainStepKind::Identifier(name)) if name == "answer" => {
                State::Answer(id.clone())
            }
            (State::Answer(_), ChainStepKind::Identifier(name)) if name == "item" => State::Item,
            _ => return Ok(None),
        };
    }

    match state {
        State::AtRoot => Ok(Some(ContextPosition::Root)),
        State::ItemFiltered(id) => Ok(Some(ContextPosition::ItemFiltered(id))),
        State::Answer(id) => Ok(Some(ContextPosition::Answer(id))),
        _ => Ok(None),
    }
}

// ── Type helpers ───────────────────────────────────────────────────────

fn is_coding_type(item_type: &str) -> bool {
    matches!(item_type, "choice" | "open-choice" | "coding")
}

fn has_value(item_type: &str) -> bool {
    matches!(
        item_type,
        "choice"
            | "open-choice"
            | "coding"
            | "boolean"
            | "decimal"
            | "integer"
            | "string"
            | "text"
            | "url"
            | "date"
            | "dateTime"
            | "time"
            | "reference"
            | "quantity"
    )
}

// ── Completion generation ──────────────────────────────────────────────

fn emit_value_completions(
    label: &str,
    link_id: &str,
    item_type: &str,
    value_prefix: &str,
    detail: Option<String>,
    traverses_repeating: bool,
    counter: &mut usize,
    out: &mut Vec<CompletionItem>,
) {
    if !has_value(item_type) {
        return;
    }

    let filter_text = format!("{label} {link_id}");

    out.push(CompletionItem {
        label: label.to_string(),
        detail: detail.clone(),
        insert_text: value_prefix.to_string(),
        filter_text: filter_text.clone(),
        sort_text: format!("{:04}", *counter),
        kind: CompletionItemKind::Value,
        link_id: link_id.to_string(),
        item_type: item_type.to_string(),
        traverses_repeating,
    });
    *counter += 1;

    if is_coding_type(item_type) {
        out.push(CompletionItem {
            label: label.to_string(),
            detail: detail.clone(),
            insert_text: format!("{value_prefix}.code"),
            filter_text: filter_text.clone(),
            sort_text: format!("{:04}", *counter),
            kind: CompletionItemKind::Code,
            link_id: link_id.to_string(),
            item_type: item_type.to_string(),
            traverses_repeating,
        });
        *counter += 1;

        out.push(CompletionItem {
            label: label.to_string(),
            detail,
            insert_text: format!("{value_prefix}.display"),
            filter_text,
            sort_text: format!("{:04}", *counter),
            kind: CompletionItemKind::Display,
            link_id: link_id.to_string(),
            item_type: item_type.to_string(),
            traverses_repeating,
        });
        *counter += 1;
    }
}

/// `anchor` is the deepest linkId resolved from `context_expr` (the user's
/// typed prefix). Items in the prefix — including the anchor itself — are
/// excluded from the walk: only repeating boundaries introduced by the
/// suggestion count. `None` (Root context) preserves the original
/// full-ancestry behavior.
fn chain_traverses_repeating(
    index: &QuestionnaireIndex,
    link_id: &str,
    anchor: Option<&str>,
) -> bool {
    if anchor == Some(link_id) {
        return false;
    }
    let leaf_repeats = index.get(link_id).map(|i| i.repeats).unwrap_or(false);
    let ancestor_repeats = match anchor {
        Some(a) => index.has_repeating_ancestor_until(link_id, a),
        None => index.has_repeating_ancestor(link_id),
    };
    leaf_repeats || ancestor_repeats
}

fn emit_subtree(
    index: &QuestionnaireIndex,
    link_id: &str,
    prefix: &str,
    breadcrumb_parts: &[&str],
    anchor: Option<&str>,
    counter: &mut usize,
    out: &mut Vec<CompletionItem>,
) {
    let Some(info) = index.get(link_id) else {
        return;
    };

    if info.item_type == "display" {
        return;
    }

    let detail = if breadcrumb_parts.is_empty() {
        None
    } else {
        Some(breadcrumb_parts.join(" > "))
    };

    if info.item_type != "group" {
        let value_prefix = format!("{prefix}.answer.value");
        emit_value_completions(
            &info.text,
            link_id,
            &info.item_type,
            &value_prefix,
            detail,
            chain_traverses_repeating(index, link_id, anchor),
            counter,
            out,
        );
    }

    let child_nav_prefix = if info.item_type == "group" {
        format!("{prefix}.item")
    } else {
        format!("{prefix}.answer.item")
    };

    let mut child_breadcrumbs: Vec<&str> = breadcrumb_parts.to_vec();
    child_breadcrumbs.push(&info.text);

    for child_id in &info.children {
        let child_prefix = format!("{child_nav_prefix}.where(linkId='{child_id}')");
        emit_subtree(
            index,
            child_id,
            &child_prefix,
            &child_breadcrumbs,
            anchor,
            counter,
            out,
        );
    }
}

// ── Public API ─────────────────────────────────────────────────────────

pub fn generate_completions(
    index: &QuestionnaireIndex,
    context_expr: &str,
) -> Result<Vec<CompletionItem>, crate::ParseError> {
    let position = match resolve_context(context_expr)? {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let mut out = Vec::new();
    let mut counter: usize = 0;

    match position {
        ContextPosition::Root => {
            for child_id in index.roots() {
                let prefix = format!("item.where(linkId='{child_id}')");
                emit_subtree(index, child_id, &prefix, &[], None, &mut counter, &mut out);
            }
        }
        ContextPosition::ItemFiltered(ref link_id) => {
            let Some(info) = index.get(link_id) else {
                return Ok(Vec::new());
            };

            let anchor = Some(link_id.as_str());

            if info.item_type == "group" {
                for child_id in &info.children {
                    let prefix = format!("item.where(linkId='{child_id}')");
                    emit_subtree(
                        index,
                        child_id,
                        &prefix,
                        &[],
                        anchor,
                        &mut counter,
                        &mut out,
                    );
                }
            } else {
                emit_value_completions(
                    &info.text,
                    link_id,
                    &info.item_type,
                    "answer.value",
                    None,
                    chain_traverses_repeating(index, link_id, anchor),
                    &mut counter,
                    &mut out,
                );

                let breadcrumbs = vec![info.text.as_str()];
                for child_id in &info.children {
                    let prefix = format!("answer.item.where(linkId='{child_id}')");
                    emit_subtree(
                        index,
                        child_id,
                        &prefix,
                        &breadcrumbs,
                        anchor,
                        &mut counter,
                        &mut out,
                    );
                }
                // breadcrumbs only needed during the loop above
                drop(breadcrumbs);
            }
        }
        ContextPosition::Answer(ref link_id) => {
            let Some(info) = index.get(link_id) else {
                return Ok(Vec::new());
            };

            let anchor = Some(link_id.as_str());

            emit_value_completions(
                &info.text,
                link_id,
                &info.item_type,
                "value",
                None,
                chain_traverses_repeating(index, link_id, anchor),
                &mut counter,
                &mut out,
            );

            let breadcrumbs = vec![info.text.as_str()];
            for child_id in &info.children {
                let prefix = format!("item.where(linkId='{child_id}')");
                emit_subtree(
                    index,
                    child_id,
                    &prefix,
                    &breadcrumbs,
                    anchor,
                    &mut counter,
                    &mut out,
                );
            }
        }
    }

    Ok(out)
}

// ── Tests ──────────────────────────────────────────────────────────────

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
                    "text": "Group One",
                    "type": "group",
                    "item": [
                        { "linkId": "choice1", "text": "Pick one", "type": "choice" },
                        { "linkId": "bool1", "text": "Yes or no", "type": "boolean" },
                        {
                            "linkId": "subgroup",
                            "text": "Sub Group",
                            "type": "group",
                            "item": [
                                { "linkId": "deep-string", "text": "Deep String", "type": "string" }
                            ]
                        },
                        { "linkId": "display1", "text": "Info text", "type": "display" }
                    ]
                },
                {
                    "linkId": "coding-with-children",
                    "text": "Resectie",
                    "type": "coding",
                    "item": [
                        { "linkId": "child-coding", "text": "Biopten", "type": "coding" },
                        { "linkId": "child-bool", "text": "Nabloeding", "type": "boolean" }
                    ]
                }
            ]
        })
    }

    #[test]
    fn test_group_context_with_mixed_children() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();

        // choice1 → 3 completions (value, code, display)
        assert!(texts.contains(&"item.where(linkId='choice1').answer.value"));
        assert!(texts.contains(&"item.where(linkId='choice1').answer.value.code"));
        assert!(texts.contains(&"item.where(linkId='choice1').answer.value.display"));

        // bool1 → 1 completion (value only)
        assert!(texts.contains(&"item.where(linkId='bool1').answer.value"));
        assert!(!texts.contains(&"item.where(linkId='bool1').answer.value.code"));

        // subgroup → no own value, but recurses to deep-string
        assert!(texts.contains(
            &"item.where(linkId='subgroup').item.where(linkId='deep-string').answer.value"
        ));

        // display1 → skipped entirely
        assert!(!texts.iter().any(|t| t.contains("display1")));
    }

    #[test]
    fn test_non_group_context_own_answer_and_children() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items =
            generate_completions(&idx, "item.where(linkId='coding-with-children')").unwrap();

        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();

        // Own answer
        assert!(texts.contains(&"answer.value"));
        assert!(texts.contains(&"answer.value.code"));
        assert!(texts.contains(&"answer.value.display"));

        // Children via answer.item
        assert!(texts
            .contains(&"answer.item.where(linkId='child-coding').answer.value"));
        assert!(texts
            .contains(&"answer.item.where(linkId='child-coding').answer.value.code"));
        assert!(texts
            .contains(&"answer.item.where(linkId='child-bool').answer.value"));
        assert!(!texts
            .contains(&"answer.item.where(linkId='child-bool').answer.value.code"));
    }

    #[test]
    fn test_answer_context() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(
            &idx,
            "item.where(linkId='coding-with-children').answer",
        )
        .unwrap();

        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();

        // Value without "answer." prefix
        assert!(texts.contains(&"value"));
        assert!(texts.contains(&"value.code"));
        assert!(texts.contains(&"value.display"));

        // Children via item (not answer.item)
        assert!(texts.contains(&"item.where(linkId='child-coding').answer.value"));
        assert!(texts.contains(&"item.where(linkId='child-bool').answer.value"));
    }

    #[test]
    fn test_nested_groups() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();

        // Group → item chain (no answer.item for groups)
        assert!(texts.contains(
            &"item.where(linkId='subgroup').item.where(linkId='deep-string').answer.value"
        ));
    }

    #[test]
    fn test_deep_non_group_nesting() {
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [{
                "linkId": "parent",
                "text": "Parent",
                "type": "coding",
                "item": [{
                    "linkId": "child",
                    "text": "Child",
                    "type": "coding",
                    "item": [{
                        "linkId": "grandchild",
                        "text": "Grandchild",
                        "type": "string"
                    }]
                }]
            }]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items = generate_completions(&idx, "item.where(linkId='parent')").unwrap();

        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();

        // Non-group → answer.item chains
        assert!(texts.contains(
            &"answer.item.where(linkId='child').answer.item.where(linkId='grandchild').answer.value"
        ));
    }

    #[test]
    fn test_breadcrumb_detail() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items =
            generate_completions(&idx, "item.where(linkId='coding-with-children')").unwrap();

        // Own value: no breadcrumb
        let own = items.iter().find(|c| c.insert_text == "answer.value").unwrap();
        assert_eq!(own.detail, None);

        // Direct child: breadcrumb shows parent text
        let child = items
            .iter()
            .find(|c| c.insert_text == "answer.item.where(linkId='child-coding').answer.value")
            .unwrap();
        assert_eq!(child.detail.as_deref(), Some("Resectie"));
    }

    #[test]
    fn test_nested_breadcrumb() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let deep = items
            .iter()
            .find(|c| {
                c.insert_text
                    == "item.where(linkId='subgroup').item.where(linkId='deep-string').answer.value"
            })
            .unwrap();
        assert_eq!(deep.detail.as_deref(), Some("Sub Group"));
    }

    #[test]
    fn test_unresolvable_expression_returns_empty() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "%context.where(true)").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_external_constant_prefix() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items =
            generate_completions(&idx, "%resource.item.where(linkId='group1')").unwrap();

        assert!(!items.is_empty());
        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
        assert!(texts.contains(&"item.where(linkId='choice1').answer.value"));
    }

    #[test]
    fn test_resource_alone_emits_top_level_items() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "%resource").unwrap();

        assert!(!items.is_empty());
        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
        // Top-level group: descend through children (group itself has no value).
        assert!(texts.contains(&"item.where(linkId='group1').item.where(linkId='choice1').answer.value"));
        // Top-level coding item: own answer.value plus child completions.
        assert!(texts.contains(&"item.where(linkId='coding-with-children').answer.value"));
        assert!(texts.contains(&"item.where(linkId='coding-with-children').answer.value.code"));
    }

    #[test]
    fn test_context_alone_emits_top_level_items() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "%context").unwrap();
        assert!(!items.is_empty());
        let texts: Vec<&str> = items.iter().map(|c| c.insert_text.as_str()).collect();
        assert!(texts.contains(&"item.where(linkId='coding-with-children').answer.value"));
    }

    #[test]
    fn test_questionnaire_external_returns_empty() {
        // %questionnaire points at the Questionnaire structure, not the QR —
        // suggesting QR-shaped completions there would be misleading.
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items =
            generate_completions(&idx, "%questionnaire.item.where(linkId='group1')").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_unknown_external_alone_returns_empty() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "%questionnaire").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_unknown_link_id_returns_empty() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='nonexistent')").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_sort_text_is_depth_first() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items =
            generate_completions(&idx, "item.where(linkId='coding-with-children')").unwrap();

        let sort_texts: Vec<&str> = items.iter().map(|c| c.sort_text.as_str()).collect();
        let mut sorted = sort_texts.clone();
        sorted.sort();
        assert_eq!(sort_texts, sorted);
    }

    #[test]
    fn test_filter_text_contains_label_and_link_id() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let choice_item = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='choice1').answer.value")
            .unwrap();
        assert!(choice_item.filter_text.contains("Pick one"));
        assert!(choice_item.filter_text.contains("choice1"));
    }

    #[test]
    fn test_completion_kinds() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let value = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='choice1').answer.value")
            .unwrap();
        assert_eq!(value.kind, CompletionItemKind::Value);

        let code = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='choice1').answer.value.code")
            .unwrap();
        assert_eq!(code.kind, CompletionItemKind::Code);

        let display = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='choice1').answer.value.display")
            .unwrap();
        assert_eq!(display.kind, CompletionItemKind::Display);
    }

    #[test]
    fn test_link_id_and_item_type_match_target_item() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let bool_item = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='bool1').answer.value")
            .unwrap();
        assert_eq!(bool_item.link_id, "bool1");
        assert_eq!(bool_item.item_type, "boolean");
    }

    #[test]
    fn test_coding_variants_share_link_id_and_item_type() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let variants: Vec<&CompletionItem> = items
            .iter()
            .filter(|c| c.link_id == "choice1")
            .collect();
        assert_eq!(variants.len(), 3);
        for v in &variants {
            assert_eq!(v.link_id, "choice1");
            assert_eq!(v.item_type, "choice");
        }
    }

    #[test]
    fn test_traverses_repeating_false_for_non_repeating_chain() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        for item in &items {
            assert!(
                !item.traverses_repeating,
                "expected traverses_repeating=false for {} (link_id={})",
                item.insert_text,
                item.link_id
            );
        }
    }

    #[test]
    fn test_traverses_repeating_false_for_own_value_at_repeating_anchor() {
        // Anchor is the repeating item itself: typing
        // `item.where(linkId='answers')` already commits to its multiplicity,
        // so suggesting its own value adds no new repeating boundary.
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [{
                "linkId": "answers",
                "text": "Multi-answer",
                "type": "string",
                "repeats": true
            }]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items = generate_completions(&idx, "item.where(linkId='answers')").unwrap();

        let value = items
            .iter()
            .find(|c| c.insert_text == "answer.value")
            .unwrap();
        assert!(!value.traverses_repeating);
    }

    #[test]
    fn test_traverses_repeating_true_for_repeating_leaf_below_anchor() {
        // Anchor `parent` does not repeat; suggestion descends into a
        // repeating child. The boundary IS introduced by the suggestion.
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [{
                "linkId": "parent",
                "text": "Parent",
                "type": "group",
                "item": [{
                    "linkId": "rep_child",
                    "text": "Rep Child",
                    "type": "string",
                    "repeats": true
                }]
            }]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items = generate_completions(&idx, "item.where(linkId='parent')").unwrap();

        let leaf = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='rep_child').answer.value")
            .unwrap();
        assert!(
            leaf.traverses_repeating,
            "repeating leaf below a non-repeating anchor should be flagged"
        );
    }

    #[test]
    fn test_traverses_repeating_true_for_repeating_intermediate_below_anchor() {
        // Anchor is the outer non-repeating group; the path from anchor down
        // to the leaf crosses a repeating intermediate group. That's a
        // boundary the suggestion introduces.
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [{
                "linkId": "outer",
                "text": "Outer",
                "type": "group",
                "item": [{
                    "linkId": "rep_group",
                    "text": "Rep Group",
                    "type": "group",
                    "repeats": true,
                    "item": [{
                        "linkId": "leaf",
                        "text": "Leaf",
                        "type": "string"
                    }]
                }]
            }]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items = generate_completions(&idx, "item.where(linkId='outer')").unwrap();

        let leaf = items
            .iter()
            .find(|c| {
                c.insert_text
                    == "item.where(linkId='rep_group').item.where(linkId='leaf').answer.value"
            })
            .unwrap();
        assert!(
            leaf.traverses_repeating,
            "leaf below a repeating intermediate (between anchor and leaf) should be flagged"
        );
    }

    #[test]
    fn test_traverses_repeating_anchor_itself_excluded() {
        // The repeating boundary on `repeating-group` is in the prefix the
        // user already typed; descending into a non-repeating child adds no
        // new boundary.
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [{
                "linkId": "repeating-group",
                "text": "Repeating Group",
                "type": "group",
                "repeats": true,
                "item": [{
                    "linkId": "leaf",
                    "text": "Leaf",
                    "type": "string"
                }]
            }]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items = generate_completions(&idx, "item.where(linkId='repeating-group')").unwrap();

        let leaf = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='leaf').answer.value")
            .unwrap();
        assert!(
            !leaf.traverses_repeating,
            "anchor's own repeats was committed by the prefix; child should not be flagged"
        );
    }

    #[test]
    fn test_traverses_repeating_mixed_within_one_questionnaire() {
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [
                {
                    "linkId": "stable",
                    "text": "Stable",
                    "type": "string"
                },
                {
                    "linkId": "rep",
                    "text": "Rep",
                    "type": "string",
                    "repeats": true
                }
            ]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items = generate_completions(&idx, "%resource").unwrap();

        let stable = items
            .iter()
            .find(|c| c.link_id == "stable")
            .unwrap();
        let rep = items.iter().find(|c| c.link_id == "rep").unwrap();
        assert!(!stable.traverses_repeating);
        assert!(rep.traverses_repeating);
    }

    #[test]
    fn test_traverses_repeating_anchor_excluded_in_answer_context() {
        // `parent` repeats, but it's already in the typed prefix. Neither the
        // own value nor the descendant introduces a new repeating boundary.
        let q = json!({
            "resourceType": "Questionnaire",
            "item": [{
                "linkId": "parent",
                "text": "Parent",
                "type": "coding",
                "repeats": true,
                "item": [{
                    "linkId": "child",
                    "text": "Child",
                    "type": "string"
                }]
            }]
        });
        let idx = QuestionnaireIndex::build(&q);
        let items =
            generate_completions(&idx, "item.where(linkId='parent').answer").unwrap();

        let own = items.iter().find(|c| c.insert_text == "value").unwrap();
        assert!(!own.traverses_repeating);

        let child = items
            .iter()
            .find(|c| c.insert_text == "item.where(linkId='child').answer.value")
            .unwrap();
        assert!(!child.traverses_repeating);
    }

    #[test]
    fn test_nested_descent_uses_leaf_link_id() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let items = generate_completions(&idx, "item.where(linkId='group1')").unwrap();

        let deep = items
            .iter()
            .find(|c| {
                c.insert_text
                    == "item.where(linkId='subgroup').item.where(linkId='deep-string').answer.value"
            })
            .unwrap();
        assert_eq!(deep.link_id, "deep-string");
        assert_eq!(deep.item_type, "string");
    }
}
