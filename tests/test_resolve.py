"""Tests for resolve_context — AST-level %context substitution."""

import pytest

from fhirpathrs import resolve_context


class TestResolveContextBasic:
    """Basic substitution cases."""

    def bare_context_test(self):
        assert resolve_context("%context", "base.path") == "base.path"

    def context_chain_test(self):
        result = resolve_context(
            "%context.item.where(linkId = 'x').answer.value",
            "%resource.item.where(linkId = 'group')",
        )
        assert (
            result
            == "%resource.item.where(linkId = 'group').item.where(linkId = 'x').answer.value"
        )

    def context_filter_test(self):
        result = resolve_context(
            "%context.where(item.where(linkId = 'check').answer.value = true)",
            "%resource.item.where(linkId = 'section')",
        )
        assert result == (
            "%resource.item.where(linkId = 'section')"
            ".where(item.where(linkId = 'check').answer.value = true)"
        )


class TestResolveContextFunctionArgs:
    """Substitution inside function arguments."""

    def iif_test(self):
        result = resolve_context(
            "iif(%context.x, 'a', 'b')",
            "%resource.item.where(linkId = 'q')",
        )
        assert result == "iif(%resource.item.where(linkId = 'q').x, 'a', 'b')"


class TestResolveContextMultipleRefs:
    """Multiple %context references in a single expression."""

    def additive_test(self):
        result = resolve_context("%context.a + %context.b", "base")
        assert result == "base.a + base.b"


class TestResolveContextNoop:
    """Expressions without %context are returned unchanged."""

    def no_context_test(self):
        expr = "%resource.item.where(linkId = 'x')"
        assert resolve_context(expr, "anything") == expr

    def other_external_constant_test(self):
        expr = "%ucum"
        assert resolve_context(expr, "base") == "%ucum"


class TestResolveContextChained:
    """Multi-level nesting (resolve output fed back as base)."""

    def three_levels_test(self):
        level1 = "%resource.item.where(linkId = 'poliepen')"
        level2 = resolve_context(
            "%context.item.where(linkId = 'poliep')", level1
        )
        assert level2 == (
            "%resource.item.where(linkId = 'poliepen')"
            ".item.where(linkId = 'poliep')"
        )
        level3 = resolve_context(
            "%context.where(item.where(linkId = 'resectie').answer.value ~ 'yes')",
            level2,
        )
        assert level3 == (
            "%resource.item.where(linkId = 'poliepen')"
            ".item.where(linkId = 'poliep')"
            ".where(item.where(linkId = 'resectie').answer.value ~ 'yes')"
        )


class TestResolveContextStringLiterals:
    """String literals containing '%context' must NOT be substituted."""

    def string_literal_test(self):
        assert resolve_context("'%context'", "base") == "'%context'"

    def string_literal_in_where_test(self):
        result = resolve_context(
            "item.where(text = '%context is not replaced')", "base"
        )
        assert result == "item.where(text = '%context is not replaced')"


class TestResolveContextErrors:
    """Invalid expressions should raise SyntaxError."""

    def bad_expr_test(self):
        with pytest.raises(SyntaxError):
            resolve_context("!!!invalid", "base")

    def bad_base_test(self):
        with pytest.raises(SyntaxError):
            resolve_context("%context", "!!!invalid")
