/// Expression analysis: annotation extraction for FHIRPath expressions.

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValueAccessor {
    /// `.answer.value` (bare)
    Value,
    /// `.answer.value.code`
    Code,
    /// `.answer.value.display`
    Display,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub span: Span,
    pub kind: AnnotationKind,
}

pub mod annotations;
pub use annotations::annotate_expression;

pub mod questionnaire_index;
pub use questionnaire_index::QuestionnaireIndex;
