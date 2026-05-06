//! Expression result inference (type + cardinality).
//!
//! Walks the parsed AST and reports the result type ([`InferredType`]) and
//! cardinality ([`Cardinality`]) of an expression. Conservative: anything
//! opaque (custom functions, mixed `iif` branches, dynamic chains) returns
//! `Unknown` rather than guessing.
//!
//! Used by [`super::analyze_expression`] when the caller declares an
//! `expected_result_type` or `expected_cardinality` on the
//! [`super::AnalysisContext`]. A definite mismatch produces a diagnostic;
//! `Unknown` is silent.

use crate::analyze::questionnaire_index::QuestionnaireIndex;
use crate::analyze::{
    Annotation, AnnotationKind, Attribution, Cardinality, InferredType, ValueAccessor,
};
use crate::ast::{BinOp, Expr, Invocation, Literal, TypeOp, TypeSpecifier};

/// Map a Questionnaire item type + value-accessor to an inferred result type.
///
/// Mirrors `validate_types::value_type_for_item` but at finer granularity:
/// distinguishes `Integer` vs `Decimal`, `Date` vs `DateTime`, etc., and
/// resolves the accessor on `Coding` items so `.code`/`.display` map to
/// `String`.
fn answer_leaf_type(item_type: &str, accessor: &ValueAccessor) -> InferredType {
    match item_type {
        "boolean" => InferredType::Boolean,
        "decimal" => InferredType::Decimal,
        "integer" => InferredType::Integer,
        "string" | "text" | "url" => InferredType::String,
        "date" => InferredType::Date,
        "dateTime" => InferredType::DateTime,
        "time" => InferredType::Time,
        "quantity" => InferredType::Quantity,
        "choice" | "open-choice" | "coding" => match accessor {
            ValueAccessor::Value => InferredType::Coding,
            ValueAccessor::Code | ValueAccessor::Display => InferredType::String,
        },
        _ => InferredType::Unknown,
    }
}

/// Read the type-spec name out of a `TypeSpecifier` (used by `is` / `as` /
/// `ofType`). Returns the unqualified head identifier — good enough for
/// matching well-known FHIRPath / FHIR primitive names.
fn type_specifier_name(ts: &TypeSpecifier) -> Option<&str> {
    ts.qualified.parts.first().map(|id| id.raw.as_str())
}

/// Map a FHIR / FHIRPath type specifier name (e.g. `Boolean`, `boolean`,
/// `String`, `Coding`) to an `InferredType`. Case-insensitive on the first
/// letter so both `Boolean` (FHIRPath) and `boolean` (FHIR primitive) work.
fn type_name_to_inferred(name: &str) -> InferredType {
    match name.to_ascii_lowercase().as_str() {
        "boolean" => InferredType::Boolean,
        "string" | "code" | "id" | "uri" | "url" | "canonical" | "oid" | "uuid" | "markdown"
        | "base64binary" => InferredType::String,
        "integer" | "positiveint" | "unsignedint" => InferredType::Integer,
        "decimal" => InferredType::Decimal,
        "date" => InferredType::Date,
        "datetime" | "instant" => InferredType::DateTime,
        "time" => InferredType::Time,
        "quantity" | "age" | "duration" | "count" | "distance" | "money" => InferredType::Quantity,
        "coding" | "codeableconcept" => InferredType::Coding,
        _ => InferredType::Unknown,
    }
}

/// If the argument is itself an `Expr::Invocation` with a Member call whose
/// identifier names a type, return that name. Used by `ofType(Coding)` etc.
fn arg_to_type_name(expr: &Expr) -> Option<&str> {
    let expr = unwrap_passthrough(expr);
    match expr {
        Expr::Invocation {
            receiver: None,
            call: Invocation::Member { ident },
            ..
        } => Some(ident.raw.as_str()),
        _ => None,
    }
}

/// Read the type-spec name from a `Type` expression's TypeSpecifier.
fn type_op_name(ts: &TypeSpecifier) -> Option<&str> {
    type_specifier_name(ts)
}

/// Result-type lookup for a function call, given the (already-inferred) type
/// of its receiver chain. `args` is the list of argument expressions if any
/// (for `ofType`, `as`, `iif`).
fn function_result_type(
    name: &str,
    receiver: InferredType,
    args: &[&Expr],
    index: &QuestionnaireIndex,
    annotations: &[Annotation],
) -> InferredType {
    // ── Boolean-returning ────────────────────────────────────────────────
    match name {
        "empty"
        | "exists"
        | "all"
        | "allTrue"
        | "anyTrue"
        | "allFalse"
        | "anyFalse"
        | "isDistinct"
        | "subsetOf"
        | "supersetOf"
        | "not"
        | "hasValue"
        | "startsWith"
        | "endsWith"
        | "matches"
        | "matchesFull"
        | "is"
        | "convertsToBoolean"
        | "convertsToInteger"
        | "convertsToDecimal"
        | "convertsToString"
        | "convertsToDate"
        | "convertsToDateTime"
        | "convertsToTime"
        | "convertsToQuantity" => return InferredType::Boolean,
        // `contains` is overloaded: substring on a String receiver, membership
        // on a collection. Both return Boolean.
        "contains" => return InferredType::Boolean,
        _ => {}
    }

    // ── Numeric ──────────────────────────────────────────────────────────
    match name {
        "count" | "length" | "indexOf" | "toInteger" => return InferredType::Integer,
        "toDecimal" => return InferredType::Decimal,
        "sum" | "avg" | "min" | "max" => {
            // Pass through the receiver type when it's numeric, otherwise
            // Unknown — we can't tell which.
            return match receiver {
                InferredType::Integer | InferredType::Decimal | InferredType::Quantity => receiver,
                _ => InferredType::Unknown,
            };
        }
        _ => {}
    }

    // ── String ───────────────────────────────────────────────────────────
    match name {
        "toString" | "substring" | "replace" | "replaceMatches" | "upper" | "lower" | "trim"
        | "split" | "join" | "encode" | "decode" | "toChars" => return InferredType::String,
        _ => {}
    }

    // ── Date / DateTime / Time ───────────────────────────────────────────
    match name {
        "toDate" | "today" => return InferredType::Date,
        "toDateTime" | "now" => return InferredType::DateTime,
        "toTime" | "timeOfDay" => return InferredType::Time,
        _ => {}
    }

    // ── Quantity ─────────────────────────────────────────────────────────
    if name == "toQuantity" {
        return InferredType::Quantity;
    }

    // ── Type-asserting (preserve the asserted type) ──────────────────────
    if name == "ofType" || name == "as" {
        if let Some(arg) = args.first() {
            if let Some(tn) = arg_to_type_name(arg) {
                return type_name_to_inferred(tn);
            }
        }
        return InferredType::Unknown;
    }

    // ── iif: infer both branches; if equal, take that type ───────────────
    if name == "iif" {
        // iif(criterion, true-result [, otherwise-result])
        let true_branch = args.get(1).copied();
        let false_branch = args.get(2).copied();
        let true_t = true_branch
            .map(|n| infer_node(n, annotations, index))
            .unwrap_or(InferredType::Unknown);
        let false_t = false_branch
            .map(|n| infer_node(n, annotations, index))
            .unwrap_or(InferredType::Unknown);
        // Two-arg iif (no `otherwise`) — empty otherwise branch is fine.
        if false_branch.is_none() {
            return true_t;
        }
        if true_t == false_t {
            return true_t;
        }
        return InferredType::Unknown;
    }

    // ── Identity-on-type: pass through receiver ──────────────────────────
    match name {
        "first" | "last" | "single" | "tail" | "take" | "skip" | "distinct" | "where"
        | "intersect" | "exclude" | "combine" | "union" | "descendants" | "children"
        | "repeat" | "aggregate" | "select" => receiver,
        _ => InferredType::Unknown,
    }
}

/// Strip parenthesized wrappers that don't affect type/cardinality.
fn unwrap_passthrough(mut node: &Expr) -> &Expr {
    while let Expr::Parenthesized { inner, .. } = node {
        node = inner;
    }
    node
}

/// Try to resolve an InvocationExpression chain to an answer-leaf type by
/// matching it against an existing annotation.
fn answer_leaf_for_chain(
    node: &Expr,
    annotations: &[Annotation],
    index: &QuestionnaireIndex,
) -> Option<InferredType> {
    let span = node.span();
    for ann in annotations {
        if ann.span.start != span.byte_start || ann.span.end != span.byte_end {
            continue;
        }
        // Skip degraded attribution — path is no longer precise.
        if matches!(
            ann.attribution,
            Attribution::WidenedScope | Attribution::Unattributable
        ) {
            return Some(InferredType::Unknown);
        }
        if let AnnotationKind::AnswerReference { link_ids, accessor } = &ann.kind {
            let last = link_ids.last()?;
            let item_type = index.resolve_item_type(last)?;
            return Some(answer_leaf_type(item_type, accessor));
        }
    }
    None
}

/// Recursive type inference on a single AST node.
fn infer_node(node: &Expr, annotations: &[Annotation], index: &QuestionnaireIndex) -> InferredType {
    let node = unwrap_passthrough(node);

    match node {
        // ── Literals ──
        Expr::Literal(lit) => match lit {
            Literal::Boolean { .. } => InferredType::Boolean,
            Literal::String { .. } => InferredType::String,
            Literal::Number { raw, .. } => {
                if raw.contains('.') {
                    InferredType::Decimal
                } else {
                    InferredType::Integer
                }
            }
            Literal::DateTime { raw, .. } => {
                // FHIRPath date-time literals start with `@`. Presence of `T`
                // distinguishes a date-time from a bare date.
                if raw.contains('T') {
                    InferredType::DateTime
                } else {
                    InferredType::Date
                }
            }
            Literal::Time { .. } => InferredType::Time,
            Literal::Quantity { .. } => InferredType::Quantity,
            Literal::Null { .. } => InferredType::Unknown,
        },

        // ── Boolean-producing operators ──
        Expr::Binary { op, lhs, rhs, .. } => match op {
            BinOp::Eq
            | BinOp::NotEq
            | BinOp::EquivTilde
            | BinOp::NotEquivTilde
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::In
            | BinOp::Contains
            | BinOp::And
            | BinOp::Or
            | BinOp::Xor
            | BinOp::Implies => InferredType::Boolean,

            BinOp::Concat => InferredType::String,

            BinOp::Plus | BinOp::Minus => {
                let l = infer_node(lhs, annotations, index);
                let r = infer_node(rhs, annotations, index);
                numeric_combine(l, r)
            }
            BinOp::Mul => {
                let l = infer_node(lhs, annotations, index);
                let r = infer_node(rhs, annotations, index);
                numeric_combine(l, r)
            }
            BinOp::TrueDiv => {
                let l = infer_node(lhs, annotations, index);
                let r = infer_node(rhs, annotations, index);
                if matches!(l, InferredType::Integer | InferredType::Decimal)
                    && matches!(r, InferredType::Integer | InferredType::Decimal)
                {
                    InferredType::Decimal
                } else {
                    numeric_combine(l, r)
                }
            }
            BinOp::IntDiv | BinOp::Mod => {
                let l = infer_node(lhs, annotations, index);
                let r = infer_node(rhs, annotations, index);
                if matches!(l, InferredType::Integer) && matches!(r, InferredType::Integer) {
                    InferredType::Integer
                } else if matches!(l, InferredType::Integer | InferredType::Decimal)
                    && matches!(r, InferredType::Integer | InferredType::Decimal)
                {
                    InferredType::Decimal
                } else {
                    InferredType::Unknown
                }
            }
            // Union `a | b` — same type if both operands agree, else Unknown.
            BinOp::Union => {
                let l = infer_node(lhs, annotations, index);
                let r = infer_node(rhs, annotations, index);
                if l == r {
                    l
                } else {
                    InferredType::Unknown
                }
            }
        },

        Expr::Type { op, type_spec, .. } => match op {
            TypeOp::Is => InferredType::Boolean,
            TypeOp::As => type_op_name(type_spec)
                .map(type_name_to_inferred)
                .unwrap_or(InferredType::Unknown),
        },

        Expr::Polarity { operand, .. } => infer_node(operand, annotations, index),

        // Bracket index `expr[i]` — type of the receiver.
        Expr::Indexer { receiver, .. } => infer_node(receiver, annotations, index),

        // ── Invocations (chained or standalone) ──
        Expr::Invocation { receiver, call, .. } => {
            // First, try to match this whole chain against an annotation so
            // an answer-leaf chain (e.g. `item.where(linkId='x').answer.value`)
            // resolves via the QuestionnaireIndex.
            if receiver.is_some() {
                if let Some(t) = answer_leaf_for_chain(node, annotations, index) {
                    return t;
                }
            }
            match call {
                Invocation::Function { name, args, .. } => {
                    let recv_t = match receiver {
                        Some(r) => infer_node(r, annotations, index),
                        None => InferredType::Unknown,
                    };
                    let arg_refs: Vec<&Expr> = args.iter().collect();
                    function_result_type(&name.raw, recv_t, &arg_refs, index, annotations)
                }
                // Member access on something — we can't generally know
                // the field type without a schema. Boolean-returning
                // member names are rare; default to Unknown.
                Invocation::Member { .. } => InferredType::Unknown,
                Invocation::This { .. } | Invocation::Index { .. } | Invocation::Total { .. } => {
                    InferredType::Unknown
                }
            }
        }

        // External constants — opaque without binding info.
        Expr::ExternalConstant { .. } => InferredType::Unknown,

        Expr::Parenthesized { .. } => unreachable!("stripped by unwrap_passthrough"),
    }
}

/// Combine two numeric types under +/- /*. Decimal absorbs Integer.
fn numeric_combine(left: InferredType, right: InferredType) -> InferredType {
    match (left, right) {
        (InferredType::Integer, InferredType::Integer) => InferredType::Integer,
        (InferredType::Decimal, InferredType::Integer)
        | (InferredType::Integer, InferredType::Decimal)
        | (InferredType::Decimal, InferredType::Decimal) => InferredType::Decimal,
        (InferredType::Quantity, InferredType::Quantity) => InferredType::Quantity,
        _ => InferredType::Unknown,
    }
}

/// Public entry point: infer the result type of a parsed FHIRPath expression.
pub(crate) fn infer_result_type(
    ast: &Expr,
    annotations: &[Annotation],
    index: &QuestionnaireIndex,
) -> InferredType {
    infer_node(ast, annotations, index)
}

// ── Cardinality inference ──────────────────────────────────────────────

/// Cardinality classification of FHIRPath functions by name.
///
/// Three groups:
/// - `Singleton`: always returns 0..1 elements (aggregates, predicates,
///   conversions, terminal extractors).
/// - `Collection`: always returns potentially many (scope wideners,
///   set operations, splits).
/// - `PassThrough`: preserves the receiver's cardinality (filters and casts
///   that never grow the input).
///
/// Anything not listed here falls through to `Unknown`.
enum FnCardinality {
    Singleton,
    Collection,
    PassThrough,
}

fn function_cardinality(name: &str) -> Option<FnCardinality> {
    match name {
        // ── Always-singleton ──
        // Aggregates / scalar producers
        "count" | "length" | "indexOf" | "sum" | "avg" | "min" | "max" | "now" | "today"
        | "timeOfDay" | "aggregate"
        // Conversions
        | "toBoolean" | "toInteger" | "toDecimal" | "toString" | "toDate" | "toDateTime"
        | "toTime" | "toQuantity"
        // Boolean predicates
        | "exists" | "empty" | "all" | "allTrue" | "anyTrue" | "allFalse" | "anyFalse"
        | "isDistinct" | "subsetOf" | "supersetOf" | "not" | "hasValue"
        | "startsWith" | "endsWith" | "matches" | "matchesFull" | "contains"
        | "convertsToBoolean" | "convertsToInteger" | "convertsToDecimal"
        | "convertsToString" | "convertsToDate" | "convertsToDateTime"
        | "convertsToTime" | "convertsToQuantity" | "is"
        // Singleton selectors
        | "single" | "first" | "last"
        // String-on-string ops
        | "substring" | "replace" | "replaceMatches" | "upper" | "lower" | "trim"
        | "join" | "encode" | "decode" => Some(FnCardinality::Singleton),

        // ── Always-collection ──
        "descendants" | "children" | "repeat" | "tail" | "intersect" | "exclude"
        | "combine" | "union" | "split" | "toChars" => Some(FnCardinality::Collection),

        // ── Pass-through (cap-preserving filters) ──
        "where" | "ofType" | "as" | "distinct" | "select" | "skip" => {
            Some(FnCardinality::PassThrough)
        }

        _ => None,
    }
}

/// `take(n)` collapses to Singleton only when `n == 1`. Otherwise it's a
/// pass-through cap (could keep the receiver's cardinality up to n elements).
fn take_cardinality(args: &[&Expr], receiver: Cardinality) -> Cardinality {
    let Some(arg) = args.first() else {
        return Cardinality::Unknown;
    };
    if let Some(n) = literal_integer(arg) {
        if n <= 1 {
            return Cardinality::Singleton;
        }
        return receiver;
    }
    Cardinality::Unknown
}

/// Pull an integer value out of a literal expression, peeling parens.
fn literal_integer(node: &Expr) -> Option<i64> {
    let inner = unwrap_passthrough(node);
    match inner {
        Expr::Literal(Literal::Number { raw, .. }) => raw.parse::<i64>().ok(),
        _ => None,
    }
}

fn answer_leaf_cardinality_for_chain(
    node: &Expr,
    annotations: &[Annotation],
    index: &QuestionnaireIndex,
) -> Option<Cardinality> {
    let span = node.span();
    for ann in annotations {
        if ann.span.start != span.byte_start || ann.span.end != span.byte_end {
            continue;
        }
        match ann.attribution {
            // Scope-widened or fully opaque chains: bail with Unknown.
            Attribution::WidenedScope | Attribution::Unattributable => {
                return Some(Cardinality::Unknown);
            }
            // A positional selector (`first()`, `[0]`, `take(1)`, …) at the
            // tail collapses the chain to at most one element regardless of
            // whether the underlying linkId repeats.
            Attribution::PartialPositional => return Some(Cardinality::Singleton),
            Attribution::Full => {}
        }
        let link_ids = match &ann.kind {
            AnnotationKind::AnswerReference { link_ids, .. } => link_ids,
            AnnotationKind::ItemReference { link_ids } => link_ids,
            _ => continue,
        };
        let last = link_ids.last()?;
        // Repeating leaf OR repeating ancestor → Collection. Otherwise,
        // a singleton answer/item reference.
        let leaf_repeats = index.resolve_item_repeats(last)?;
        if leaf_repeats || index.has_repeating_ancestor(last) {
            return Some(Cardinality::Collection);
        }
        return Some(Cardinality::Singleton);
    }
    None
}

fn infer_card_node(
    node: &Expr,
    annotations: &[Annotation],
    index: &QuestionnaireIndex,
) -> Cardinality {
    let node = unwrap_passthrough(node);

    match node {
        // ── Literals: always one ──
        Expr::Literal(_) => Cardinality::Singleton,

        // External constants — `%foo` resolves to whatever is bound. Treat as
        // unknown rather than guessing.
        Expr::ExternalConstant { .. } => Cardinality::Unknown,

        // ── Boolean and arithmetic operators all return a singleton ──
        Expr::Binary { op, .. } => match op {
            // Union always Collection
            BinOp::Union => Cardinality::Collection,
            _ => Cardinality::Singleton,
        },

        Expr::Type { op, lhs, .. } => match op {
            TypeOp::Is => Cardinality::Singleton,
            // `as` preserves the operand's cardinality.
            TypeOp::As => infer_card_node(lhs, annotations, index),
        },

        Expr::Polarity { .. } => Cardinality::Singleton,

        // ── Indexer: single element ──
        Expr::Indexer { .. } => Cardinality::Singleton,

        // ── Invocations ──
        Expr::Invocation { receiver, call, .. } => {
            // Answer-leaf chain: defer to questionnaire metadata.
            if receiver.is_some() {
                if let Some(c) = answer_leaf_cardinality_for_chain(node, annotations, index) {
                    return c;
                }
            }
            match call {
                Invocation::Function { name, args, .. } => {
                    let recv_card = match receiver {
                        Some(r) => infer_card_node(r, annotations, index),
                        None => Cardinality::Unknown,
                    };
                    let arg_refs: Vec<&Expr> = args.iter().collect();
                    function_cardinality_dispatch(
                        &name.raw,
                        recv_card,
                        &arg_refs,
                        annotations,
                        index,
                    )
                }
                // Bare identifier in invocation position (e.g. `Patient`). With no
                // schema we can't say.
                Invocation::Member { .. } => Cardinality::Unknown,
                // Context vars: `$this`/`$index`/`$total` are scalars per spec.
                Invocation::This { .. } | Invocation::Index { .. } | Invocation::Total { .. } => {
                    Cardinality::Singleton
                }
            }
        }

        Expr::Parenthesized { .. } => unreachable!("stripped by unwrap_passthrough"),
    }
}

fn function_cardinality_dispatch(
    name: &str,
    receiver: Cardinality,
    args: &[&Expr],
    annotations: &[Annotation],
    index: &QuestionnaireIndex,
) -> Cardinality {
    if name == "iif" {
        let true_branch = args
            .get(1)
            .map(|n| infer_card_node(n, annotations, index))
            .unwrap_or(Cardinality::Unknown);
        // Two-arg iif — `otherwise` defaults to empty (Singleton-empty).
        let false_branch = args
            .get(2)
            .map(|n| infer_card_node(n, annotations, index))
            .unwrap_or(Cardinality::Singleton);
        return if true_branch == false_branch {
            true_branch
        } else if matches!(true_branch, Cardinality::Collection)
            || matches!(false_branch, Cardinality::Collection)
        {
            Cardinality::Collection
        } else {
            Cardinality::Unknown
        };
    }

    if name == "take" {
        return take_cardinality(args, receiver);
    }

    match function_cardinality(name) {
        Some(FnCardinality::Singleton) => Cardinality::Singleton,
        Some(FnCardinality::Collection) => Cardinality::Collection,
        Some(FnCardinality::PassThrough) => receiver,
        None => Cardinality::Unknown,
    }
}

/// Public entry point: infer the cardinality of a parsed FHIRPath expression.
pub(crate) fn infer_cardinality(
    ast: &Expr,
    annotations: &[Annotation],
    index: &QuestionnaireIndex,
) -> Cardinality {
    infer_card_node(ast, annotations, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::annotations::annotate_expression_with_ast;
    use serde_json::json;

    fn idx() -> QuestionnaireIndex {
        QuestionnaireIndex::build(&json!({
            "resourceType": "Questionnaire",
            "item": [
                { "linkId": "bool1", "type": "boolean" },
                { "linkId": "choice1", "type": "choice" },
                { "linkId": "string1", "type": "string" },
                { "linkId": "decimal1", "type": "decimal" },
                { "linkId": "integer1", "type": "integer" },
                { "linkId": "date1", "type": "date" },
                { "linkId": "datetime1", "type": "dateTime" },
            ]
        }))
    }

    fn infer(expr: &str) -> InferredType {
        let (ast, anns, _diags) = annotate_expression_with_ast(expr).expect("parse");
        infer_result_type(&ast, &anns, &idx())
    }

    // ── Literals ──
    #[test]
    fn boolean_literals() {
        assert_eq!(infer("true"), InferredType::Boolean);
        assert_eq!(infer("false"), InferredType::Boolean);
    }

    #[test]
    fn number_literals() {
        assert_eq!(infer("42"), InferredType::Integer);
        assert_eq!(infer("3.14"), InferredType::Decimal);
    }

    #[test]
    fn string_literal() {
        assert_eq!(infer("'hi'"), InferredType::String);
    }

    // ── Boolean operators ──
    #[test]
    fn equality_is_boolean() {
        assert_eq!(infer("1 = 1"), InferredType::Boolean);
        assert_eq!(infer("'a' != 'b'"), InferredType::Boolean);
        assert_eq!(infer("x ~ y"), InferredType::Boolean);
    }

    #[test]
    fn comparison_is_boolean() {
        assert_eq!(infer("1 < 2"), InferredType::Boolean);
        assert_eq!(infer("a >= b"), InferredType::Boolean);
    }

    #[test]
    fn logical_is_boolean() {
        assert_eq!(infer("a and b"), InferredType::Boolean);
        assert_eq!(infer("a or b"), InferredType::Boolean);
        assert_eq!(infer("a xor b"), InferredType::Boolean);
        assert_eq!(infer("a implies b"), InferredType::Boolean);
    }

    #[test]
    fn membership_is_boolean() {
        assert_eq!(infer("x in y"), InferredType::Boolean);
        assert_eq!(infer("x contains y"), InferredType::Boolean);
    }

    #[test]
    fn type_op_is_boolean() {
        assert_eq!(infer("Patient is Patient"), InferredType::Boolean);
    }

    #[test]
    fn type_op_as_uses_specifier() {
        assert_eq!(infer("x as Boolean"), InferredType::Boolean);
        assert_eq!(infer("x as String"), InferredType::String);
    }

    // ── Boolean function calls ──
    #[test]
    fn exists_is_boolean() {
        assert_eq!(infer("foo.exists()"), InferredType::Boolean);
        assert_eq!(infer("foo.empty()"), InferredType::Boolean);
        assert_eq!(infer("foo.allTrue()"), InferredType::Boolean);
    }

    #[test]
    fn string_predicates_are_boolean() {
        assert_eq!(infer("name.startsWith('Mr')"), InferredType::Boolean);
        assert_eq!(infer("name.endsWith('z')"), InferredType::Boolean);
        assert_eq!(infer("name.matches('^a.*$')"), InferredType::Boolean);
    }

    #[test]
    fn not_is_boolean() {
        assert_eq!(infer("foo.not()"), InferredType::Boolean);
    }

    // ── Numeric / string functions ──
    #[test]
    fn count_is_integer() {
        assert_eq!(infer("foo.count()"), InferredType::Integer);
        assert_eq!(infer("'abc'.length()"), InferredType::Integer);
    }

    #[test]
    fn to_string_is_string() {
        assert_eq!(infer("42.toString()"), InferredType::String);
    }

    // ── Arithmetic ──
    #[test]
    fn arithmetic_integer() {
        assert_eq!(infer("1 + 2"), InferredType::Integer);
        assert_eq!(infer("3 * 4"), InferredType::Integer);
    }

    #[test]
    fn arithmetic_decimal() {
        assert_eq!(infer("1.0 + 2"), InferredType::Decimal);
        assert_eq!(infer("1 / 2"), InferredType::Decimal);
    }

    #[test]
    fn ampersand_is_string() {
        assert_eq!(infer("'a' & 'b'"), InferredType::String);
    }

    // ── Answer-leaf chains ──
    #[test]
    fn boolean_answer_value() {
        assert_eq!(
            infer("item.where(linkId='bool1').answer.value"),
            InferredType::Boolean
        );
    }

    #[test]
    fn choice_answer_value_is_coding() {
        assert_eq!(
            infer("item.where(linkId='choice1').answer.value"),
            InferredType::Coding
        );
    }

    #[test]
    fn choice_answer_value_code_is_string() {
        assert_eq!(
            infer("item.where(linkId='choice1').answer.value.code"),
            InferredType::String
        );
    }

    #[test]
    fn date_answer_value() {
        assert_eq!(
            infer("item.where(linkId='date1').answer.value"),
            InferredType::Date
        );
        assert_eq!(
            infer("item.where(linkId='datetime1').answer.value"),
            InferredType::DateTime
        );
    }

    // ── iif ──
    #[test]
    fn iif_matching_branches() {
        assert_eq!(
            infer("iif(x > 0, true, false)"),
            InferredType::Boolean
        );
    }

    #[test]
    fn iif_mismatched_branches_unknown() {
        assert_eq!(infer("iif(x > 0, 'a', 1)"), InferredType::Unknown);
    }

    // ── Unknown fallbacks ──
    #[test]
    fn unknown_chain_is_unknown() {
        assert_eq!(infer("Patient.name.given"), InferredType::Unknown);
    }

    #[test]
    fn unknown_function_is_unknown() {
        assert_eq!(infer("foo.someCustomFn()"), InferredType::Unknown);
    }

    // ── Identity-on-type passthrough ──
    #[test]
    fn first_passes_through_type() {
        assert_eq!(
            infer("item.where(linkId='bool1').answer.value.first()"),
            InferredType::Boolean
        );
    }

    // ── Parentheses ──
    #[test]
    fn parens_are_transparent() {
        assert_eq!(infer("(1 + 1)"), InferredType::Integer);
        assert_eq!(infer("(true and false)"), InferredType::Boolean);
    }

    // ── Cardinality ──────────────────────────────────────────────────────

    fn idx_with_repeats() -> QuestionnaireIndex {
        QuestionnaireIndex::build(&json!({
            "resourceType": "Questionnaire",
            "item": [
                { "linkId": "bool1", "type": "boolean" },
                { "linkId": "choice_single", "type": "choice" },
                { "linkId": "choice_multi", "type": "choice", "repeats": true },
                {
                    "linkId": "rep_group",
                    "type": "group",
                    "repeats": true,
                    "item": [
                        { "linkId": "in_repeating", "type": "string" }
                    ]
                }
            ]
        }))
    }

    fn card(expr: &str) -> Cardinality {
        let (ast, anns, _diags) = annotate_expression_with_ast(expr).expect("parse");
        infer_cardinality(&ast, &anns, &idx_with_repeats())
    }

    #[test]
    fn literals_are_singleton() {
        assert_eq!(card("true"), Cardinality::Singleton);
        assert_eq!(card("'hi'"), Cardinality::Singleton);
        assert_eq!(card("42"), Cardinality::Singleton);
    }

    #[test]
    fn boolean_operators_are_singleton() {
        assert_eq!(card("a = b"), Cardinality::Singleton);
        assert_eq!(card("a and b"), Cardinality::Singleton);
        assert_eq!(card("a > b"), Cardinality::Singleton);
        assert_eq!(card("a in b"), Cardinality::Singleton);
        assert_eq!(card("x is Patient"), Cardinality::Singleton);
    }

    #[test]
    fn arithmetic_is_singleton() {
        assert_eq!(card("1 + 2"), Cardinality::Singleton);
        assert_eq!(card("3 * 4"), Cardinality::Singleton);
        assert_eq!(card("'a' & 'b'"), Cardinality::Singleton);
    }

    #[test]
    fn aggregates_are_singleton() {
        assert_eq!(card("foo.count()"), Cardinality::Singleton);
        assert_eq!(card("foo.exists()"), Cardinality::Singleton);
        assert_eq!(card("foo.sum()"), Cardinality::Singleton);
        assert_eq!(card("'abc'.length()"), Cardinality::Singleton);
    }

    #[test]
    fn singleton_selectors_are_singleton() {
        assert_eq!(card("foo.first()"), Cardinality::Singleton);
        assert_eq!(card("foo.last()"), Cardinality::Singleton);
        assert_eq!(card("foo.single()"), Cardinality::Singleton);
        assert_eq!(card("foo.take(1)"), Cardinality::Singleton);
    }

    #[test]
    fn collection_widening_is_collection() {
        assert_eq!(card("foo.descendants()"), Cardinality::Collection);
        assert_eq!(card("foo.children()"), Cardinality::Collection);
        assert_eq!(card("foo.repeat($this)"), Cardinality::Collection);
        assert_eq!(card("a | b"), Cardinality::Collection);
        assert_eq!(card("'abc'.split(',')"), Cardinality::Collection);
    }

    #[test]
    fn indexer_is_singleton() {
        assert_eq!(card("foo[0]"), Cardinality::Singleton);
    }

    #[test]
    fn answer_chain_non_repeating_is_singleton() {
        assert_eq!(
            card("item.where(linkId='bool1').answer.value"),
            Cardinality::Singleton
        );
        assert_eq!(
            card("item.where(linkId='choice_single').answer.value"),
            Cardinality::Singleton
        );
    }

    #[test]
    fn answer_chain_repeating_is_collection() {
        assert_eq!(
            card("item.where(linkId='choice_multi').answer.value"),
            Cardinality::Collection
        );
    }

    #[test]
    fn answer_chain_in_repeating_group_is_collection() {
        assert_eq!(
            card("item.where(linkId='in_repeating').answer.value"),
            Cardinality::Collection
        );
    }

    #[test]
    fn first_after_collection_is_singleton() {
        assert_eq!(
            card("item.where(linkId='choice_multi').answer.value.first()"),
            Cardinality::Singleton
        );
    }

    #[test]
    fn where_passes_through_cardinality_on_unknown_receiver() {
        // No QR-chain match → fall through to the function dispatcher.
        // `where` is pass-through; the receiver is Unknown; result Unknown.
        assert_eq!(card("foo.where(x = 1)"), Cardinality::Unknown);
    }

    #[test]
    fn iif_singleton_branches_is_singleton() {
        assert_eq!(card("iif(x > 0, 1, 2)"), Cardinality::Singleton);
    }

    #[test]
    fn iif_collection_branch_is_collection() {
        assert_eq!(
            card("iif(x > 0, foo.descendants(), bar.descendants())"),
            Cardinality::Collection
        );
    }

    #[test]
    fn unknown_function_is_unknown_cardinality() {
        assert_eq!(card("foo.someCustomFn()"), Cardinality::Unknown);
    }
}
