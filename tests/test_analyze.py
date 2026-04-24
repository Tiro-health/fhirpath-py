"""Integration tests for the analysis API exposed via PyO3."""

import json

import pytest

from fhirpathrs import annotate_expression, analyze_expression, QuestionnaireIndex


QUESTIONNAIRE_JSON = json.dumps(
    {
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
                            {
                                "valueCoding": {
                                    "system": "http://example.com",
                                    "code": "A",
                                    "display": "Alpha",
                                }
                            },
                            {
                                "valueCoding": {
                                    "system": "http://example.com",
                                    "code": "B",
                                    "display": "Beta",
                                }
                            },
                        ],
                    },
                    {"linkId": "bool1", "text": "Yes or no", "type": "boolean"},
                    {
                        "linkId": "subgroup",
                        "type": "group",
                        "item": [{"linkId": "deep", "type": "string"}],
                    },
                ],
            },
            {
                "linkId": "group2",
                "type": "group",
                "item": [{"linkId": "other", "type": "decimal"}],
            },
        ],
    }
)


# ── Import smoke test ──────────────────────────────────────────────────


def imports_test():
    """All analysis symbols are importable from the top-level package."""
    from fhirpathrs import annotate_expression, analyze_expression, QuestionnaireIndex  # noqa: F811

    assert callable(annotate_expression)
    assert callable(analyze_expression)
    assert QuestionnaireIndex is not None


# ── QuestionnaireIndex ─────────────────────────────────────────────────


def questionnaire_index_construction_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    assert idx.contains("group1")
    assert idx.contains("choice1")
    assert not idx.contains("nonexistent")


def questionnaire_index_resolve_item_text_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    assert idx.resolve_item_text("group1") == "Group One"
    assert idx.resolve_item_text("choice1") == "Pick one"
    assert idx.resolve_item_text("nonexistent") is None


def questionnaire_index_resolve_code_display_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    assert idx.resolve_code_display("choice1", "http://example.com", "A") == "Alpha"
    assert idx.resolve_code_display("choice1", "http://example.com", "B") == "Beta"
    assert idx.resolve_code_display("choice1", "http://example.com", "C") is None
    assert idx.resolve_code_display("bool1", "http://example.com", "A") is None


def questionnaire_index_invalid_json_test():
    with pytest.raises(ValueError):
        QuestionnaireIndex("not valid json")


# ── annotate_expression ────────────────────────────────────────────────


def annotate_answer_reference_test():
    result = annotate_expression("item.where(linkId='x').answer.value.code")
    assert isinstance(result, list)
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "answer_reference"
    assert ann["link_ids"] == ["x"]
    assert ann["accessor"] == "code"
    assert ann["start"] == 0
    assert ann["end"] == len("item.where(linkId='x').answer.value.code")


def annotate_item_reference_test():
    result = annotate_expression("item.where(linkId='group1')")
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "item_reference"
    assert ann["link_ids"] == ["group1"]


def annotate_coded_value_test():
    result = annotate_expression("item.where(linkId='x').answer.value.code = 'yes'")
    assert len(result) == 2
    kinds = {a["kind"] for a in result}
    assert "answer_reference" in kinds
    assert "coded_value" in kinds
    coded = next(a for a in result if a["kind"] == "coded_value")
    assert coded["code"] == "yes"
    assert coded["system"] is None
    assert coded["context_link_id"] == "x"


def annotate_factory_coding_test():
    expr = "item.where(linkId='x').answer.value ~ %factory.Coding('http://snomed.info/sct', '373067005')"
    result = annotate_expression(expr)
    coded = next(a for a in result if a["kind"] == "coded_value")
    assert coded["code"] == "373067005"
    assert coded["system"] == "http://snomed.info/sct"


def annotate_non_qr_expression_test():
    result = annotate_expression("Patient.name.given")
    assert result == []


def annotate_nested_item_navigation_test():
    result = annotate_expression(
        "item.where(linkId='a').item.where(linkId='b').answer.value.code"
    )
    assert len(result) == 1
    assert result[0]["link_ids"] == ["a", "b"]
    assert result[0]["accessor"] == "code"


def annotate_value_accessor_test():
    bare = annotate_expression("item.where(linkId='x').answer.value")
    assert bare[0]["accessor"] == "value"

    display = annotate_expression("item.where(linkId='x').answer.value.display")
    assert display[0]["accessor"] == "display"


def annotate_syntax_error_test():
    with pytest.raises(SyntaxError):
        annotate_expression("!")


# ── annotate_expression: positional selectors (Phase 1) ────────────────


def annotate_positional_after_where_demotes_attribution_test():
    result = annotate_expression("item.where(linkId='x').first().answer.value")
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "answer_reference"
    assert ann["link_ids"] == ["x"]
    assert ann["accessor"] == "value"
    assert ann["attribution"] == "partial_positional"


def annotate_positional_after_value_demotes_attribution_test():
    result = annotate_expression("item.where(linkId='x').answer.value.first()")
    assert len(result) == 1
    assert result[0]["attribution"] == "partial_positional"


def annotate_indexer_after_where_demotes_item_ref_test():
    result = annotate_expression("item.where(linkId='x')[0]")
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "item_reference"
    assert ann["link_ids"] == ["x"]
    assert ann["attribution"] == "partial_positional"


def annotate_full_attribution_not_emitted_for_clean_chain_test():
    # Wire compat: dict shape is byte-identical to v3.0.0 when attribution is Full.
    result = annotate_expression("item.where(linkId='x').answer.value.code")
    assert "attribution" not in result[0]


# ── annotate_expression: context variables & composite positional (Phase 3)


def annotate_this_dot_link_id_in_where_test():
    # `$this.linkId = 'x'` inside where() is equivalent to `linkId = 'x'`.
    result = annotate_expression("item.where($this.linkId = 'x').answer.value")
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "answer_reference"
    assert ann["link_ids"] == ["x"]
    # Full attribution preserved — wire shape omits the attribution key.
    assert "attribution" not in ann


def annotate_skip_zero_take_one_demotes_test():
    result = annotate_expression("item.where(linkId='x').answer.skip(0).take(1).value")
    assert len(result) == 1
    assert result[0]["attribution"] == "partial_positional"


def annotate_skip_one_take_two_preserves_attribution_test():
    # `.skip(1).take(2)` leaves cardinality at Many → attribution stays Full.
    result = annotate_expression("item.where(linkId='x').answer.skip(1).take(2).value")
    assert len(result) == 1
    assert "attribution" not in result[0]


def annotate_take_one_demotes_test():
    result = annotate_expression("item.where(linkId='x').answer.take(1).value")
    assert len(result) == 1
    assert result[0]["attribution"] == "partial_positional"


def annotate_take_many_is_transparent_test():
    result = annotate_expression("item.where(linkId='x').answer.take(5).value")
    assert len(result) == 1
    assert "attribution" not in result[0]


def annotate_skip_alone_is_transparent_test():
    result = annotate_expression("item.where(linkId='x').answer.skip(3).value")
    assert len(result) == 1
    assert "attribution" not in result[0]


# ── annotate_expression: widened scope & unattributable (Phase 2) ──────


def annotate_descendants_widens_attribution_test():
    result = annotate_expression(
        "QuestionnaireResponse.descendants().where(linkId='x').answer.value"
    )
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "answer_reference"
    assert ann["link_ids"] == ["x"]
    assert ann["attribution"] == "widened_scope"


def annotate_children_produces_widened_item_ref_test():
    result = annotate_expression("item.where(linkId='group1').children()")
    assert len(result) == 1
    ann = result[0]
    assert ann["kind"] == "item_reference"
    assert ann["link_ids"] == ["group1"]
    assert ann["attribution"] == "widened_scope"


def annotate_opaque_where_yields_unattributable_test():
    result = annotate_expression(
        "item.where(linkId='x').answer.where(value.code = 'yes')"
        ".item.where(linkId='y').answer.value"
    )
    # LinkIds are preserved even though attribution is tainted.
    ans_refs = [a for a in result if a["kind"] == "answer_reference"]
    assert len(ans_refs) == 1
    assert ans_refs[0]["link_ids"] == ["x", "y"]
    assert ans_refs[0]["attribution"] == "unattributable"


def annotate_iif_without_terminal_is_not_annotated_test():
    result = annotate_expression("item.iif(linkId='x', answer.value, 'fallback')")
    assert result == []


# ── analyze_expression: validator gating on degraded attribution ──────


def analyze_widened_skips_type_validation_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    # Direct path hits invalid_accessor_for_type.
    direct = analyze_expression(
        "item.where(linkId='bool1').answer.value.code", idx
    )
    assert any(d["code"] == "invalid_accessor_for_type" for d in direct["diagnostics"])

    # Via descendants() — WidenedScope — type check must skip.
    widened = analyze_expression(
        "QuestionnaireResponse.descendants().where(linkId='bool1').answer.value.code",
        idx,
    )
    assert all(
        d["code"] != "invalid_accessor_for_type" for d in widened["diagnostics"]
    )


def analyze_widened_skips_context_reachability_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    # Without descendants(), group2 is unreachable from group1 parent scope.
    unreachable = analyze_expression(
        "item.where(linkId='group2')",
        idx,
        parent_context_expr="%resource.item.where(linkId='group1')",
    )
    assert any(
        d["code"] == "context_unreachable_from_parent"
        for d in unreachable["diagnostics"]
    )

    # With descendants() on the target side, attribution is WidenedScope —
    # reachability check must be skipped.
    via_descendants = analyze_expression(
        "QuestionnaireResponse.descendants().where(linkId='group2')",
        idx,
        parent_context_expr="%resource.item.where(linkId='group1')",
    )
    assert all(
        d["code"] != "context_unreachable_from_parent"
        for d in via_descendants["diagnostics"]
    )


# ── analyze_expression ─────────────────────────────────────────────────


def analyze_clean_expression_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression("item.where(linkId='choice1').answer.value.code", idx)
    assert isinstance(result, dict)
    assert "annotations" in result
    assert "diagnostics" in result
    assert len(result["annotations"]) > 0
    assert len(result["diagnostics"]) == 0


def analyze_unknown_link_id_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression("item.where(linkId='typo').answer.value", idx)
    diags = result["diagnostics"]
    assert any(d["code"] == "unknown_link_id" for d in diags)
    diag = next(d for d in diags if d["code"] == "unknown_link_id")
    assert diag["severity"] == "error"
    assert isinstance(diag["message"], str)
    assert isinstance(diag["start"], int)
    assert isinstance(diag["end"], int)


def analyze_unreachable_link_id_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression(
        "item.where(linkId='other').answer.value",
        idx,
        context_link_id="group1",
    )
    diags = result["diagnostics"]
    assert any(d["code"] == "unreachable_link_id" for d in diags)


def analyze_invalid_accessor_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression("item.where(linkId='bool1').answer.value.code", idx)
    diags = result["diagnostics"]
    assert any(d["code"] == "invalid_accessor_for_type" for d in diags)


def analyze_non_qr_expression_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression("Patient.name.given", idx)
    assert result["annotations"] == []
    assert result["diagnostics"] == []


def analyze_with_parent_context_unreachable_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression(
        "item.where(linkId='group2')",
        idx,
        parent_context_expr="%resource.item.where(linkId='group1')",
    )
    diags = result["diagnostics"]
    assert any(d["code"] == "context_unreachable_from_parent" for d in diags)


def analyze_with_parent_context_reachable_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression(
        "item.where(linkId='subgroup')",
        idx,
        parent_context_expr="%resource.item.where(linkId='group1')",
    )
    diags = result["diagnostics"]
    assert not any(d["code"] == "context_unreachable_from_parent" for d in diags)


def analyze_item_reference_targets_leaf_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression(
        "item.where(linkId='group1').item.where(linkId='bool1')",
        idx,
    )
    diags = result["diagnostics"]
    assert any(d["code"] == "item_reference_targets_leaf" for d in diags)


def analyze_syntax_error_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    with pytest.raises(SyntaxError):
        analyze_expression("!", idx)


def analyze_expression_not_attributable_test():
    idx = QuestionnaireIndex(QUESTIONNAIRE_JSON)
    result = analyze_expression("item[0].answer.value", idx)
    diags = result["diagnostics"]
    assert any(d["code"] == "expression_not_attributable" for d in diags)
    diag = next(d for d in diags if d["code"] == "expression_not_attributable")
    assert diag["severity"] == "info"
    # No annotation because the chain is unattributable.
    assert result["annotations"] == []
