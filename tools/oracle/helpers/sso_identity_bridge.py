import os
from collections.abc import Mapping
from types import MappingProxyType
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.management_endpoints.sso.agent_subject_enrollment import (
    microsoft_interactive_subject,
)
from litellm.proxy.management_endpoints.sso.id_jag_assertion_capture import (
    active_sso_provider,
    id_jag_assertion_capture_gap,
    id_jag_assertion_capture_gap_at_startup,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class ProviderInput(TypedDict):
    function: Literal["provider"]
    environment: dict[str, str]


class GapInput(TypedDict):
    function: Literal["gap"]
    environment: dict[str, str]


class StartupGapInput(TypedDict):
    function: Literal["startup_gap"]
    environment: dict[str, str]


class SubjectInput(TypedDict):
    function: Literal["subject"]
    environment: dict[str, str]
    tenant: str | None
    response_id: str | None
    endpoints: dict[str, str | None]


Input: TypeAlias = ProviderInput | GapInput | StartupGapInput | SubjectInput


def subject_projection(input_value: SubjectInput) -> JsonValue:
    response: Mapping[str, str | None] = MappingProxyType(
        {"id": input_value["response_id"]}
    )
    overrides: Mapping[str, str | None] = MappingProxyType(input_value["endpoints"])
    projected = microsoft_interactive_subject(
        input_value["tenant"], response, overrides
    )
    if projected is None:
        return None
    return JSON.validate_json(projected.model_dump_json())


def dispatch(input_value: Input) -> JsonValue:
    result: JsonValue = None
    match input_value["function"]:
        case "provider":
            result = active_sso_provider().value
        case "gap":
            result = id_jag_assertion_capture_gap()
        case "startup_gap":
            result = id_jag_assertion_capture_gap_at_startup()
        case "subject":
            result = subject_projection(input_value)
    return result


def run(input_value: Input) -> JsonValue:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        return dispatch(input_value)
    finally:
        os.environ.clear()
        os.environ.update(saved)
