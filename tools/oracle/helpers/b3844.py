import json
from collections.abc import Callable, Mapping, Sequence
from dataclasses import asdict
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.opentelemetry_utils import base_otel_llm_obs_attributes
from litellm.integrations.otel.model import payloads

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload


def run(input_value: dict[str, Json]) -> Json:
    return {"result": dispatch(input_value)}


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "primitive_value":
        result: Json = primitive_record(input_value.get("value"))
    elif function == "tools":
        result = tools_record(input_value.get("value"))
    elif function == "embedding_output":
        result = embedding_record(input_value.get("value"))
    elif function == "llm_call":
        result = llm_call_record(input_value)
    elif function == "mcp_tool_call":
        result = tool_call_record(input_value)
    elif function == "mcp_list_tools":
        result = list_tools_record(input_value.get("payload"))
    elif function == "payload_kind":
        result = kind_record(input_value.get("payload"))
    else:
        raise ValueError(f"unknown LiteLLM helper function {function} ({litellm.__name__})")
    return result


def primitive_record(value: Json) -> Json:
    name = "cast_as_primitive_value_type"
    cast_primitive = cast(
        Callable[[object], str | bool | int | float],
        getattr(base_otel_llm_obs_attributes, name),
    )
    return cast(Json, cast_primitive(cast(object, value)))


def tools_record(value: Json) -> Json:
    parameters = object_value(value, "value")
    name = "_extract_tools"
    extract = cast(
        Callable[[Mapping[str, object]], Sequence[payloads.ToolDefinition]],
        getattr(payloads, name),
    )
    return tool_records(extract(parameters))


def tool_records(tools: Sequence[payloads.ToolDefinition]) -> Json:
    return cast(
        Json,
        [
            {
                "name": tool.name,
                "description": tool.description,
                "parameters_json": compact_json(tool.parameters_json),
            }
            for tool in tools
        ],
    )


def compact_json(text: str | None) -> Json:
    if text is None:
        return None
    try:
        parsed = cast(Json, json.loads(text))
    except json.JSONDecodeError:
        return text
    return cast(Json, json.dumps(parsed, separators=(",", ":"), sort_keys=True))


def embedding_record(value: Json) -> Json:
    response = object_value(value, "value")
    parsed = payloads.EmbeddingOutput.from_response(response)
    return None if parsed is None else cast(Json, asdict(parsed))


def llm_call_record(value: dict[str, Json]) -> Json:
    payload = object_value(value.get("payload"), "payload")
    parsed = payloads.LLMCallSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload),
        capture_content=bool_value(value.get("capture_content")),
        time_to_first_chunk_seconds=optional_float(
            value.get("time_to_first_chunk_seconds")
        ),
        request_route=optional_string(value.get("request_route")),
        session_id=optional_string(value.get("session_id")),
    )
    return cast(
        Json,
        {
            "operation": parsed.operation.value,
            "provider": parsed.provider,
            "request_model": parsed.request_model,
            "response_model": parsed.response_model,
            "response_id": parsed.response_id,
            "request_params": cast(Json, asdict(parsed.request_params)),
            "usage": usage_record(parsed.usage),
            "finish_reasons": list(parsed.finish_reasons),
            "error": record_or_none(parsed.error),
            "response_cost": parsed.response_cost,
            "server": record_or_none(parsed.server),
            "identity": identity_record(parsed.identity),
            "is_streaming": parsed.is_streaming,
            "cost": cost_record(parsed.cost),
            "tools": tool_records(parsed.tools),
            "messages_in": [cast(Json, dict(message)) for message in parsed.messages_in],
            "choices_out": [cast(Json, dict(choice)) for choice in parsed.choices_out],
            "system_fingerprint": parsed.system_fingerprint,
            "time_to_first_chunk_seconds": parsed.time_to_first_chunk_seconds,
            "output_type": None
            if parsed.output_type is None
            else parsed.output_type.value,
            "call_type": parsed.call_type,
            "request_route": parsed.request_route,
            "session_id": parsed.session_id,
            "embedding_output": record_or_none(parsed.embedding_output),
        },
    )


def tool_call_record(value: dict[str, Json]) -> Json:
    payload = object_value(value.get("payload"), "payload")
    parsed = payloads.MCPToolCallSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload),
        capture_content=bool_value(value.get("capture_content")),
    )
    return cast(
        Json,
        {
            "operation": parsed.operation.value,
            "method": parsed.method,
            "tool_name": parsed.tool_name,
            "server_name": parsed.server_name,
            "server_address": parsed.server_address,
            "server_port": parsed.server_port,
            "session_id": parsed.session_id,
            "arguments_json": compact_json(parsed.arguments_json),
            "result_json": compact_json(parsed.result_json),
            "error": record_or_none(parsed.error),
            "response_cost": parsed.response_cost,
            "identity": identity_record(parsed.identity),
        },
    )


def list_tools_record(value: Json) -> Json:
    payload = object_value(value, "payload")
    parsed = payloads.MCPListToolsSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload)
    )
    return cast(
        Json,
        {
            "method": parsed.method,
            "session_id": parsed.session_id,
            "error": record_or_none(parsed.error),
            "identity": identity_record(parsed.identity),
        },
    )


def kind_record(value: Json) -> Json:
    payload = cast("StandardLoggingPayload", object_value(value, "payload"))
    if payloads.is_mcp_tool_call(payload):
        return "mcp_tool_call"
    if payloads.is_mcp_list_tools(payload):
        return "mcp_list_tools"
    return "inference"


def usage_record(usage: payloads.LLMUsage) -> Json:
    return cast(
        Json,
        {
            **usage.__dict__,
            "uncached_input_tokens": usage.uncached_input_tokens,
            "non_reasoning_output_tokens": usage.non_reasoning_output_tokens,
        },
    )


def cost_record(cost: payloads.LLMCost) -> Json:
    fields = cast(dict[str, Json], cost.__dict__)
    return cast(
        Json,
        {
            key: int(amount)
            if isinstance(amount, float) and amount.is_integer()
            else amount
            for key, amount in fields.items()
        },
    )


def identity_record(identity: payloads.RequestIdentity) -> Json:
    return cast(
        Json,
        {
            "call_id": identity.call_id,
            "request_route": identity.request_route,
        },
    )


def record_or_none(
    value: payloads.EmbeddingOutput | payloads.SpanError | payloads.ServerInfo | None,
) -> Json:
    if value is None:
        return None
    return cast(Json, asdict(value))


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(dict[str, Json], value)


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
