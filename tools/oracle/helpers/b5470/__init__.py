from typing import TypeAlias, cast

from litellm.proxy.guardrails._content_utils import (
    NON_CONVERSATIONAL_CALL_TYPES,
    TEXT_CONTENT_CALL_TYPES,
    apply_redacted_messages_back,
    build_inspection_messages,
    has_non_string_content,
    is_string_batch_input,
    iter_message_text,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    result: Json
    match function:
        case "iter_message_text":
            data = object_value(input_value.get("data"), "data")
            result = {"result": cast(Json, list(iter_message_text(data)))}
        case "build_inspection_messages":
            data = object_value(input_value.get("data"), "data")
            result = {"result": cast(Json, build_inspection_messages(data))}
        case "is_string_batch_input":
            data = object_value(input_value.get("data"), "data")
            result = {"result": is_string_batch_input(data)}
        case "has_non_string_content":
            data = object_value(input_value.get("data"), "data")
            result = {"result": has_non_string_content(data)}
        case "apply_redacted_messages_back":
            data = object_value(input_value.get("data"), "data")
            redacted = array_value(input_value.get("redacted_messages"), "redacted_messages")
            returned = apply_redacted_messages_back(data, redacted)
            result = {"returned": returned, "data": cast(Json, data)}
        case "call_types":
            call_type = string_value(input_value.get("call_type"), "call_type")
            result = {
                "text_content": call_type in TEXT_CONTENT_CALL_TYPES,
                "non_conversational": call_type in NON_CONVERSATIONAL_CALL_TYPES,
            }
        case _:
            raise ValueError(f"unknown function {function}")
    return result
