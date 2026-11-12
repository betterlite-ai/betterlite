from collections.abc import Mapping, Sequence

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.tool_calls import (
    ChatCompletionAudioResponse,
    ChatCompletionDeltaToolCall,
    ChatCompletionMessageToolCall,
    FunctionCall,
    chat_completion_tool_call_from_dict,
    is_custom_tool_call_dict,
)
from betterlite.types.utils.tool_calls import (
    ChatCompletionDeltaCustomToolCall as DeltaCustomToolCall,
)
from betterlite.types.utils.tool_calls import (
    ChatCompletionMessageCustomToolCall as MessageCustomToolCall,
)
from betterlite.types.utils.values import JsonValue, StoredValue

MESSAGE_OPTIONAL = (
    "audio", "images", "annotations", "reasoning_content", "thinking_blocks", "reasoning_items",
)
DELTA_OPTIONAL = ("annotations", "reasoning_content", "thinking_blocks", "reasoning_items")


def add_provider_specific_fields(
    response: ResponseObject, provider_specific_fields: StoredValue,
) -> None:
    match provider_specific_fields:
        case None:
            return
        case Mapping() if len(provider_specific_fields) == 0:
            return
        case _other:
            response["provider_specific_fields"] = provider_specific_fields


class Message(ResponseObject):
    content: str | None
    role: str
    tool_calls: list[ChatCompletionMessageToolCall | MessageCustomToolCall] | None
    function_call: FunctionCall | None
    audio: ChatCompletionAudioResponse | None
    images: list[JsonValue] | None
    reasoning_content: str | None
    thinking_blocks: list[JsonValue] | None
    reasoning_items: list[JsonValue] | None
    provider_specific_fields: dict[str, JsonValue] | None
    annotations: list[JsonValue] | None

    @override
    def __init__(
        self, content: StoredValue = None, role: StoredValue = None,
        function_call: StoredValue = None, tool_calls: StoredValue = None,
        audio: StoredValue = None, images: StoredValue = None,
        provider_specific_fields: StoredValue = None, reasoning_content: StoredValue = None,
        thinking_blocks: StoredValue = None, reasoning_items: StoredValue = None,
        annotations: StoredValue = None, **params: StoredValue,
    ) -> None:
        data: dict[str, StoredValue] = dict(params)
        data.update(
            content=content, role="assistant" if role is None or role == "" else role,
            function_call=function_call, tool_calls=message_tool_calls(tool_calls),
            audio=audio, images=images, annotations=annotations,
            reasoning_content=reasoning_content, thinking_blocks=thinking_blocks,
            reasoning_items=reasoning_items,
        )
        super().__init__(**data)
        for name in MESSAGE_OPTIONAL:
            if self[name] is None:
                del self[name]
        add_provider_specific_fields(self, provider_specific_fields)

    @override
    def json(self, **_kwargs: JsonValue) -> JsonValue:
        return self.model_dump()


class Delta(ResponseObject):
    content: str | None
    role: str | None
    function_call: FunctionCall | None
    tool_calls: list[ChatCompletionDeltaToolCall | DeltaCustomToolCall] | None
    audio: ChatCompletionAudioResponse | JsonValue
    reasoning_content: str | None
    thinking_blocks: list[JsonValue] | None
    reasoning_items: list[JsonValue] | None
    provider_specific_fields: dict[str, JsonValue] | None

    @override
    def __init__(
        self, content: StoredValue = None, role: StoredValue = None,
        function_call: StoredValue = None, tool_calls: StoredValue = None,
        audio: StoredValue = None, images: StoredValue = None,
        reasoning_content: StoredValue = None, thinking_blocks: StoredValue = None,
        reasoning_items: StoredValue = None, annotations: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        if reasoning_content is None and "reasoning" in params:
            reasoning_content = params.pop("reasoning")
        data: dict[str, StoredValue] = dict(params)
        data.update(
            content=content, role=role, function_call=function_call,
            tool_calls=delta_tool_calls(tool_calls), audio=audio, annotations=annotations,
            reasoning_content=reasoning_content, thinking_blocks=thinking_blocks,
            reasoning_items=reasoning_items,
        )
        images_value = nonempty_sequence(images)
        if images_value is not None:
            data["images"] = images_value
        super().__init__(**data)
        provider_fields = params.get("provider_specific_fields")
        add_provider_specific_fields(self, provider_fields)
        for name in DELTA_OPTIONAL:
            if self[name] is None:
                del self[name]


def nonempty_sequence(value: StoredValue) -> StoredValue:
    match value:
        case Sequence() if len(value) > 0:
            return value
        case _other:
            return None


def message_tool_calls(tool_calls: StoredValue) -> StoredValue:
    match tool_calls:
        case Sequence() if len(tool_calls) > 0:
            return [message_tool_call(tool_call) for tool_call in tool_calls]
        case _other:
            return None


def message_tool_call(tool_call: StoredValue) -> StoredValue:
    match tool_call:
        case Mapping():
            return chat_completion_tool_call_from_dict(tool_call)
        case _other:
            return tool_call


def delta_tool_calls(tool_calls: StoredValue) -> StoredValue:
    match tool_calls:
        case Sequence():
            converted: list[StoredValue] = []
            counter = 0
            for tool_call in tool_calls:
                match tool_call:
                    case Mapping():
                        updated = tool_call
                        if tool_call.get("index") is None:
                            updated = {**tool_call, "index": counter}
                            counter += 1
                        converted.append(delta_tool_call(updated))
                    case _other:
                        converted.append(tool_call)
            return converted
        case _other:
            return tool_calls


def delta_tool_call(tool_call: Mapping[str, StoredValue]) -> StoredValue:
    if is_custom_tool_call_dict(tool_call):
        filtered = {
            key: value
            for key, value in tool_call.items()
            if not (key == "function" and value is None)
        }
        return DeltaCustomToolCall(**filtered)
    data = dict(tool_call)
    if "type" not in data or data["type"] is None:
        data["type"] = "function"
    return ChatCompletionDeltaToolCall(**data)
