/// AST-level resolution of `%context` references in FHIRPath expressions.
///
/// `resolve_context` parses both expressions, substitutes every `%context`
/// reference in `expr` with the parsed `base` AST, then serializes the
/// result back to a valid FHIRPath string.

use crate::parser::AstNode;
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
    Ok(ast_to_string(&resolved))
}

/// AST-level substitution: replace every `%context` node in `expr_ast` with
/// a clone of `base_ast`.
pub fn resolve_context_ast(expr_ast: &AstNode, base_ast: &AstNode) -> AstNode {
    substitute(expr_ast, "context", base_ast)
}

// ── Substitution ───────────────────────────────────────────────────────

/// Recursively walk `node`, replacing any `TermExpression` that wraps
/// `ExternalConstantTerm → ExternalConstant → Identifier(name)` with a
/// clone of `replacement`.
fn substitute(node: &AstNode, name: &str, replacement: &AstNode) -> AstNode {
    if is_external_constant_term_expr(node, name) {
        return replacement.clone();
    }

    let mut result = node.clone();
    result.children = node
        .children
        .iter()
        .map(|child| substitute(child, name, replacement))
        .collect();
    result
}

/// Check whether `node` is `TermExpression > ExternalConstantTerm >
/// ExternalConstant > Identifier` with the given constant name.
fn is_external_constant_term_expr(node: &AstNode, name: &str) -> bool {
    if node.node_type != "TermExpression" {
        return false;
    }
    let Some(ect) = node.children.first() else { return false };
    if ect.node_type != "ExternalConstantTerm" {
        return false;
    }
    let Some(ec) = ect.children.first() else { return false };
    if ec.node_type != "ExternalConstant" {
        return false;
    }
    let Some(ident) = ec.children.first() else { return false };
    if ident.node_type != "Identifier" {
        return false;
    }
    ident.terminal_node_text.first().map(|s| s.as_str()) == Some(name)
}

// ── AST → FHIRPath string ─────────────────────────────────────────────

/// Serialize an `AstNode` back to a valid FHIRPath expression string.
pub fn ast_to_string(node: &AstNode) -> String {
    match node.node_type {
        // ── Binary expressions ──────────────────────────────────────
        "ImpliesExpression" | "OrExpression" | "AndExpression"
        | "MembershipExpression" | "EqualityExpression"
        | "InequalityExpression" | "UnionExpression" | "AdditiveExpression"
        | "MultiplicativeExpression" => {
            // children: [left, right], terminal_node_text: [operator]
            let left = ast_to_string(&node.children[0]);
            let right = ast_to_string(&node.children[1]);
            let op = &node.terminal_node_text[0];
            format!("{left} {op} {right}")
        }

        // ── Type expression (is/as) ────────────────────────────────
        "TypeExpression" => {
            let left = ast_to_string(&node.children[0]);
            let type_spec = ast_to_string(&node.children[1]);
            let op = &node.terminal_node_text[0];
            format!("{left} {op} {type_spec}")
        }

        // ── Unary ──────────────────────────────────────────────────
        "PolarityExpression" => {
            let op = &node.terminal_node_text[0];
            let operand = ast_to_string(&node.children[0]);
            format!("{op}{operand}")
        }

        // ── Postfix ────────────────────────────────────────────────
        "InvocationExpression" => {
            let left = ast_to_string(&node.children[0]);
            let right = ast_to_string(&node.children[1]);
            format!("{left}.{right}")
        }

        "IndexerExpression" => {
            let left = ast_to_string(&node.children[0]);
            let index = ast_to_string(&node.children[1]);
            format!("{left}[{index}]")
        }

        // ── Terms (single-child wrappers) ──────────────────────────
        "TermExpression" | "LiteralTerm" | "InvocationTerm"
        | "ExternalConstantTerm" | "QuantityLiteral" | "TypeSpecifier" => {
            ast_to_string(&node.children[0])
        }

        // ── Parenthesized ──────────────────────────────────────────
        "ParenthesizedTerm" => {
            let inner = ast_to_string(&node.children[0]);
            format!("({inner})")
        }

        // ── External constant (%name) ──────────────────────────────
        "ExternalConstant" => {
            let ident = ast_to_string(&node.children[0]);
            format!("%{ident}")
        }

        // ── Invocations ────────────────────────────────────────────
        "MemberInvocation" => ast_to_string(&node.children[0]),

        "FunctionInvocation" => ast_to_string(&node.children[0]),

        "Functn" => {
            // children[0] = Identifier, children[1] = optional ParamList
            let name = ast_to_string(&node.children[0]);
            if node.children.len() > 1 {
                let params = ast_to_string(&node.children[1]);
                format!("{name}({params})")
            } else {
                format!("{name}()")
            }
        }

        "ParamList" => {
            // children = [expr, expr, ...], separated by commas
            node.children
                .iter()
                .map(ast_to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }

        // ── Special invocations ────────────────────────────────────
        "ThisInvocation" | "IndexInvocation" | "TotalInvocation" => {
            node.terminal_node_text[0].clone()
        }

        // ── Identifier ─────────────────────────────────────────────
        "Identifier" => node.terminal_node_text[0].clone(),

        // ── Qualified identifier (for type specifiers) ─────────────
        "QualifiedIdentifier" => {
            node.children
                .iter()
                .map(ast_to_string)
                .collect::<Vec<_>>()
                .join(".")
        }

        // ── Literals ───────────────────────────────────────────────
        "NullLiteral" => "{}".to_string(),

        "BooleanLiteral" | "StringLiteral" | "NumberLiteral"
        | "DateTimeLiteral" | "TimeLiteral" => {
            node.terminal_node_text[0].clone()
        }

        // ── Quantity (number + unit) ───────────────────────────────
        "Quantity" => {
            let number = &node.terminal_node_text[0];
            let unit = ast_to_string(&node.children[0]);
            format!("{number} {unit}")
        }

        "Unit" => {
            if !node.children.is_empty() {
                // DateTimePrecision or PluralDateTimePrecision
                ast_to_string(&node.children[0])
            } else {
                // String-literal unit (e.g. 'mg')
                node.terminal_node_text[0].clone()
            }
        }

        "DateTimePrecision" | "PluralDateTimePrecision" => {
            node.terminal_node_text[0].clone()
        }

        // Fallback — shouldn't happen with a well-formed AST
        _ => {
            eprintln!("ast_to_string: unknown node type {:?}", node.node_type);
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(expr: &str) -> String {
        let ast = crate::parse(expr).unwrap();
        ast_to_string(&ast)
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
