from typing import Final, Literal, TypedDict

from litellm.proxy.compliance_checks import ComplianceChecker
from litellm.types.proxy.compliance_endpoints import (
    ComplianceCheckRequest,
    ComplianceResponse,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class Input(TypedDict):
    regulation: Literal["eu-ai-act", "gdpr"]
    request: dict[str, JsonValue]


def run(input_value: Input) -> JsonValue:
    request = ComplianceCheckRequest.model_validate(input_value["request"])
    checker = ComplianceChecker(request)
    checks = (
        checker.check_eu_ai_act()
        if input_value["regulation"] == "eu-ai-act"
        else checker.check_gdpr()
    )
    regulation = "EU AI Act" if input_value["regulation"] == "eu-ai-act" else "GDPR"
    response = ComplianceResponse(
        compliant=all(check.passed for check in checks),
        regulation=regulation,
        checks=checks,
    )
    return JSON.validate_json(response.model_dump_json())
