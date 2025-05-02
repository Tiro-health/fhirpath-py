import pytest
import yaml

from fhirpathpy import evaluate
from fhirpathpy.engine.nodes import FP_Quantity
from fhirpathpy.models import models
from tests.resources import resources


def pytest_collect_file(parent, path):
    if path.ext == ".yaml":
        return YamlFile.from_parent(parent, fspath=path)


class YamlFile(pytest.File):
    def collect(self):
        raw = yaml.safe_load(self.fspath.open())

        suites = raw["tests"]
        subject = raw["subject"] if "subject" in raw else None

        return self.collect_tests(suites, subject)

    def is_group(self, test):
        if not isinstance(test, dict):
            return False

        return any(key.startswith("group") for key in test.keys())

    def collect_tests(self, suites, subject, is_group_disabled=False):
        for suite in suites:
            current_group_disabled = is_group_disabled or suite.get("disable", False)
            if self.is_group(suite):
                name = next(iter(suite))
                tests = suite[name]
                for test in self.collect_tests(tests, subject, current_group_disabled):
                    yield test
            else:
                for test in self.collect_test(suite, subject, current_group_disabled):
                    yield test

    def collect_test(self, test, subject, is_group_disabled):
        name = test["desc"] if "desc" in test else ""
        is_disabled = (
            is_group_disabled if is_group_disabled else "disable" in test and test["disable"]
        )

        if "expression" in test and not is_disabled:
            if isinstance(test["expression"], list):
                for expression in test["expression"]:
                    test["expression"] = expression
                    yield YamlItem.from_parent(
                        self,
                        name=name,
                        test=test,
                        resource=subject,
                    )
            else:
                yield YamlItem.from_parent(
                    self,
                    name=name,
                    test=test,
                    resource=subject,
                )


class YamlItem(pytest.Item):
    def __init__(self, name, parent, test, resource=None):
        super().__init__(name, parent)

        self.test = test
        self.resource = resource

    def runtest(self):
        expression = self.test["expression"]
        resource = self.resource

        model = models[self.test["model"]] if "model" in self.test else None

        if "inputfile" in self.test:
            if self.test["inputfile"] in resources:
                resource = resources[self.test["inputfile"]]

        variables = {"resource": resource}

        if "context" in self.test:
            variables["context"] = evaluate(resource, self.test["context"])[0]

        if "variables" in self.test:
            variables.update(self.test["variables"])

        if "error" in self.test and self.test["error"] is True:
            with pytest.raises(Exception) as exc:
                raise Exception(self.test["desc"]) from exc
        else:
            result = evaluate(resource, expression, variables, model)
            compare(result, self.test["result"])


def compare(l1, l2):
    # TODO REFACTOR
    if l1 == l2:
        assert True
    elif len(l1) == len(l2) == 1:
        e1 = l1[0]
        e2 = evaluate({}, l2[0])[0] if isinstance(l2[0], str) else l2[0]
        if isinstance(e1, FP_Quantity) and isinstance(e2, FP_Quantity):
            assert e1 == e2
        else:
            assert str(e1) == str(e2)
    else:
        raise AssertionError(f"{l1} != {l2}")
