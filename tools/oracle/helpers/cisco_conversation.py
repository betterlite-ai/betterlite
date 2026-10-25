# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false
import json
from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.cisco_ai_defense.cisco_ai_defense import (
    CiscoAIDefenseGuardrail as Guardrail,
)
from litellm.types.utils import ModelResponse, TextCompletionResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def as_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def response_messages(function: str, response: dict[str, Json]) -> Json:
    match function:
        case "chat_response":
            built: object = ModelResponse(**response)
        case "text_completion":
            built = TextCompletionResponse(choices=response.get("choices"))
        case _:
            built = SimpleNamespace(**response)
    return as_json(Guardrail._extract_response_messages(built))


def responses_input(function: str, input_value: dict[str, Json]) -> Json:
    if function == "to_responses":
        messages = cast("list[dict[str, object]]", input_value["messages"])
        return as_json(Guardrail._sanitized_messages_to_responses_input(messages))
    text = input_value["sanitized_text"]
    if not isinstance(text, str):
        raise TypeError("sanitized_text must be a string")
    return as_json(Guardrail._rewrite_responses_input_text(input_value["input"], text))


def run(input_value: dict[str, Json]) -> Json:  # noqa: PLR0911
    function = input_value.pop("function")
    # LiteLLM annotates these data parameters as a bare dict
    request = Guardrail._extract_inspect_messages_from_request  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    tools = Guardrail._extract_tool_definition_text  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    match function:
        case "normalize_content":
            return Guardrail._normalize_message_content(input_value["content"])
        case "tool_arguments":
            message = object_of(input_value["message"])
            return as_json(Guardrail._extract_message_tool_argument_parts(message))
        case "reasoning":
            message = object_of(input_value["message"])
            return as_json(Guardrail._extract_message_reasoning_parts(message))
        case "flatten":
            return as_json(Guardrail._flatten_responses_input(input_value["input"]))
        case "request_messages":
            return as_json(request(object_of(input_value["data"])))
        case "tool_definitions":
            return tools(object_of(input_value["data"]))
        case "chat_response" | "text_completion" | "responses_output":
            return response_messages(function, object_of(input_value["response"]))
        case "to_responses" | "rewrite":
            return responses_input(function, input_value)
        case _:
            raise ValueError(f"unknown cisco_conversation function: {function}")
