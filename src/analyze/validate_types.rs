use crate::analyze::{
    AnnotationKind, Diagnostic, DiagnosticCode, Severity, ValueAccessor,
};
use crate::analyze::annotations::annotate_expression;
use crate::analyze::questionnaire_index::QuestionnaireIndex;

/// FHIR value type categories based on Questionnaire item type.
enum FhirValueType {
    /// choice, open-choice -> Coding: .code, .display are valid accessors
    Coding,
    /// boolean, decimal, integer, string, text, url -> only bare .value
    Primitive,
    /// date, dateTime, time -> only bare .value
    Temporal,
    /// reference -> only bare .value (for now)
    Reference,
    /// quantity -> .value, .unit, .code, .system
    Quantity,
}

fn value_type_for_item(item_type: &str) -> Option<FhirValueType> {
    match item_type {
        "choice" | "open-choice" => Some(FhirValueType::Coding),
        "boolean" | "decimal" | "integer" | "string" | "text" | "url" => {
            Some(FhirValueType::Primitive)
        }
        "date" | "dateTime" | "time" => Some(FhirValueType::Temporal),
        "reference" => Some(FhirValueType::Reference),
        "quantity" => Some(FhirValueType::Quantity),
        // group, display, attachment, etc. -- no value type validation applicable
        _ => None,
    }
}

fn is_valid_accessor(value_type: &FhirValueType, accessor: &ValueAccessor) -> bool {
    match (value_type, accessor) {
        // Coding: .code and .display are the main accessors; bare .value is technically valid
        (FhirValueType::Coding, _) => true,
        // Primitive/Temporal/Reference: only bare .value
        (FhirValueType::Primitive, ValueAccessor::Value) => true,
        (FhirValueType::Temporal, ValueAccessor::Value) => true,
        (FhirValueType::Reference, ValueAccessor::Value) => true,
        // Quantity: bare .value is valid (for the numeric value)
        (FhirValueType::Quantity, ValueAccessor::Value) => true,
        // Everything else is invalid
        _ => false,
    }
}

/// Validate that answer value accessors (.code, .display, bare .value) are
/// appropriate for the Questionnaire item type.
pub fn validate_value_types(
    expr: &str,
    index: &QuestionnaireIndex,
) -> Result<Vec<Diagnostic>, crate::ParseError> {
    let annotations = annotate_expression(expr)?;
    let mut diagnostics = Vec::new();

    for ann in &annotations {
        if let AnnotationKind::AnswerReference { link_ids, accessor } = &ann.kind {
            let Some(last_link_id) = link_ids.last() else {
                continue;
            };
            let Some(item_type_str) = index.resolve_item_type(last_link_id) else {
                continue; // unknown linkId -- that's validate_link_ids' job
            };
            let Some(value_type) = value_type_for_item(item_type_str) else {
                continue; // group, display, etc. -- skip
            };

            if !is_valid_accessor(&value_type, accessor) {
                let accessor_name = match accessor {
                    ValueAccessor::Code => ".code",
                    ValueAccessor::Display => ".display",
                    ValueAccessor::Value => ".value",
                };
                diagnostics.push(Diagnostic {
                    span: ann.span.clone(),
                    severity: Severity::Error,
                    code: DiagnosticCode::InvalidAccessorForType,
                    message: format!(
                        "Accessor '{}' is not valid for item type '{}'",
                        accessor_name, item_type_str
                    ),
                });
            }

            // Warning: bare .value on a Coding item -- likely missing .code or .display
            if matches!(value_type, FhirValueType::Coding)
                && matches!(accessor, ValueAccessor::Value)
            {
                diagnostics.push(Diagnostic {
                    span: ann.span.clone(),
                    severity: Severity::Warning,
                    code: DiagnosticCode::MissingAccessorForCoding,
                    message: format!(
                        "Item '{}' is type '{}' -- consider using .code or .display instead of bare .value",
                        last_link_id, item_type_str
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

    fn questionnaire_with_types() -> serde_json::Value {
        json!({
            "resourceType": "Questionnaire",
            "item": [
                { "linkId": "choice1", "text": "Pick", "type": "choice" },
                { "linkId": "bool1", "text": "Yes/No", "type": "boolean" },
                { "linkId": "decimal1", "text": "Amount", "type": "decimal" },
                { "linkId": "string1", "text": "Name", "type": "string" },
                { "linkId": "date1", "text": "Date", "type": "date" },
                { "linkId": "quantity1", "text": "Weight", "type": "quantity" },
                { "linkId": "ref1", "text": "Reference", "type": "reference" },
                { "linkId": "open1", "text": "Open", "type": "open-choice" }
            ]
        })
    }

    #[test]
    fn test_code_on_boolean_is_error() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='bool1').answer.value.code",
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::InvalidAccessorForType);
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_display_on_decimal_is_error() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='decimal1').answer.value.display",
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::InvalidAccessorForType);
    }

    #[test]
    fn test_code_on_date_is_error() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='date1').answer.value.code",
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::InvalidAccessorForType);
    }

    #[test]
    fn test_bare_value_on_choice_is_warning() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='choice1').answer.value",
            &idx,
        ).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::MissingAccessorForCoding);
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn test_code_on_choice_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='choice1').answer.value.code",
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_display_on_choice_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='choice1').answer.value.display",
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_bare_value_on_boolean_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='bool1').answer.value",
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_bare_value_on_string_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='string1').answer.value",
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_bare_value_on_quantity_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='quantity1').answer.value",
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_code_on_open_choice_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='open1').answer.value.code",
            &idx,
        ).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unknown_link_id_is_skipped() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types(
            "item.where(linkId='unknown').answer.value.code",
            &idx,
        ).unwrap();
        // Unknown linkId is not this validator's concern
        assert!(diags.is_empty());
    }

    #[test]
    fn test_non_matching_expression_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_types());
        let diags = validate_value_types("Patient.name.given", &idx).unwrap();
        assert!(diags.is_empty());
    }
}
