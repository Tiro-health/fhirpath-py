/// FHIRPath expression analysis in SDC context.
///
/// Provides semantic annotations and diagnostics for any FHIRPath expression
/// that runs against a QuestionnaireResponse — templateExtractContext,
/// templateExtractValue, calculatedExpression, answerExpression,
/// enableWhenExpression, etc.
///
/// ## Architecture
///
/// ```text
///              ┌─────────────────────────────────────────────────────┐
///              │                    analyze_expression                │  <- orchestrator
///              │    (expr, &QuestionnaireIndex, &AnalysisContext)    │
///              │                  → ExpressionAnalysis                │
///              └─────────┬─────────────────────────────────┬─────────┘
///                        │                                 │
///                        ▼                                 ▼
///           ┌────────────────────────┐       ┌──────────────────────────┐
///           │      annotations       │       │        validators        │
///           │   (index-agnostic)     │       │      (index-aware)       │
///           │                        │       │                          │
///           │   expr → AST → chain   │──────▶│  validate_link_ids       │
///           │   steps → state        │  ann  │  validate_types          │
///           │   machine → Vec<Ann>   │       │  validate_context        │
///           │         + Vec<Diag>    │       │       → Vec<Diagnostic>  │
///           └────────────────────────┘       └──────────────────────────┘
///
///           ┌────────────────────────┐       ┌──────────────────────────┐
///           │      completions       │       │         resolve          │
///           │  expr + index → items  │       │   expr + base → expr'    │
///           │   (separate entrypt)   │       │ (%context substitution)  │
///           └────────────────────────┘       └──────────────────────────┘
/// ```
///
/// ## Interfaces
///
/// - **Annotator** (`annotations::annotate_expression_with_diagnostics`) —
///   purely syntactic. Decomposes each navigation chain into
///   `ChainStep`s (see `annotations.rs`) and runs a state machine producing
///   [`Annotation`]s (with [`Attribution`] lattice) plus any
///   `ExpressionNotAttributable` [`Diagnostic`]s. **Never sees the
///   [`QuestionnaireIndex`]** — keeps this layer testable in isolation
///   and makes it safe to call from contexts without a Questionnaire.
///
/// - **Validators** — each takes `&[Annotation]` (and/or the raw `expr` +
///   a reparsed AST) plus `&QuestionnaireIndex`, returns `Vec<Diagnostic>`.
///   They don't mutate annotations; they only read them and emit extra
///   diagnostics. Gated on [`Attribution`]: once an annotation drops to
///   `WidenedScope` / `Unattributable`, type and reachability checks skip
///   it because the path is no longer precisely modeled.
///
/// - **Orchestrator** ([`analyze_expression`]) — wires annotator + the
///   three validators in a fixed order and merges their diagnostics. This
///   is the primary public entry point; bindings call it directly.
///
/// - **Bindings** (`crate::bindings::{python, wasm}`) — thin adapters.
///   [`Annotation`] and [`Diagnostic`] already derive `serde::Serialize`
///   with snake_case variants; the WASM binding uses that directly, the
///   Python binding converts manually to preserve v3.0.0 dict shapes
///   (notably: `Attribution::Full` omits the `attribution` key).
///
/// ## Attribution lattice
///
/// Demotion is monotonic (see `Attribution::demote_to`). A chain never
/// regains precision:
///
/// ```text
///   Full ──(positional op)──▶ PartialPositional
///    │                             │
///    └───────(descendants / children / repeat)─────▶ WidenedScope
///                                                         │
///   Full ──────(iif / select / non-linkId where)──▶ Unattributable ◀─┘
/// ```
///
/// Once below `PartialPositional`, linkIds collected so far are preserved
/// in the terminal annotation but consumers should treat them as hints.

// ── Core types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueAccessor {
    /// `.answer.value` (bare)
    Value,
    /// `.answer.value.code`
    Code,
    /// `.answer.value.display`
    Display,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

/// How precisely an annotation attributes to its linkId scope.
///
/// Ordered from most to least precise. Transitions only ever demote (never
/// promote) — once a chain loses precision, it stays lost.
///
/// - `Full` — the expression navigates to the complete set of answers/items
///   for the named linkId(s).
/// - `PartialPositional` — a positional selector (`first()`, `[i]`, `take(1)`, …)
///   narrowed the navigation to a subset. The linkId attribution still holds,
///   but the evaluator sees fewer items than a bare `where(linkId=…)` would.
/// - `WidenedScope` — a scope-widening operator (`descendants()`, `children()`,
///   `repeat()`) broke the structural parent-chain. linkIds are still known,
///   but the path to them is no longer deterministic — parent-context
///   reachability checks can't be trusted.
/// - `Unattributable` — an opaque operation (`iif()`, `select()`, non-linkId
///   `where(…)`, unknown function) severed precise attribution. linkIds
///   collected up to that point are preserved but consumers should treat them
///   as hints, not guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    #[default]
    Full,
    PartialPositional,
    WidenedScope,
    Unattributable,
}

impl Attribution {
    pub(crate) fn is_default(&self) -> bool {
        matches!(self, Attribution::Full)
    }

    /// Rank within the lattice. Higher = more precise.
    fn rank(&self) -> u8 {
        match self {
            Attribution::Full => 3,
            Attribution::PartialPositional => 2,
            Attribution::WidenedScope => 1,
            Attribution::Unattributable => 0,
        }
    }

    /// Degrade `self` toward `floor`, never below.
    /// Returns the lower-ranked of the two.
    pub(crate) fn demote_to(self, floor: Attribution) -> Attribution {
        if floor.rank() < self.rank() {
            floor
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Annotation {
    pub span: Span,
    pub kind: AnnotationKind,
    #[serde(default, skip_serializing_if = "Attribution::is_default")]
    pub attribution: Attribution,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    UnknownLinkId,
    UnreachableLinkId,
    InvalidAccessorForType,
    MissingAccessorForCoding,
    ItemReferenceTargetsLeaf,
    ContextUnreachableFromParent,
    ExpressionNotAttributable,
    ExpressionTypeMismatch,
    ExpressionCardinalityMismatch,
}

/// Inferred result type of a FHIRPath expression.
///
/// Conservative — when inference cannot determine the type with confidence,
/// returns `Unknown`. Unknown is silent: it never produces a mismatch
/// diagnostic, only a *known* type that contradicts the expected type does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InferredType {
    Boolean,
    String,
    Integer,
    Decimal,
    Date,
    DateTime,
    Time,
    Quantity,
    Coding,
    Unknown,
}

/// Inferred cardinality of a FHIRPath expression result.
///
/// Three-state: `Singleton` (provably 0..1), `Collection` (could yield more
/// than one), or `Unknown` (could not determine). Same conservative-silent-
/// on-Unknown rule as [`InferredType`] — a definite mismatch produces an
/// `ExpressionCardinalityMismatch` diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    /// At most one element (0..1 or 1..1).
    Singleton,
    /// Possibly more than one element.
    Collection,
    /// Could not be determined.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
}

// ── Unified analysis API ────────────────────────────────────────────────

/// Describes the scope an expression runs within.
#[derive(Debug, Clone, Default)]
pub struct AnalysisContext {
    /// The linkId of the scope this expression runs within.
    /// Used for reachability checks — are the referenced linkIds
    /// descendants of this scope?
    pub scope_link_id: Option<String>,

    /// The parent scope's context expression (raw FHIRPath string).
    /// When provided, validates that item references in this expression
    /// are reachable from the parent scope's target.
    pub parent_context_expr: Option<String>,

    /// Expected result type of the expression.
    /// When set, the analyzer infers the expression's result type and emits
    /// `ExpressionTypeMismatch` if inference produces a definite, conflicting
    /// type. `Unknown` results never produce a diagnostic.
    pub expected_result_type: Option<InferredType>,

    /// Expected cardinality of the expression's result.
    /// When set, the analyzer infers the expression's cardinality and emits
    /// `ExpressionCardinalityMismatch` if inference produces a definite,
    /// conflicting cardinality. `Unknown` results never produce a diagnostic.
    pub expected_cardinality: Option<Cardinality>,
}

/// Result of analyzing a FHIRPath expression.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExpressionAnalysis {
    pub annotations: Vec<Annotation>,
    pub diagnostics: Vec<Diagnostic>,
    /// Inferred result type of the expression. `Unknown` when inference
    /// cannot determine the type with confidence — UI hosts should treat
    /// it as "no information" rather than "any type".
    pub inferred_type: InferredType,
    /// Inferred cardinality of the expression's result. `Unknown` when
    /// inference cannot determine the cardinality with confidence.
    pub inferred_cardinality: Cardinality,
}

/// Analyze a FHIRPath expression in the context of a Questionnaire.
///
/// Produces semantic annotations (answer references, coded values, item
/// references) and validation diagnostics (unknown linkIds, type mismatches,
/// unreachable items, etc.).
///
/// Works for any SDC expression type — templateExtractContext,
/// templateExtractValue, calculatedExpression, answerExpression,
/// enableWhenExpression, initialExpression, etc.
pub fn analyze_expression(
    expr: &str,
    index: &questionnaire_index::QuestionnaireIndex,
    context: &AnalysisContext,
) -> Result<ExpressionAnalysis, crate::ParseError> {
    let (ast, ann, mut diagnostics) = annotations::annotate_expression_with_ast(expr)?;

    // 1. LinkId validation — applies to ALL expressions
    diagnostics.extend(validate_link_ids::validate_link_ids_from_expr(
        expr,
        index,
        context.scope_link_id.as_deref(),
    )?);

    // 2. Value type validation — applies to expressions with answer references
    diagnostics.extend(validate_types::validate_value_types_from_annotations(
        &ann, index,
    ));

    // 3. Structural checks on item references — applies to ALL expressions
    diagnostics.extend(validate_context::validate_item_refs(
        expr,
        &ann,
        context.parent_context_expr.as_deref(),
        index,
    )?);

    // 4. Result-type and cardinality inference — always run so callers can
    //    use the values for UI metadata (hover, badges, …). Diagnostics fire
    //    only when an expected_* is set and inference produces a definite,
    //    conflicting result. `Unknown` is silent.
    let inferred_type = result_type::infer_result_type(&ast, &ann, index);
    let inferred_cardinality = result_type::infer_cardinality(&ast, &ann, index);

    if let Some(expected) = context.expected_result_type {
        if inferred_type != InferredType::Unknown && inferred_type != expected {
            diagnostics.push(Diagnostic {
                span: Span {
                    start: ast.byte_start,
                    end: ast.byte_end,
                },
                severity: Severity::Error,
                code: DiagnosticCode::ExpressionTypeMismatch,
                message: format!(
                    "Expression evaluates to {} but {} was expected",
                    inferred_type_name(inferred_type),
                    inferred_type_name(expected),
                ),
            });
        }
    }

    if let Some(expected) = context.expected_cardinality {
        if inferred_cardinality != Cardinality::Unknown && inferred_cardinality != expected {
            diagnostics.push(Diagnostic {
                span: Span {
                    start: ast.byte_start,
                    end: ast.byte_end,
                },
                severity: Severity::Error,
                code: DiagnosticCode::ExpressionCardinalityMismatch,
                message: format!(
                    "Expression yields {} but {} was expected",
                    cardinality_name(inferred_cardinality),
                    cardinality_name(expected),
                ),
            });
        }
    }

    Ok(ExpressionAnalysis {
        annotations: ann,
        diagnostics,
        inferred_type,
        inferred_cardinality,
    })
}

fn cardinality_name(c: Cardinality) -> &'static str {
    match c {
        Cardinality::Singleton => "a singleton",
        Cardinality::Collection => "a collection",
        Cardinality::Unknown => "unknown",
    }
}

fn inferred_type_name(t: InferredType) -> &'static str {
    match t {
        InferredType::Boolean => "Boolean",
        InferredType::String => "String",
        InferredType::Integer => "Integer",
        InferredType::Decimal => "Decimal",
        InferredType::Date => "Date",
        InferredType::DateTime => "DateTime",
        InferredType::Time => "Time",
        InferredType::Quantity => "Quantity",
        InferredType::Coding => "Coding",
        InferredType::Unknown => "Unknown",
    }
}

// ── Internal modules ────────────────────────────────────────────────────

pub mod annotations;
pub mod completions;
pub mod questionnaire_index;

pub(crate) mod result_type;
pub(crate) mod validate_context;
pub(crate) mod validate_link_ids;
pub(crate) mod validate_types;

// ── Convenience re-exports ──────────────────────────────────────────────

pub use annotations::annotate_expression;
pub use completions::{generate_completions, CompletionItem, CompletionItemKind};
pub use questionnaire_index::QuestionnaireIndex;

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
                    "type": "group",
                    "item": [
                        { "linkId": "choice1", "type": "choice" },
                        { "linkId": "bool1", "type": "boolean" },
                        {
                            "linkId": "subgroup",
                            "type": "group",
                            "item": [
                                { "linkId": "deep", "type": "string" }
                            ]
                        }
                    ]
                },
                {
                    "linkId": "group2",
                    "type": "group",
                    "item": [
                        { "linkId": "other", "type": "decimal" }
                    ]
                }
            ]
        })
    }

    #[test]
    fn test_value_expression_gets_linkid_and_type_checks() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='bool1').answer.value.code",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(!result.annotations.is_empty());
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::InvalidAccessorForType));
    }

    #[test]
    fn test_value_expression_unknown_linkid() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='typo').answer.value",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::UnknownLinkId));
    }

    #[test]
    fn test_value_expression_with_scope() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='other').answer.value",
            &idx,
            &AnalysisContext {
                scope_link_id: Some("group1".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::UnreachableLinkId));
    }

    #[test]
    fn test_item_ref_to_leaf_is_warning() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1').item.where(linkId='bool1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().any(|d| d.code == DiagnosticCode::ItemReferenceTargetsLeaf));
    }

    #[test]
    fn test_item_ref_to_group_is_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='group1')",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.iter().all(|d| d.code != DiagnosticCode::ItemReferenceTargetsLeaf));
    }

    #[test]
    fn test_parent_context_unreachable() {
        let idx = QuestionnaireIndex::build(&questionnaire());
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
    fn test_parent_context_reachable() {
        let idx = QuestionnaireIndex::build(&questionnaire());
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
    fn test_clean_value_expression() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='choice1').answer.value.code",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.diagnostics.is_empty());
        assert!(!result.annotations.is_empty());
    }

    #[test]
    fn test_calculated_expression_with_scope() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='deep').answer.value",
            &idx,
            &AnalysisContext {
                scope_link_id: Some("subgroup".into()),
                ..Default::default()
            },
        ).unwrap();
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn test_non_qr_expression_passes_clean() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "Patient.name.given",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result.annotations.is_empty());
        assert!(result.diagnostics.is_empty());
    }

    // ── expected_result_type ──

    #[test]
    fn expected_boolean_definite_mismatch_emits_diagnostic() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "1 + 1",
            &idx,
            &AnalysisContext {
                expected_result_type: Some(InferredType::Boolean),
                ..Default::default()
            },
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::ExpressionTypeMismatch));
    }

    #[test]
    fn expected_boolean_match_no_diagnostic() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "item.where(linkId='bool1').answer.value",
            &idx,
            &AnalysisContext {
                expected_result_type: Some(InferredType::Boolean),
                ..Default::default()
            },
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ExpressionTypeMismatch));
    }

    #[test]
    fn expected_boolean_unknown_is_silent() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "Patient.name.given",
            &idx,
            &AnalysisContext {
                expected_result_type: Some(InferredType::Boolean),
                ..Default::default()
            },
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ExpressionTypeMismatch));
    }

    #[test]
    fn no_expected_type_no_inference_diagnostic() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "1 + 1",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ExpressionTypeMismatch));
    }

    // ── expected_cardinality ──

    fn questionnaire_with_repeats() -> serde_json::Value {
        json!({
            "resourceType": "Questionnaire",
            "item": [
                { "linkId": "bool1", "type": "boolean" },
                { "linkId": "multi", "type": "choice", "repeats": true },
            ]
        })
    }

    #[test]
    fn expected_singleton_collection_emits_diagnostic() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_repeats());
        let result = analyze_expression(
            "item.where(linkId='multi').answer.value",
            &idx,
            &AnalysisContext {
                expected_cardinality: Some(Cardinality::Singleton),
                ..Default::default()
            },
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::ExpressionCardinalityMismatch));
    }

    #[test]
    fn expected_singleton_match_no_diagnostic() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_repeats());
        let result = analyze_expression(
            "item.where(linkId='bool1').answer.value",
            &idx,
            &AnalysisContext {
                expected_cardinality: Some(Cardinality::Singleton),
                ..Default::default()
            },
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ExpressionCardinalityMismatch));
    }

    #[test]
    fn expected_singleton_unknown_is_silent() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_repeats());
        let result = analyze_expression(
            "Patient.name.given",
            &idx,
            &AnalysisContext {
                expected_cardinality: Some(Cardinality::Singleton),
                ..Default::default()
            },
        ).unwrap();
        assert!(result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ExpressionCardinalityMismatch));
    }

    // ── inferred_type / inferred_cardinality on result ──

    #[test]
    fn result_exposes_inferred_type_and_cardinality_without_expectations() {
        let idx = QuestionnaireIndex::build(&questionnaire_with_repeats());
        let result = analyze_expression(
            "item.where(linkId='multi').answer.value",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert_eq!(result.inferred_type, InferredType::Coding);
        assert_eq!(result.inferred_cardinality, Cardinality::Collection);
    }

    #[test]
    fn result_exposes_unknown_when_inference_cannot_determine() {
        let idx = QuestionnaireIndex::build(&questionnaire());
        let result = analyze_expression(
            "Patient.name.given",
            &idx,
            &AnalysisContext::default(),
        ).unwrap();
        assert_eq!(result.inferred_type, InferredType::Unknown);
        assert_eq!(result.inferred_cardinality, Cardinality::Unknown);
    }

    #[test]
    fn enable_when_style_check_combines_type_and_cardinality() {
        // Realistic enableWhenExpression validation: must be a singleton boolean.
        let idx = QuestionnaireIndex::build(&questionnaire_with_repeats());
        let result = analyze_expression(
            "item.where(linkId='multi').answer.value",
            &idx,
            &AnalysisContext {
                expected_result_type: Some(InferredType::Boolean),
                expected_cardinality: Some(Cardinality::Singleton),
                ..Default::default()
            },
        ).unwrap();
        // Multi-choice yields Coding (not Boolean) and Collection (not Singleton).
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::ExpressionTypeMismatch));
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::ExpressionCardinalityMismatch));
    }
}
