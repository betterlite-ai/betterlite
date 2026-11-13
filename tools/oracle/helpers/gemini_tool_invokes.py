from collections.abc import Callable
from typing import Final, TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.factory import (
    _encode_tool_call_id_with_signature,  # pyright: ignore[reportPrivateUsage]
    _gemini_tool_call_invoke_helper,  # pyright: ignore[reportPrivateUsage]
    _get_dummy_thought_signature,  # pyright: ignore[reportPrivateUsage]
    _get_thought_signature_from_tool,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    convert_to_gemini_tool_call_invoke,
    infer_protocol_value,
)
from litellm.types.llms.openai import (
    ChatCompletionAssistantMessage,
    ChatCompletionToolCallFunctionChunk,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _infer_protocol_values(input_value: dict[str, Json]) -> Json:
    values = cast(list[Json], input_value["values"])
    return cast(Json, [infer_protocol_value(value) for value in values])


def _invoke_helper(input_value: dict[str, Json]) -> Json:
    params = cast(ChatCompletionToolCallFunctionChunk, input_value["function_call_params"])
    tool_call_id = cast("str | None", input_value.get("tool_call_id"))
    return cast(
        Json,
        _gemini_tool_call_invoke_helper(
            function_call_params=params,
            tool_call_id=tool_call_id,
        ),
    )


def _encode_tool_call_id(input_value: dict[str, Json]) -> Json:
    pairs = cast(list[dict[str, Json]], input_value["pairs"])
    return cast(
        Json,
        [
            _encode_tool_call_id_with_signature(
                cast(str, pair["tool_call_id"]),
                cast("str | None", pair.get("thought_signature")),
            )
            for pair in pairs
        ],
    )


def _thought_signature_from_tool(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, Json]], input_value["tools"])
    return cast(Json, [_get_thought_signature_from_tool(dict(tool)) for tool in tools])


def _dummy_thought_signature(_input_value: dict[str, Json]) -> Json:
    return _get_dummy_thought_signature()


def _convert(input_value: dict[str, Json]) -> Json:
    message = cast(ChatCompletionAssistantMessage, input_value["message"])
    model = cast("str | None", input_value.get("model"))
    forward_function_call_id = cast(bool, input_value.get("forward_function_call_id", False))
    return cast(
        Json,
        convert_to_gemini_tool_call_invoke(
            message,
            model=model,
            forward_function_call_id=forward_function_call_id,
        ),
    )


HANDLERS: Final[dict[str, Callable[[dict[str, Json]], Json]]] = {
    "infer_protocol_value": _infer_protocol_values,
    "gemini_tool_call_invoke_helper": _invoke_helper,
    "encode_tool_call_id_with_signature": _encode_tool_call_id,
    "thought_signature_from_tool": _thought_signature_from_tool,
    "dummy_thought_signature": _dummy_thought_signature,
    "convert_to_gemini_tool_call_invoke": _convert,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if not isinstance(operation, str):
        raise ValueError("operation must be a string")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unsupported operation {operation}")
    return handler(input_value)
