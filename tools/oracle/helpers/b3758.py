from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail as Guardrail,
)

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false
Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def run(input_value: dict[str, Json]) -> Json:  # noqa: PLR0911
    function = input_value.get("function")
    match function:
        case "unwrap_verdict":
            response = object_of(input_value["response"])
            return cast("Json", Guardrail._unwrap_verdict_envelope(response))
        case "has_decision_fields":
            return Guardrail._has_decision_fields(input_value["response"])
        case "normalize_action":
            return Guardrail._normalize_action(cast("str", input_value["action"]))
        case "extract_jsonrpc_error":
            response = object_of(input_value["response"])
            return cast("Json", Guardrail._extract_jsonrpc_error(response))
        case "extract_sanitized_text":
            response = object_of(input_value["response"])
            return Guardrail._extract_sanitized_text(response)
        case "extract_sanitized_messages":
            response = object_of(input_value["response"])
            return cast("Json", Guardrail._extract_sanitized_messages(response))
        case "extract_sanitized_mcp_arguments":
            response = cast("dict[str, object]", object_of(input_value["response"]))
            return cast("Json", Guardrail._extract_sanitized_mcp_arguments(response))
        case "extract_masked_entity_count":
            rules = cast("list[dict[str, Json]]", input_value["rules"])
            return cast("Json", Guardrail._extract_masked_entity_count(rules))
        case _:
            raise ValueError(f"unknown function: {function}")
