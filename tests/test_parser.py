import json
from pathlib import Path

import pytest
from antlr4.error.Errors import LexerNoViableAltException

from fhirpathpy.parser import parse

ast_fixtures_path = Path(__file__).resolve().parent / "fixtures" / "ast"


def load_ast_fixture(fixture_name):
    fixture_path = ast_fixtures_path / (fixture_name + ".json")
    with open(fixture_path) as f:
        return json.load(f)


def load_golden_asts():
    with open(ast_fixtures_path / "golden_asts.json") as f:
        return json.load(f)


GOLDEN_ASTS = load_golden_asts()


@pytest.mark.parametrize(
    "expression",
    [
        "4+4",
        "object",
        "object.method()",
        "object.method(42)",
        "object.property",
        "object.property.method()",
        "object.property.method(42)",
    ],
)
def parse_valid_test(expression):
    assert parse(expression) != {}


def parse_non_valid_test():
    with pytest.raises(LexerNoViableAltException):
        parse("!")


@pytest.mark.parametrize(
    "expression",
    [
        "%v+2",
        "a.b+2",
        "Observation.value",
        "Patient.name.given",
    ],
)
def output_correct_ast_test(expression):
    expected = load_ast_fixture(expression)
    assert json.dumps(parse(expression), sort_keys=True) == json.dumps(expected, sort_keys=True)


@pytest.mark.parametrize("expression", list(GOLDEN_ASTS.keys()))
def golden_ast_test(expression):
    expected = GOLDEN_ASTS[expression]
    actual = parse(expression)
    assert json.dumps(actual, sort_keys=True) == json.dumps(expected, sort_keys=True), (
        f"AST mismatch for {expression!r}"
    )
