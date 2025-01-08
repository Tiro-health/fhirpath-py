from dataclasses import dataclass, fields
from typing import Literal

import pytest

from fhirpathpy import evaluate
from fhirpathpy.models import models


@dataclass
class Answer:
    valueString: str

    def __iter__(self):
        return iter([field.name for field in fields(self)])

    def __getitem__(self, key: str):
        try:
            return getattr(self, key)
        except AttributeError:
            raise KeyError(key)

    def __len__(self):
        return len(fields(self))


@dataclass
class QuestionnaireResponseItem:
    linkId: str
    item: list["QuestionnaireResponseItem"]
    answer: list[Answer]

    def __iter__(self):
        return iter([field.name for field in fields(self)])

    def __getitem__(self, key: str):
        try:
            return getattr(self, key)
        except AttributeError:
            raise KeyError(key)

    def __len__(self):
        return len(fields(self))


@dataclass
class QuestionnaireResponse:
    item: list[QuestionnaireResponseItem]
    resourceType: Literal["QuestionnaireResponse"] = "QuestionnaireResponse"

    def __iter__(self):
        return iter([field.name for field in fields(self)])

    def __getitem__(self, key: str):
        try:
            return getattr(self, key)
        except AttributeError:
            raise KeyError(key)

    def __len__(self):
        return len(fields(self))


@pytest.mark.parametrize(
    "expression, data, expected",
    [
        (
            "QuestionnaireResponse.item[0].item[0].answer.value",
            QuestionnaireResponse(
                item=[
                    QuestionnaireResponseItem(
                        linkId="1",
                        item=[
                            QuestionnaireResponseItem(
                                linkId="1.1",
                                item=[],
                                answer=[Answer(valueString="foo")],
                            )
                        ],
                        answer=[Answer(valueString="bar")],
                    )
                ]
            ),
            ["foo"],
        ),
        (
            "QuestionnaireResponse.item[0].item[0].answer.value",
            dict(
                resourceType="QuestionnaireResponse",
                item=[
                    dict(
                        linkId="1",
                        item=[
                            dict(
                                linkId="1.1",
                                item=[],
                                answer=[dict(valueString="foo")],
                            )
                        ],
                        answer=[dict(valueString="bar")],
                    )
                ],
            ),
            ["foo"],
        ),
    ],
)
def polymorhic_field_test(expression, data, expected):
    result = evaluate(data, expression, model=models["r5"])
    assert result == expected
