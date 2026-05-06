/// AST-level resolution of `%context` references in FHIRPath expressions.
///
/// `resolve_context` parses both expressions, substitutes every `%context`
/// reference in `expr` with the parsed `base` AST, then serializes the
/// result back to a valid FHIRPath string.

use crate::ast::{
    Expr, ExternalConstantId, Invocation, Literal, QualifiedIdentifier, TypeSpecifier, Unit,
};
use crate::ParseError;

// ── Public API ─────────────────────────────────────────────────────────

/// String-in / string-out convenience wrapper.
///
/// Parses both expressions, performs AST-level substitution, and returns the
/// serialized result.  Returns `expr` unchanged when no `%context` reference
/// exists.
pub fn resolve_context(expr: &str, base: &str) -> Result<String, ParseError> {
    let expr_ast = crate::parse(expr)?;
    let base_ast = crate::parse(base)?;
    let resolved = resolve_context_ast(&expr_ast, &base_ast);
    Ok(expr_to_string(&resolved))
}

/// AST-level substitution: replace every `%context` node in `expr_ast` with
/// a clone of `base_ast`.
pub fn resolve_context_ast(expr_ast: &Expr, base_ast: &Expr) -> Expr {
    substitute(expr_ast, "context", base_ast)
}

// ── Substitution ───────────────────────────────────────────────────────

/// Recursively walk `expr`, replacing any `ExternalConstant` with the given
/// constant name with a clone of `replacement`.
fn substitute(expr: &Expr, name: &str, replacement: &Expr) -> Expr {
    match expr {
        Expr::ExternalConstant { ident, .. } => {
            if external_constant_name(ident) == Some(name) {
                replacement.clone()
            } else {
                expr.clone()
            }
        }
        Expr::Binary { op, lhs, rhs, span } => Expr::Binary {
            op: *op,
            lhs: Box::new(substitute(lhs, name, replacement)),
            rhs: Box::new(substitute(rhs, name, replacement)),
            span: span.clone(),
        },
        Expr::Polarity { op, operand, span } => Expr::Polarity {
            op: *op,
            operand: Box::new(substitute(operand, name, replacement)),
            span: span.clone(),
        },
        Expr::Type {
            lhs,
            op,
            type_spec,
            span,
        } => Expr::Type {
            lhs: Box::new(substitute(lhs, name, replacement)),
            op: *op,
            type_spec: type_spec.clone(),
            span: span.clone(),
        },
        Expr::Indexer {
            receiver,
            index,
            span,
        } => Expr::Indexer {
            receiver: Box::new(substitute(receiver, name, replacement)),
            index: Box::new(substitute(index, name, replacement)),
            span: span.clone(),
        },
        Expr::Invocation {
            receiver,
            call,
            span,
        } => {
            let new_receiver = receiver
                .as_ref()
                .map(|r| Box::new(substitute(r, name, replacement)));
            let new_call = match call {
                Invocation::Function {
                    name: fname,
                    args,
                    span: fspan,
                } => Invocation::Function {
                    name: fname.clone(),
                    args: args.iter().map(|a| substitute(a, name, replacement)).collect(),
                    span: fspan.clone(),
                },
                _ => call.clone(),
            };
            Expr::Invocation {
                receiver: new_receiver,
                call: new_call,
                span: span.clone(),
            }
        }
        Expr::Parenthesized { inner, span } => Expr::Parenthesized {
            inner: Box::new(substitute(inner, name, replacement)),
            span: span.clone(),
        },
        // Literals and ExternalConstant{non-matching} are returned as-is.
        Expr::Literal(_) => expr.clone(),
    }
}

fn external_constant_name(ident: &ExternalConstantId) -> Option<&str> {
    match ident {
        ExternalConstantId::Identifier(id) => Some(id.raw.as_str()),
        // ExternalConstantId::String stores the raw value INCLUDING the
        // surrounding quotes — strip them before comparing to the bare name.
        ExternalConstantId::String { raw, .. } => {
            if raw.len() >= 2 {
                Some(&raw[1..raw.len() - 1])
            } else {
                None
            }
        }
    }
}

// ── Expr → FHIRPath string ────────────────────────────────────────────

/// Serialize an `Expr` back to a valid FHIRPath expression string.
pub fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Binary { op, lhs, rhs, .. } => {
            let left = expr_to_string(lhs);
            let right = expr_to_string(rhs);
            format!("{left} {op} {right}", op = op.keyword())
        }
        Expr::Type { lhs, op, type_spec, .. } => {
            let left = expr_to_string(lhs);
            let ts = type_specifier_to_string(type_spec);
            format!("{left} {op} {ts}", op = op.keyword())
        }
        Expr::Polarity { op, operand, .. } => {
            let inner = expr_to_string(operand);
            format!("{op}{inner}", op = op.keyword())
        }
        Expr::Indexer { receiver, index, .. } => {
            let r = expr_to_string(receiver);
            let i = expr_to_string(index);
            format!("{r}[{i}]")
        }
        Expr::Invocation {
            receiver: Some(rcv),
            call,
            ..
        } => {
            let left = expr_to_string(rcv);
            let right = invocation_to_string(call);
            format!("{left}.{right}")
        }
        Expr::Invocation {
            receiver: None,
            call,
            ..
        } => invocation_to_string(call),
        Expr::Parenthesized { inner, .. } => {
            let s = expr_to_string(inner);
            format!("({s})")
        }
        Expr::Literal(lit) => literal_to_string(lit),
        Expr::ExternalConstant { ident, .. } => match ident {
            ExternalConstantId::Identifier(id) => format!("%{}", id.raw),
            ExternalConstantId::String { raw, .. } => format!("%{}", raw),
        },
    }
}

fn invocation_to_string(inv: &Invocation) -> String {
    match inv {
        Invocation::Member { ident } => ident.raw.clone(),
        Invocation::Function { name, args, .. } => {
            let arg_strs: Vec<String> = args.iter().map(expr_to_string).collect();
            format!("{}({})", name.raw, arg_strs.join(", "))
        }
        Invocation::This { .. } => "$this".to_string(),
        Invocation::Index { .. } => "$index".to_string(),
        Invocation::Total { .. } => "$total".to_string(),
    }
}

fn literal_to_string(lit: &Literal) -> String {
    match lit {
        Literal::Null { .. } => "{}".to_string(),
        Literal::Boolean { value, .. } => if *value { "true" } else { "false" }.to_string(),
        Literal::String { raw, .. } => raw.clone(),
        Literal::Number { raw, .. } => raw.clone(),
        Literal::DateTime { raw, .. } => raw.clone(),
        Literal::Time { raw, .. } => raw.clone(),
        Literal::Quantity { number, unit, .. } => {
            format!("{} {}", number, unit_to_string(unit))
        }
    }
}

fn unit_to_string(unit: &Unit) -> String {
    match unit {
        Unit::String { raw, .. } => raw.clone(),
        Unit::DateTimePrecision { word, .. } => word.clone(),
        Unit::PluralDateTimePrecision { word, .. } => word.clone(),
    }
}

fn type_specifier_to_string(ts: &TypeSpecifier) -> String {
    qualified_identifier_to_string(&ts.qualified)
}

fn qualified_identifier_to_string(qi: &QualifiedIdentifier) -> String {
    qi.parts
        .iter()
        .map(|i| i.raw.clone())
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(expr: &str) -> String {
        let ast = crate::parse(expr).unwrap();
        expr_to_string(&ast)
    }

    #[test]
    fn test_simple_path() {
        assert_eq!(roundtrip("Patient.name.given"), "Patient.name.given");
    }

    #[test]
    fn test_function_call() {
        assert_eq!(
            roundtrip("item.where(linkId = 'x')"),
            "item.where(linkId = 'x')"
        );
    }

    #[test]
    fn test_external_constant() {
        assert_eq!(roundtrip("%context"), "%context");
        assert_eq!(roundtrip("%resource"), "%resource");
        assert_eq!(roundtrip("%ucum"), "%ucum");
    }

    #[test]
    fn test_external_constant_chain() {
        assert_eq!(
            roundtrip("%context.item.where(linkId = 'x')"),
            "%context.item.where(linkId = 'x')"
        );
    }

    #[test]
    fn test_binary_ops() {
        assert_eq!(roundtrip("a + b"), "a + b");
        assert_eq!(roundtrip("a and b"), "a and b");
        assert_eq!(roundtrip("a or b"), "a or b");
        assert_eq!(roundtrip("a = b"), "a = b");
        assert_eq!(roundtrip("a != b"), "a != b");
    }

    #[test]
    fn test_unary() {
        assert_eq!(roundtrip("-1"), "-1");
    }

    #[test]
    fn test_parenthesized() {
        assert_eq!(roundtrip("(a + b)"), "(a + b)");
    }

    #[test]
    fn test_literals() {
        assert_eq!(roundtrip("true"), "true");
        assert_eq!(roundtrip("false"), "false");
        assert_eq!(roundtrip("42"), "42");
        assert_eq!(roundtrip("3.14"), "3.14");
        assert_eq!(roundtrip("'hello'"), "'hello'");
        assert_eq!(roundtrip("{}"), "{}");
        assert_eq!(roundtrip("@2024-01-01"), "@2024-01-01");
        assert_eq!(roundtrip("@T10:00:00"), "@T10:00:00");
    }

    #[test]
    fn test_quantity() {
        assert_eq!(roundtrip("10 'mg'"), "10 'mg'");
        assert_eq!(roundtrip("1 year"), "1 year");
        assert_eq!(roundtrip("2 days"), "2 days");
    }

    #[test]
    fn test_type_expression() {
        assert_eq!(roundtrip("x is String"), "x is String");
        assert_eq!(roundtrip("x as Integer"), "x as Integer");
    }

    #[test]
    fn test_indexer() {
        assert_eq!(roundtrip("a[0]"), "a[0]");
    }

    #[test]
    fn test_special_vars() {
        assert_eq!(roundtrip("$this"), "$this");
        assert_eq!(roundtrip("$index"), "$index");
        assert_eq!(roundtrip("$total"), "$total");
    }

    #[test]
    fn test_iif() {
        assert_eq!(
            roundtrip("iif(a, 'yes', 'no')"),
            "iif(a, 'yes', 'no')"
        );
    }

    #[test]
    fn test_resolve_context_simple() {
        let result = resolve_context(
            "%context.item.where(linkId = 'x').answer.value",
            "%resource.item.where(linkId = 'group')",
        )
        .unwrap();
        assert_eq!(
            result,
            "%resource.item.where(linkId = 'group').item.where(linkId = 'x').answer.value"
        );
    }

    #[test]
    fn test_resolve_context_filter() {
        let result = resolve_context(
            "%context.where(item.where(linkId = 'check').answer.value = true)",
            "%resource.item.where(linkId = 'section')",
        )
        .unwrap();
        assert_eq!(
            result,
            "%resource.item.where(linkId = 'section').where(item.where(linkId = 'check').answer.value = true)"
        );
    }

    #[test]
    fn test_resolve_context_in_function_arg() {
        let result = resolve_context(
            "iif(%context.x, 'a', 'b')",
            "%resource.item.where(linkId = 'q')",
        )
        .unwrap();
        assert_eq!(
            result,
            "iif(%resource.item.where(linkId = 'q').x, 'a', 'b')"
        );
    }

    #[test]
    fn test_resolve_context_multiple_refs() {
        let result = resolve_context(
            "%context.a + %context.b",
            "base",
        )
        .unwrap();
        assert_eq!(result, "base.a + base.b");
    }

    #[test]
    fn test_resolve_context_no_context() {
        let result = resolve_context(
            "%resource.item.where(linkId = 'x')",
            "anything",
        )
        .unwrap();
        // No %context reference → unchanged
        assert_eq!(result, "%resource.item.where(linkId = 'x')");
    }

    #[test]
    fn test_resolve_context_bare() {
        // %context alone (not chained)
        let result = resolve_context("%context", "base.path").unwrap();
        assert_eq!(result, "base.path");
    }

    #[test]
    fn test_resolve_context_chained() {
        // Multi-level nesting
        let level1 = "%resource.item.where(linkId = 'poliepen')";
        let level2 = resolve_context(
            "%context.item.where(linkId = 'poliep')",
            level1,
        )
        .unwrap();
        assert_eq!(
            level2,
            "%resource.item.where(linkId = 'poliepen').item.where(linkId = 'poliep')"
        );
        let level3 = resolve_context(
            "%context.where(item.where(linkId = 'resectie').answer.value ~ 'yes')",
            &level2,
        )
        .unwrap();
        assert_eq!(
            level3,
            "%resource.item.where(linkId = 'poliepen').item.where(linkId = 'poliep').where(item.where(linkId = 'resectie').answer.value ~ 'yes')"
        );
    }

    #[test]
    fn test_resolve_context_string_literal_not_substituted() {
        // %context inside a string literal should NOT be substituted
        // (the lexer treats 'some %context text' as a single StringLiteral token)
        let result = resolve_context("'%context'", "base").unwrap();
        assert_eq!(result, "'%context'");
    }
}
