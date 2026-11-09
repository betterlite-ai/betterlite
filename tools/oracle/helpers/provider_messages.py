import copy
import json
from collections.abc import Callable
from typing import Final, TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates import factory
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

convert_to_azure_openai_messages = cast(
    Callable[[list[AllMessageValues]], list[AllMessageValues]],
    factory.convert_to_azure_openai_messages,
)
azure_text_pt = cast(
    Callable[[list[AllMessageValues]], str],
    factory.azure_text_pt,
)
stringify_json_tool_call_content = cast(
    Callable[[list[AllMessageValues]], list[AllMessageValues]],
    factory.stringify_json_tool_call_content,
)
cohere_messages_pt_v2 = cast(
    Callable[[list[AllMessageValues], str, str], tuple[Json, list[Json]]],
    factory.cohere_messages_pt_v2,
)
cohere_message_pt = cast(
    Callable[[list[AllMessageValues]], tuple[Json, list[Json]]],
    factory.cohere_message_pt,
)


def _messages(input_value: dict[str, Json]) -> list[AllMessageValues]:
    return cast("list[AllMessageValues]", copy.deepcopy(input_value["messages"]))


def _as_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def _without_nulls(value: Json) -> Json:
    # betterlite's message model cannot tell an absent key from a null one, so both sides drop nulls
    if isinstance(value, list):
        return [_without_nulls(item) for item in value]
    if isinstance(value, dict):
        return {key: _without_nulls(item) for key, item in value.items() if item is not None}
    return value


def _azure_messages(input_value: dict[str, Json]) -> Json:
    return _without_nulls(cast(Json, convert_to_azure_openai_messages(_messages(input_value))))


def _azure_text(input_value: dict[str, Json]) -> Json:
    return azure_text_pt(_messages(input_value))


def _content_parsed(message: AllMessageValues) -> Json:
    fields = cast("dict[str, Json]", message)
    content = fields.get("content")
    if fields.get("role") == "tool" and isinstance(content, str):
        parsed = dict(fields)
        parsed["content"] = json.loads(content)
        return _without_nulls(cast(Json, parsed))
    return _without_nulls(cast(Json, fields))


def _stringify(input_value: dict[str, Json]) -> Json:
    converted = stringify_json_tool_call_content(_messages(input_value))
    return [_content_parsed(message) for message in converted]


def _cohere_messages(input_value: dict[str, Json]) -> Json:
    messages = _messages(input_value)
    recent, history = cohere_messages_pt_v2(copy.deepcopy(messages), "test-model", "cohere")
    prompt, tool_results = cohere_message_pt(copy.deepcopy(messages))
    return _without_nulls(
        cast(
            Json,
            [
                _as_json({"message": recent, "chat_history": history}),
                _as_json({"prompt": prompt, "tool_results": tool_results}),
            ],
        )
    )


HANDLERS: Final[dict[str, Callable[[dict[str, Json]], Json]]] = {
    "convert_to_azure_openai_messages": _azure_messages,
    "azure_text_pt": _azure_text,
    "stringify_json_tool_call_content": _stringify,
    "cohere_messages": _cohere_messages,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if not isinstance(operation, str):
        raise ValueError("operation must be a string")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unsupported operation {operation}")
    return handler(input_value)
