import json
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.agent_endpoints import agent_registry
from litellm.proxy.agent_endpoints.agent_registry import _restore_redacted_litellm_params  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/agent_endpoints/agent_registry.py#L389-L416
from pydantic import JsonValue, TypeAdapter, ValidationError


class RedactCall(TypedDict):
    op: Literal["redact"]
    params: JsonValue


class RedactTextCall(TypedDict):
    op: Literal["redact_text"]
    text: str


class ParseCall(TypedDict):
    op: Literal["parse"]
    value: JsonValue


class RestoreCall(TypedDict):
    op: Literal["restore"]
    incoming: dict[str, JsonValue]
    existing: dict[str, JsonValue]


Call: TypeAlias = RedactCall | RedactTextCall | ParseCall | RestoreCall

CALL: Final[TypeAdapter[Call]] = TypeAdapter(Call)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)


def redacted(params: JsonValue) -> JsonValue:
    result = agent_registry.redact_sensitive_agent_litellm_params(params)
    return JSON.validate_json(json.dumps(result))


def parsed_text(text: str) -> JsonValue:
    produced = TEXT.validate_python(
        agent_registry.redact_sensitive_agent_litellm_params(text)
    )
    try:
        parsed = JSON.validate_json(produced)
    except ValidationError:
        return produced
    return parsed


def parsed_params(value: JsonValue) -> JsonValue:
    parsed = agent_registry.parse_agent_litellm_params(value)
    return JSON.validate_json(json.dumps(parsed, default=dict))


def restored(incoming: dict[str, JsonValue], existing: dict[str, JsonValue]) -> JsonValue:
    result = _restore_redacted_litellm_params(incoming, existing)
    return JSON.validate_json(json.dumps(result))


def run(input_value: JsonValue) -> JsonValue:
    call = CALL.validate_python(input_value)
    if call["op"] == "redact":
        return redacted(call["params"])
    if call["op"] == "redact_text":
        return parsed_text(call["text"])
    if call["op"] == "parse":
        return parsed_params(call["value"])
    return restored(call["incoming"], call["existing"])
