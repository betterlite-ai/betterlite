import json
import uuid
from collections.abc import Mapping, Sequence
from typing import ClassVar, Literal

from pydantic import JsonValue, field_validator
from typing_extensions import override

from betterlite.types.utils.base import Dumpable, InitValue, ModelValue, ResponseObject

__all__ = ["_CustomToolCallAccess"]


class _CustomToolCallAccess(ResponseObject): ...


class ChatCompletionCustomToolCallPayload(_CustomToolCallAccess):  # ast-grep-ignore: no-long-names
    name: str
    input: str


# ast-grep-ignore: no-long-names
class ChatCompletionDeltaCustomToolCallPayload(_CustomToolCallAccess):
    name: str | None = None
    input: str | None = None


# ast-grep-ignore: no-long-names
class ChatCompletionMessageCustomToolCall(_CustomToolCallAccess):
    id: str
    type: Literal["custom"] = "custom"
    custom: ChatCompletionCustomToolCallPayload  # ast-grep-ignore: no-long-names


class ChatCompletionDeltaCustomToolCall(_CustomToolCallAccess):  # ast-grep-ignore: no-long-names
    id: str | None = None
    type: str | None = None
    custom: ChatCompletionDeltaCustomToolCallPayload  # ast-grep-ignore: no-long-names
    index: int


class TopLogprob(ResponseObject):
    token: str
    bytes: list[int] | None = None
    logprob: float


class ChatCompletionTokenLogprob(ResponseObject):
    token: str
    bytes: list[int] | None = None
    logprob: float
    top_logprobs: list[TopLogprob]

    @override
    def __init__(
        self, token: JsonValue, logprob: JsonValue, top_logprobs: JsonValue,
        **params: InitValue,
    ) -> None:
        super().__init__(
            token=token, logprob=logprob, top_logprobs=top_logprobs, **params,
        )

    @field_validator("top_logprobs", mode="before")
    @classmethod
    def ensure_top_logprobs_is_list(cls, value: JsonValue) -> JsonValue:
        if value is None:
            return []
        return value


class ChoiceLogprobs(ResponseObject):
    content: list[ChatCompletionTokenLogprob] | None = None


class FunctionCall(ResponseObject):
    arguments: str
    name: str | None = None


class Function(ResponseObject):
    arguments: str
    name: str | None

    @override
    def __init__(
        self, arguments: JsonValue = None, name: JsonValue = None, **params: InitValue,
    ) -> None:
        super().__init__(arguments=function_arguments(arguments, params), name=name)


class ChatCompletionDeltaToolCall(ResponseObject):
    id: str | None = None
    function: Function
    type: str | None = None
    index: int


class ChatCompletionMessageToolCall(ResponseObject):
    id: ClassVar[str]
    function: ClassVar[Function]
    type: ClassVar[str]

    @override
    def __init__(
        self, function: ModelValue, id: JsonValue = None, type: JsonValue = None,
        **params: InitValue,
    ) -> None:
        generated = str(uuid.uuid4()) if id is None else id
        kind = "function" if type is None else type
        selected: ModelValue = function
        if function is not None:
            match function:
                case Mapping():
                    selected = Function.model_validate(function)
                case Dumpable() | bool() | int() | float() | str() | list():
                    selected = function
        super().__init__(function=selected, id=generated, type=kind, **params)


class ChatCompletionAudioResponse(ResponseObject):
    id: str
    data: str
    expires_at: int
    transcript: str

    @override
    def __init__(
        self, data: JsonValue = None, expires_at: JsonValue = None,
        transcript: JsonValue = None, id: JsonValue = None, **params: InitValue,
    ) -> None:
        generated = str(uuid.uuid4()) if id is None else id
        super().__init__(
            data=data, expires_at=expires_at, transcript=transcript, id=generated, **params,
        )


def function_arguments(arguments: JsonValue, params: dict[str, InitValue]) -> JsonValue:
    if arguments is None:
        parameters = params.pop("parameters", None)
        if parameters is None:
            return ""
        match parameters:
            case Mapping():
                packed: dict[str, InitValue] = dict(parameters)
                return json.dumps(packed)
            case Dumpable() | Sequence() | bool() | int() | float():
                return ""
    match arguments:
        case Mapping():
            fields: dict[str, JsonValue] = dict(arguments)
            return json.dumps(fields)
        case bool() | int() | float() | str() | list():
            return arguments


def is_custom_tool_call_dict(tool_call: Mapping[str, JsonValue]) -> bool:
    if tool_call.get("type") == "custom":
        return True
    return tool_call.get("custom") is not None


def chat_completion_tool_call_from_dict(  # ast-grep-ignore: no-long-names
    tool_call: Mapping[str, JsonValue],
) -> "ChatCompletionMessageToolCall | ChatCompletionMessageCustomToolCall":
    if is_custom_tool_call_dict(tool_call):
        filtered: dict[str, JsonValue] = {
            key: value for key, value in tool_call.items()
            if not (key in ("function", "type") and value is None)
        }
        # ast-grep-ignore: no-long-names
        return ChatCompletionMessageCustomToolCall.model_validate(filtered)
    return ChatCompletionMessageToolCall(**tool_call)
