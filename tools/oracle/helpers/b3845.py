import json
import os
from collections.abc import Callable, Mapping, Sequence
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.opentelemetry import OpenTelemetry
from litellm.integrations.otel.mappers import genai, legacy, utils
from litellm.integrations.otel.mappers.base import AttributeMap, AttributeMapper
from litellm.integrations.otel.model import payloads
from litellm.types.services import ServiceLoggerPayload, ServiceTypes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
MapperClass: TypeAlias = Callable[..., AttributeMapper]

if TYPE_CHECKING:
    from litellm.types.utils import (
        StandardLoggingGuardrailInformation,
        StandardLoggingPayload,
    )

_DATABASE_ENVIRONMENT: tuple[str, ...] = ("DATABASE_URL", "DATABASE_URL_READ_REPLICA")
_LOGGER = OpenTelemetry.__new__(OpenTelemetry)


def run(input_value: dict[str, Json]) -> Json:
    return {"result": dispatch(input_value)}


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "genai_llm_call":
        result = mapper_record(input_value, genai.GenAIMapper)
    elif function == "genai_guardrail":
        result = guardrail_record(input_value)
    elif function == "genai_service":
        result = service_record(input_value)
    elif function == "genai_mcp_tool_call":
        result = tool_call_record(input_value)
    elif function == "genai_mcp_list_tools":
        result = list_tools_record(input_value)
    elif function == "legacy_llm_call":
        result = mapper_record(input_value, legacy.LegacyMapper)
    elif function == "tool_definition_budget":
        result = budget_record(input_value)
    elif function == "messages":
        result = messages_record(input_value)
    elif function == "responses_api_output":
        result = responses_output_record(input_value)
    else:
        raise ValueError(f"unknown LiteLLM helper function {function} ({litellm.__name__})")
    return result


def mapper_record(input_value: dict[str, Json], mapper_class: MapperClass) -> Json:
    span = llm_call_span(input_value)
    mapper = mapper_class(tool_attr_budget=tool_budget_value(input_value))
    mapped: AttributeMap = mapper.map(span)
    return attributes_record(cast(Mapping[str, object], mapped))


def llm_call_span(input_value: dict[str, Json]) -> payloads.LLMCallSpanData:
    payload = object_value(input_value.get("payload"), "payload")
    return payloads.LLMCallSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload),
        capture_content=bool_value(input_value.get("capture_content")),
        time_to_first_chunk_seconds=optional_float(
            input_value.get("time_to_first_chunk_seconds")
        ),
        request_route=optional_string(input_value.get("request_route")),
        session_id=optional_string(input_value.get("session_id")),
    )


def tool_budget_value(input_value: dict[str, Json]) -> int:
    raw = input_value.get("tool_attr_budget")
    if raw is None:
        return utils.MAX_TOOL_DEFINITION_ATTRS_PER_SPAN
    return integer_value(raw, "tool_attr_budget")


def guardrail_record(input_value: dict[str, Json]) -> Json:
    entry = object_value(input_value.get("entry"), "entry")
    span = payloads.GuardrailSpanData.from_logging_entry(
        cast("StandardLoggingGuardrailInformation", entry)
    )
    mapped = genai.GenAIMapper().map(span)
    return attributes_record(cast(Mapping[str, object], mapped))


def service_record(input_value: dict[str, Json]) -> Json:
    service = string_value(input_value.get("service"), "service")
    payload = ServiceLoggerPayload(
        is_error=input_value.get("error") is not None,
        error=optional_string(input_value.get("error")),
        service=ServiceTypes(service),
        duration=0.0,
        call_type=string_value(input_value.get("call_type"), "call_type"),
        caller=optional_string(input_value.get("caller")),
        event_metadata=None,
    )
    event_metadata = object_value(input_value.get("event_metadata", {}), "event_metadata")
    environment = object_value(input_value.get("environment", {}), "environment")
    span = payloads.ServiceSpanData.from_payload(
        payload, cast(Mapping[str, object], event_metadata)
    )

    def mapped() -> Json:
        result = genai.GenAIMapper().map(span)
        return attributes_record(cast(Mapping[str, object], result))

    return with_environment(environment, mapped)


def tool_call_record(input_value: dict[str, Json]) -> Json:
    payload = object_value(input_value.get("payload"), "payload")
    span = payloads.MCPToolCallSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload),
        capture_content=bool_value(input_value.get("capture_content")),
    )
    mapped = genai.GenAIMapper().map(span)
    return attributes_record(cast(Mapping[str, object], mapped))


def list_tools_record(input_value: dict[str, Json]) -> Json:
    payload = object_value(input_value.get("payload"), "payload")
    span = payloads.MCPListToolsSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload)
    )
    mapped = genai.GenAIMapper().map(span)
    return attributes_record(cast(Mapping[str, object], mapped))


def budget_record(input_value: dict[str, Json]) -> Json:
    vocabularies = integer_value(input_value.get("vocabularies"), "vocabularies")
    budget = utils.tool_attr_budget(vocabularies)
    span = llm_call_span(input_value)
    genai_attrs = genai.GenAIMapper(tool_attr_budget=budget).map(span)
    legacy_attrs = legacy.LegacyMapper(tool_attr_budget=budget).map(span)
    return cast(
        Json,
        {
            "budget": budget,
            "genai": attributes_record(cast(Mapping[str, object], genai_attrs)),
            "legacy": attributes_record(cast(Mapping[str, object], legacy_attrs)),
        },
    )


def messages_record(input_value: dict[str, Json]) -> Json:
    messages = list_value(input_value.get("messages"), "messages")
    serialized = utils.serialize_messages(cast(Sequence[object], messages))
    parts = logger_method("_transform_messages_to_otel_semantic_conventions")(messages)
    return cast(
        Json,
        {
            "serialized": None if serialized is None else json.loads(serialized),
            "parts": parts,
        },
    )


def responses_output_record(input_value: dict[str, Json]) -> Json:
    output = list_value(input_value.get("output"), "output")
    transformed = logger_method("_transform_responses_api_output_to_otel")(output)
    return cast(Json, {"messages": transformed})


def logger_method(name: str) -> Callable[[Json], Json]:
    return cast(Callable[[Json], Json], getattr(_LOGGER, name))


def attributes_record(attributes: Mapping[str, object]) -> Json:
    return cast(Json, {key: json_value(value) for key, value in attributes.items()})


def json_value(value: object) -> Json:
    if isinstance(value, str):
        try:
            parsed: Json = cast(Json, json.loads(value))
        except json.JSONDecodeError:
            return value
        return parsed
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, (list, tuple)):
        return cast(Json, [json_value(item) for item in cast(Sequence[object], value)])
    raise ValueError(f"unsupported attribute value of type {type(value).__name__}")


def with_environment(environment: dict[str, Json], action: Callable[[], Json]) -> Json:
    saved = {name: os.environ.get(name) for name in _DATABASE_ENVIRONMENT}
    try:
        for name in _DATABASE_ENVIRONMENT:
            os.environ.pop(name, None)
        for name, value in environment.items():
            os.environ[name] = string_value(value, name)
        return action()
    finally:
        for name, value in saved.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def bool_value(value: Json) -> bool:
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError("capture_content must be a boolean")
    return value


def optional_float(value: Json) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("time_to_first_chunk_seconds must be a number")
    return float(value)


def optional_string(value: Json) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("expected a string")
    return value
