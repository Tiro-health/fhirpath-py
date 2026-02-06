"""Generate golden AST fixtures from the ANTLR parser.

Run with: uv run python scripts/generate_golden_asts.py
"""

import json
import sys
from pathlib import Path

# Force ANTLR parser regardless of Rust availability
sys.modules.pop("fhirpathpy.parser", None)
from fhirpathpy.parser._antlr import parse

EXPRESSIONS = [
    # ── Identifiers / member invocation ──
    "x",
    "Patient",
    "_private",

    # ── Literals ──
    "true",
    "false",
    "{}",
    "'hello'",
    "'hello world'",
    "42",
    "3.14",
    "0",

    # DateTime literals
    "@2024",
    "@2024-01",
    "@2024-01-15",
    "@2024-01-15T10:30:00",
    "@2024-01-15T10:30:00Z",

    # Time literals
    "@T10:30",
    "@T10:30:00",

    # Quantity literals
    "10 'mg'",
    "5 days",
    "1 year",
    "2.5 'cm'",

    # ── External constants ──
    "%context",
    "%vs",

    # ── Parenthesized expressions ──
    "(1 + 2)",

    # ── Special invocations ──
    "$this",
    "$index",
    "$total",

    # ── Invocation expressions (dot access) ──
    "Patient.name",
    "Patient.name.given",
    "a.b.c.d",

    # ── Function invocations ──
    "name.exists()",
    "name.count()",
    "name.where(use = 'official')",
    "iif(a, b, c)",
    "children().name",

    # ── Indexer expressions ──
    "name[0]",
    "a.b[0]",
    "a[0].b",

    # ── Polarity expressions ──
    "-5",
    "+5",
    "-a",

    # ── Multiplicative expressions ──
    "a * b",
    "a / b",
    "a div b",
    "a mod b",
    "10 * 3",

    # ── Additive expressions ──
    "a + b",
    "a - b",
    "a & b",
    "1 + 2",

    # ── Union expressions ──
    "a | b",
    "a | b | c",

    # ── Inequality expressions ──
    "a < b",
    "a > b",
    "a <= b",
    "a >= b",
    "count() > 0",

    # ── Type expressions ──
    "value is string",
    "value as string",
    "value is FHIR.string",

    # ── Equality expressions ──
    "a = b",
    "a != b",
    "a ~ b",
    "a !~ b",
    "name = 'John'",

    # ── Membership expressions ──
    "a in b",
    "a contains b",

    # ── Boolean expressions ──
    "a and b",
    "a or b",
    "a xor b",
    "a implies b",
    "a and b and c",
    "a or b or c",

    # ── Precedence tests ──
    "(a + b) * c",
    "a + b * c",
    "a or b and c",

    # ── Complex / combined expressions ──
    "Patient.name.where(use = 'official').given.first()",
    "name.exists() and age > 18",
    "Patient.name | Patient.alias",
    "iif(a > b, a, b)",
    "%resource.id",
    "Observation.component.where(code.coding.code = '8480-6').value",

    # ── Delimited identifier ──
    "`div`",

    # ── String concatenation ──
    "'hello' & ' ' & 'world'",

    # ── Nested function calls ──
    "a.where(b.exists())",
    "a.select(b + c)",

    # ── as/is used as identifiers ──
    "Patient.as",
    "Patient.is",
    "Patient.contains",
    "Patient.in",
]

OUTPUT = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "ast" / "golden_asts.json"


def main() -> None:
    results: dict[str, dict] = {}
    errors: list[str] = []

    for expr in EXPRESSIONS:
        try:
            ast = parse(expr)
            results[expr] = ast
        except Exception as e:
            errors.append(f"  FAIL: {expr!r} -> {e}")

    if errors:
        print("Errors parsing some expressions:", file=sys.stderr)
        for err in errors:
            print(err, file=sys.stderr)
        sys.exit(1)

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    with open(OUTPUT, "w") as f:
        json.dump(results, f, indent=2)

    print(f"Generated {len(results)} golden AST fixtures -> {OUTPUT}")


if __name__ == "__main__":
    main()
