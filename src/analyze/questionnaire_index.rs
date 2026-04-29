use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct QuestionnaireItemInfo {
    pub link_id: String,
    pub text: String,
    pub item_type: String,
    /// `true` if the item is declared `repeats: true` in the Questionnaire.
    /// Drives cardinality inference for answer-leaf chains.
    pub repeats: bool,
    pub answer_options: HashMap<(String, String), String>, // (system, code) -> display
    pub parent_link_id: Option<String>,
    pub children: Vec<String>,
}

#[derive(Debug, Default)]
pub struct QuestionnaireIndex {
    items: HashMap<String, QuestionnaireItemInfo>,
    /// Top-level item linkIds in document order. Iterating the `items`
    /// HashMap doesn't preserve order, so we keep a parallel list to drive
    /// deterministic suggestion output.
    root_link_ids: Vec<String>,
}

impl QuestionnaireIndex {
    /// Build index from a FHIR Questionnaire JSON value.
    /// Expects `questionnaire["item"]` to be an array of item objects.
    pub fn build(questionnaire: &serde_json::Value) -> Self {
        let mut index = Self::default();
        if let Some(items) = questionnaire.get("item").and_then(|v| v.as_array()) {
            index.walk(items, None);
        }
        index
    }

    /// Top-level item linkIds in document order.
    pub fn roots(&self) -> &[String] {
        &self.root_link_ids
    }

    fn walk(&mut self, items: &[serde_json::Value], parent_link_id: Option<&str>) {
        for item in items {
            let Some(link_id) = item.get("linkId").and_then(|v| v.as_str()) else {
                continue;
            };
            let link_id = link_id.to_string();

            let mut answer_options = HashMap::new();
            if let Some(opts) = item.get("answerOption").and_then(|v| v.as_array()) {
                for opt in opts {
                    if let (Some(system), Some(code), Some(display)) = (
                        opt.pointer("/valueCoding/system").and_then(|v| v.as_str()),
                        opt.pointer("/valueCoding/code").and_then(|v| v.as_str()),
                        opt.pointer("/valueCoding/display").and_then(|v| v.as_str()),
                    ) {
                        answer_options.insert(
                            (system.to_string(), code.to_string()),
                            display.to_string(),
                        );
                    }
                }
            }

            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or(&link_id)
                .to_string();
            let item_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("group")
                .to_string();
            let repeats = item
                .get("repeats")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            self.items.insert(
                link_id.clone(),
                QuestionnaireItemInfo {
                    link_id: link_id.clone(),
                    text,
                    item_type,
                    repeats,
                    answer_options,
                    parent_link_id: parent_link_id.map(|s| s.to_string()),
                    children: Vec::new(),
                },
            );

            // Update parent's children vec, or record this as a root.
            if let Some(pid) = parent_link_id {
                if let Some(parent) = self.items.get_mut(pid) {
                    parent.children.push(link_id.clone());
                }
            } else {
                self.root_link_ids.push(link_id.clone());
            }

            if let Some(sub_items) = item.get("item").and_then(|v| v.as_array()) {
                self.walk(sub_items, Some(&link_id));
            }
        }
    }

    pub fn get(&self, link_id: &str) -> Option<&QuestionnaireItemInfo> {
        self.items.get(link_id)
    }

    pub fn contains(&self, link_id: &str) -> bool {
        self.items.contains_key(link_id)
    }

    pub fn resolve_item_text(&self, link_id: &str) -> Option<&str> {
        self.items.get(link_id).map(|i| i.text.as_str())
    }

    pub fn resolve_code_display(&self, link_id: &str, system: &str, code: &str) -> Option<&str> {
        self.items
            .get(link_id)?
            .answer_options
            .get(&(system.to_string(), code.to_string()))
            .map(|s| s.as_str())
    }

    pub fn resolve_item_type(&self, link_id: &str) -> Option<&str> {
        self.items.get(link_id).map(|i| i.item_type.as_str())
    }

    /// Whether the item allows multiple answers (`repeats: true`).
    /// Returns `None` when the linkId is unknown.
    pub fn resolve_item_repeats(&self, link_id: &str) -> Option<bool> {
        self.items.get(link_id).map(|i| i.repeats)
    }

    /// Whether *any* item on the path from `link_id` up to the root has
    /// `repeats: true`. Used to detect answer chains that flatten across
    /// repeating-group instances.
    pub fn has_repeating_ancestor(&self, link_id: &str) -> bool {
        let mut current = match self.items.get(link_id) {
            Some(i) => i.parent_link_id.as_deref(),
            None => return false,
        };
        for _ in 0..100 {
            let Some(parent_id) = current else {
                return false;
            };
            let Some(parent) = self.items.get(parent_id) else {
                return false;
            };
            if parent.repeats {
                return true;
            }
            current = parent.parent_link_id.as_deref();
        }
        false
    }

    /// Check if `descendant` is a child/grandchild/... of `ancestor`.
    pub fn is_descendant(&self, ancestor: &str, descendant: &str) -> bool {
        let mut current = descendant;
        for _ in 0..100 {
            // depth limit for safety
            match self
                .items
                .get(current)
                .and_then(|i| i.parent_link_id.as_deref())
            {
                Some(p) if p == ancestor => return true,
                Some(p) => current = p,
                None => return false,
            }
        }
        false
    }
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
                            "linkId": "choice1",
                            "text": "Pick one",
                            "type": "choice",
                            "answerOption": [
                                { "valueCoding": { "system": "http://example.com", "code": "A", "display": "Alpha" } },
                                { "valueCoding": { "system": "http://example.com", "code": "B", "display": "Beta" } }
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
                    "linkId": "string1",
                    "text": "Free text",
                    "type": "string"
                }
            ]
        })
    }

    #[test]
    fn test_resolve_text() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        assert_eq!(idx.resolve_item_text("group1"), Some("Group One"));
        assert_eq!(idx.resolve_item_text("choice1"), Some("Pick one"));
        assert_eq!(idx.resolve_item_text("nonexistent"), None);
    }

    #[test]
    fn test_resolve_type() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        assert_eq!(idx.resolve_item_type("group1"), Some("group"));
        assert_eq!(idx.resolve_item_type("choice1"), Some("choice"));
        assert_eq!(idx.resolve_item_type("bool1"), Some("boolean"));
    }

    #[test]
    fn test_resolve_code_display() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        assert_eq!(idx.resolve_code_display("choice1", "http://example.com", "A"), Some("Alpha"));
        assert_eq!(idx.resolve_code_display("choice1", "http://example.com", "B"), Some("Beta"));
        assert_eq!(idx.resolve_code_display("choice1", "http://example.com", "C"), None);
        assert_eq!(idx.resolve_code_display("choice1", "http://other.com", "A"), None);
        assert_eq!(idx.resolve_code_display("bool1", "http://example.com", "A"), None);
    }

    #[test]
    fn test_contains() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        assert!(idx.contains("group1"));
        assert!(!idx.contains("ghost"));
    }

    #[test]
    fn test_parent_child() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        let group = idx.get("group1").unwrap();
        assert_eq!(group.parent_link_id, None);
        assert!(group.children.contains(&"choice1".to_string()));
        assert!(group.children.contains(&"bool1".to_string()));

        let choice = idx.get("choice1").unwrap();
        assert_eq!(choice.parent_link_id.as_deref(), Some("group1"));
    }

    #[test]
    fn test_is_descendant() {
        let idx = QuestionnaireIndex::build(&sample_questionnaire());
        assert!(idx.is_descendant("group1", "choice1"));
        assert!(idx.is_descendant("group1", "bool1"));
        assert!(!idx.is_descendant("string1", "choice1"));
        assert!(!idx.is_descendant("choice1", "group1"));
        assert!(!idx.is_descendant("group1", "group1"));
    }

    #[test]
    fn test_text_fallback_to_link_id() {
        let q = json!({"resourceType": "Questionnaire", "item": [{"linkId": "no-text", "type": "string"}]});
        let idx = QuestionnaireIndex::build(&q);
        assert_eq!(idx.resolve_item_text("no-text"), Some("no-text"));
    }

    #[test]
    fn test_type_fallback_to_group() {
        let q = json!({"resourceType": "Questionnaire", "item": [{"linkId": "no-type"}]});
        let idx = QuestionnaireIndex::build(&q);
        assert_eq!(idx.resolve_item_type("no-type"), Some("group"));
    }
}
