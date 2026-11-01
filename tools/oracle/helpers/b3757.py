from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail as Guardrail,
)
from litellm.types.utils import Choices, TextCompletionResponse

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false
Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def guardrail() -> Guardrail:
    return Guardrail(api_key="key")


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    sanitized_text = cast("str | None", input_value.get("sanitized_text"))
    sanitized_messages = cast(
        "list[dict[str, object]] | None", input_value.get("sanitized_messages")
    )
    result: Json
    match function:
        case "apply_redaction_mcp_input":
            request_data = dict(object_of(input_value["request_data"]))
            sanitized_mcp_arguments = cast(
                "dict[str, object] | None", input_value.get("sanitized_mcp_arguments")
            )
            applied = Guardrail._redact_mcp_input(  # pyright: ignore[reportUnknownMemberType]
                cast("dict[str, object]", request_data), sanitized_text, sanitized_mcp_arguments
            )
            result = {"applied": applied, "request_data": cast("Json", request_data)}
        case "apply_redaction_chat_input":
            request_data = dict(object_of(input_value["request_data"]))
            applied = guardrail()._redact_chat_input(  # pyright: ignore[reportUnknownMemberType]
                cast("dict[str, object]", request_data), sanitized_text, sanitized_messages
            )
            result = {"applied": applied, "request_data": cast("Json", request_data)}
        case "apply_redaction_chat_output_model_response":
            raw_choices = cast("list[dict[str, object]]", input_value["choices"])
            choices = [Choices(**choice) for choice in raw_choices]  # pyright: ignore[reportArgumentType]
            applied = Guardrail._redact_model_response_choices(  # pyright: ignore[reportUnknownMemberType]
                choices, sanitized_text, sanitized_messages
            )
            messages = [choice.message.model_dump(exclude_none=True) for choice in choices]
            result = {"applied": applied, "messages": cast("Json", messages)}
        case "apply_redaction_chat_output_text_completion":
            raw_choices = cast("list[dict[str, object]]", input_value["choices"])
            built = TextCompletionResponse(choices=raw_choices)
            applied = Guardrail._redact_text_completion_choices(  # pyright: ignore[reportUnknownMemberType]
                built.choices, sanitized_text, sanitized_messages
            )
            texts = cast("Json", [choice.text for choice in built.choices])
            result = {"applied": applied, "texts": texts}
        case "apply_redaction_responses_api_output":
            response = dict(object_of(input_value["response"]))
            output_items = cast("list[dict[str, object]]", response["output"])
            applied = guardrail()._redact_responses_api_output(  # pyright: ignore[reportUnknownMemberType]
                output_items, sanitized_text, sanitized_messages
            )
            result = {"applied": applied, "response": cast("Json", response)}
        case "apply_redaction_mcp_output":
            response_obj = input_value.get("response")
            if response_obj is not None and sanitized_text:
                applied = Guardrail._set_mcp_tool_response_text(response_obj, sanitized_text)
            else:
                applied = False
            result = {"applied": applied, "response": response_obj}
        case "extract_masked_entity_count_rules":
            rules = cast("list[dict[str, Json]]", input_value["rules"])
            result = cast("Json", Guardrail._extract_masked_entity_count(rules))
        case _:
            raise ValueError(f"unknown function: {function}")
    return {"result": result}
