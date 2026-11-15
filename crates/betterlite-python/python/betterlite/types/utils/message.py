from collections.abc import Mapping, Sequence
from typing import ClassVar, Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    Dumpable,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
)
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

MESSAGE_OPTIONAL = (
    "audio", "images", "annotations", "reasoning_content", "thinking_blocks", "reasoning_items",
)
DELTA_DROPPED = (
    "images", "annotations", "reasoning_content", "thinking_blocks", "reasoning_items",
)
MessageRole: TypeAlias = Literal["assistant", "user", "system", "tool", "function"]


def add_provider_specific_fields(
    response: ResponseObject, provider_specific_fields: InitValue,
) -> None:
    if provider_specific_fields is None:
        return
    match provider_specific_fields:
        case Mapping() if len(provider_specific_fields) == 0:
            return
        case Mapping():
            response["provider_specific_fields"] = {
                str(name): dump_value(item, exclude_unset=True)
                for name, item in provider_specific_fields.items()
            }
        case Dumpable() | Sequence() | bool() | int() | float():
            return


class Message(DictJsonObject):
    content: str | None
    role: MessageRole
    tool_calls: list[ChatCompletionMessageToolCall | MessageCustomToolCall] | None
    function_call: FunctionCall | None
    audio: ChatCompletionAudioResponse | None = None
    images: list[JsonValue] | None = None
    reasoning_content: str | None = None
    thinking_blocks: list[JsonValue] | None = None
    reasoning_items: list[JsonValue] | None = None
    provider_specific_fields: dict[str, JsonValue] | None = None
    annotations: list[JsonValue] | None = None

    @override
    def __init__(
        self, content: JsonValue = None, role: JsonValue = None,
        function_call: JsonValue = None, tool_calls: Sequence[ModelValue] | None = None,
        audio: JsonValue = None, images: Sequence[JsonValue] | None = None,
        provider_specific_fields: Mapping[str, JsonValue] | None = None,
        reasoning_content: JsonValue = None, thinking_blocks: Sequence[JsonValue] | None = None,
        reasoning_items: Sequence[JsonValue] | None = None,
        annotations: Sequence[JsonValue] | None = None, **params: InitValue,
    ) -> None:
        selected_role = "assistant" if role is None or role == "" else role
        fields: dict[str, InitValue] = {
            "content": content, "role": selected_role, "function_call": function_call,
            "tool_calls": message_tool_calls(tool_calls),
        }
        if audio is not None:
            fields["audio"] = audio
        if images is not None:
            fields["images"] = list(images)
        if annotations is not None:
            fields["annotations"] = list(annotations)
        if reasoning_content is not None:
            fields["reasoning_content"] = reasoning_content
        if thinking_blocks is not None:
            fields["thinking_blocks"] = list(thinking_blocks)
        if reasoning_items is not None:
            fields["reasoning_items"] = list(reasoning_items)
        super().__init__(**fields, **params)
        # LiteLLM drops each optional field it was not given, so no null key reaches the wire
        for name in MESSAGE_OPTIONAL:
            if name not in fields:
                del self[name]
        add_provider_specific_fields(self, provider_specific_fields)


class Delta(ResponseObject):
    content: ClassVar[str | None]
    role: ClassVar[str | None]
    function_call: ClassVar[FunctionCall | None]
    tool_calls: ClassVar[list[ChatCompletionDeltaToolCall | DeltaCustomToolCall] | None]
    audio: ClassVar[ChatCompletionAudioResponse | JsonValue]
    images: ClassVar[list[JsonValue] | None]
    annotations: ClassVar[list[JsonValue] | None]
    reasoning_content: str | None = None
    thinking_blocks: list[JsonValue] | None = None
    reasoning_items: list[JsonValue] | None = None
    provider_specific_fields: dict[str, JsonValue] | None = None

    @override
    def __init__(
        self, content: JsonValue = None, role: JsonValue = None,
        function_call: JsonValue = None, tool_calls: Sequence[ModelValue] | None = None,
        audio: JsonValue = None, images: Sequence[JsonValue] | None = None,
        reasoning_content: JsonValue = None, thinking_blocks: Sequence[JsonValue] | None = None,
        reasoning_items: Sequence[JsonValue] | None = None,
        annotations: Sequence[JsonValue] | None = None, **params: InitValue,
    ) -> None:
        selected_reasoning: InitValue = reasoning_content
        if selected_reasoning is None and "reasoning" in params:
            selected_reasoning = params.pop("reasoning")
        provider_fields: InitValue = params.pop("provider_specific_fields", None)
        extras: dict[str, InitValue] = {
            "content": content, "role": role,
            "function_call": function_call_value(function_call),
            "tool_calls": delta_tool_calls(tool_calls), "audio": audio,
        }
        if images is not None and len(images) > 0:
            extras["images"] = list(images)
        if annotations is not None:
            extras["annotations"] = list(annotations)
        if selected_reasoning is not None:
            extras["reasoning_content"] = selected_reasoning
        if thinking_blocks is not None:
            extras["thinking_blocks"] = list(thinking_blocks)
        if reasoning_items is not None:
            extras["reasoning_items"] = list(reasoning_items)
        super().__init__(**params, **extras)
        for name in DELTA_DROPPED:
            if name not in extras:
                del self[name]
        # LiteLLM marks both names set while building a delta, then leaves their values out
        self.model_fields_set.update(("images", "annotations"))
        add_provider_specific_fields(self, provider_fields)


def function_call_value(function_call: ModelValue) -> ModelValue:
    match function_call:
        case Mapping():
            return FunctionCall.model_validate(function_call)
        case Dumpable() | list() | bool() | int() | float() | str() | None:
            return function_call


def message_tool_calls(tool_calls: Sequence[ModelValue] | None) -> list[ModelValue] | None:
    if tool_calls is None or len(tool_calls) == 0:
        return None
    return [message_tool_call(tool_call) for tool_call in tool_calls]


def message_tool_call(tool_call: ModelValue) -> ModelValue:
    if tool_call is None:
        return None
    match tool_call:
        case Mapping():
            return chat_completion_tool_call_from_dict(tool_call)
        case Dumpable() | list() | bool() | int() | float() | str():
            return tool_call


def delta_tool_calls(tool_calls: Sequence[ModelValue] | None) -> list[ModelValue] | None:
    if tool_calls is None:
        return None
    converted: list[ModelValue] = []
    counter = 0
    for tool_call in tool_calls:
        if tool_call is None:
            continue
        match tool_call:
            case Mapping():
                updated = tool_call
                if tool_call.get("index") is None:
                    updated = {**tool_call, "index": counter}
                    counter += 1
                converted.append(delta_tool_call(updated))
            case Dumpable() | list() | bool() | int() | float() | str():
                converted.append(tool_call)
    return converted


def delta_tool_call(tool_call: Mapping[str, JsonValue]) -> ModelValue:
    if is_custom_tool_call_dict(tool_call):
        filtered: dict[str, JsonValue] = {
            key: value
            for key, value in tool_call.items()
            if not (key == "function" and value is None)
        }
        return DeltaCustomToolCall.model_validate(filtered)
    data: dict[str, JsonValue] = dict(tool_call)
    if data.get("type") is None:
        data["type"] = "function"
    return ChatCompletionDeltaToolCall.model_validate(data)
