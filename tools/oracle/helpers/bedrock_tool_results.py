from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import (
    _convert_to_bedrock_tool_call_invoke,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _convert_to_bedrock_tool_call_result,  # pyright: ignore[reportPrivateUsage]
    _sanitize_bedrock_tool_use_id,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionFunctionMessage,
    ChatCompletionToolMessage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def uuid4(self) -> str:
        return self.value


def _tool_call_invoke(input_value: dict[str, Json]) -> Json:
    tool_calls = cast("list[object]", input_value["tool_calls"])
    declared_model = input_value.get("model")
    model = declared_model if isinstance(declared_model, str) else None
    return cast(Json, _convert_to_bedrock_tool_call_invoke(tool_calls, model=model))


def _tool_call_result(input_value: dict[str, Json]) -> Json:
    message = cast(
        "ChatCompletionToolMessage | ChatCompletionFunctionMessage",
        cast(AllMessageValues, input_value["message"]),
    )
    fixed_id = cast(str, input_value.get("id", ""))
    with patch.object(factory, "uuid", _FixedUuid(fixed_id)):
        return cast(Json, _convert_to_bedrock_tool_call_result(message))


def _sanitize_ids(input_value: dict[str, Json]) -> Json:
    values = cast("list[str]", input_value["values"])
    return [_sanitize_bedrock_tool_use_id(value) for value in values]


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("function"):
        case "tool_call_invoke":
            return _tool_call_invoke(input_value)
        case "tool_call_result":
            return _tool_call_result(input_value)
        case "sanitize_tool_use_ids":
            return _sanitize_ids(input_value)
        case other:
            raise ValueError(f"unknown bedrock_tool_results function: {other!r}")
