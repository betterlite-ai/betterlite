import json
import uuid
from collections.abc import Mapping
from typing import ClassVar, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import StoredValue, ValidationError, field_validator

__all__ = ["_CustomToolCallAccess"]


class _CustomToolCallAccess(ResponseObject):
    __slots__ = ()


class ChatCompletionCustomToolCallPayload(_CustomToolCallAccess):  # ast-grep-ignore: no-long-names
    name: str
    input: str


# ast-grep-ignore: no-long-names
class ChatCompletionDeltaCustomToolCallPayload(_CustomToolCallAccess):
    name: str | None
    input: str | None


# ast-grep-ignore: no-long-names
class ChatCompletionMessageCustomToolCall(_CustomToolCallAccess):
    id: str
    type: Literal["custom"]
    custom: ChatCompletionCustomToolCallPayload  # ast-grep-ignore: no-long-names
    defaults: ClassVar[dict[str, StoredValue]] = {"type": "custom"}


class ChatCompletionDeltaCustomToolCall(_CustomToolCallAccess):  # ast-grep-ignore: no-long-names
    id: str | None
    type: str | None
    custom: ChatCompletionDeltaCustomToolCallPayload  # ast-grep-ignore: no-long-names
    index: int


class TopLogprob(ResponseObject):
    token: str
    bytes: list[int] | None
    logprob: float


class ChatCompletionTokenLogprob(ResponseObject):
    token: str
    bytes: list[int] | None
    logprob: float
    top_logprobs: list[TopLogprob]

    @field_validator("top_logprobs", mode="before")
    def ensure_top_logprobs_is_list(self, value: StoredValue) -> StoredValue:
        if value is None:
            return []
        return value


class ChoiceLogprobs(ResponseObject):
    content: list[ChatCompletionTokenLogprob] | None


class FunctionCall(ResponseObject):
    arguments: str
    name: str | None


class Function(ResponseObject):
    arguments: str
    name: str | None

    @override
    def __init__(
        self, arguments: StoredValue = None, name: StoredValue = None, **params: StoredValue,
    ) -> None:
        super().__init__(arguments=function_arguments(arguments, params), name=name)


class ChatCompletionDeltaToolCall(ResponseObject):
    id: str | None
    function: Function
    type: str | None
    index: int


class ChatCompletionMessageToolCall(ResponseObject):
    id: str
    function: Function
    type: str

    @override
    def __init__(
        self, function: StoredValue, id: StoredValue = None, type: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        generated = str(uuid.uuid4()) if id is None else id
        kind = "function" if type is None else type
        super().__init__(function=function, id=generated, type=kind, **params)


class ChatCompletionAudioResponse(ResponseObject):
    id: str
    data: str
    expires_at: int
    transcript: str

    @override
    def __init__(
        self, data: StoredValue, expires_at: StoredValue, transcript: StoredValue,
        id: StoredValue = None, **params: StoredValue,
    ) -> None:
        generated = str(uuid.uuid4()) if id is None else id
        super().__init__(
            data=data, expires_at=expires_at, transcript=transcript, id=generated, **params,
        )


def function_arguments(arguments: StoredValue, params: dict[str, StoredValue]) -> str:
    match arguments:
        case None if "parameters" not in params:
            return ""
        case None:
            parameters = params.pop("parameters")
            match parameters:
                case dict():
                    return json.dumps(parameters)
                case _other:
                    return ""
        case dict():
            return json.dumps(arguments)
        case str():
            return arguments
        case _other:
            raise ValidationError("Function", ("arguments",), "wrong type")


def is_custom_tool_call_dict(tool_call: Mapping[str, StoredValue]) -> bool:
    if tool_call.get("type") == "custom":
        return True
    return tool_call.get("custom") is not None


def chat_completion_tool_call_from_dict(  # ast-grep-ignore: no-long-names
    tool_call: Mapping[str, StoredValue],
) -> "ChatCompletionMessageToolCall | ChatCompletionMessageCustomToolCall":
    if is_custom_tool_call_dict(tool_call):
        return ChatCompletionMessageCustomToolCall(**{
            key: value for key, value in tool_call.items()
            if not (key in ("function", "type") and value is None)
        })
    return ChatCompletionMessageToolCall(**tool_call)
