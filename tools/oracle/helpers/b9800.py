import json
from typing import TypedDict

from litellm.proxy.route_llm_request import (
    MockTestingParamsDisabledError,
    ProxyMissingRequiredParamError,
    raise_if_mock_testing_params_disallowed,
    raise_if_required_body_param_missing,
)
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    gate: str
    route_type: str
    data: dict[str, JsonValue]
    allowed: bool


def required_params(route_type: str, data: dict[str, JsonValue]) -> JsonValue:
    try:
        raise_if_required_body_param_missing(route_type=route_type, data=data)
    except ProxyMissingRequiredParamError as error:
        answer: dict[str, JsonValue] = {
            "message": error.message,
            "type": error.type,
            "param": error.param,
            "code": error.code,
        }
        return answer
    return None


def mock_params(data: dict[str, JsonValue], *, allowed: bool) -> JsonValue:
    try:
        raise_if_mock_testing_params_disallowed(data, allowed=allowed)
    except MockTestingParamsDisabledError as error:
        detail = TypeAdapter(dict[str, JsonValue]).validate_json(json.dumps(error.detail))
        answer: dict[str, JsonValue] = {"status": error.status_code, "detail": detail}
        return answer
    return None


def run(input_value: Input) -> JsonValue:
    if input_value["gate"] == "required_params":
        return required_params(input_value["route_type"], input_value["data"])
    return mock_params(input_value["data"], allowed=input_value["allowed"])
