from typing import ClassVar, Literal

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import StoredValue


class _CustomToolCallAccess(ResponseObject):
    __slots__ = ()


# ast-grep-ignore: no-long-names
class ChatCompletionCustomToolCallPayload(_CustomToolCallAccess):
    name: str
    input: str


# ast-grep-ignore: no-long-names
class ChatCompletionDeltaCustomToolCallPayload(_CustomToolCallAccess):
    name: str | None
    input: str | None
    defaults: ClassVar[dict[str, StoredValue]] = {"name": None, "input": None}


# ast-grep-ignore: no-long-names
class ChatCompletionMessageCustomToolCall(_CustomToolCallAccess):
    id: str
    type: Literal["custom"]
    custom: ChatCompletionCustomToolCallPayload  # ast-grep-ignore: no-long-names
    defaults: ClassVar[dict[str, StoredValue]] = {"type": "custom"}


# ast-grep-ignore: no-long-names
class ChatCompletionDeltaCustomToolCall(_CustomToolCallAccess):
    id: str | None
    type: str | None
    custom: ChatCompletionDeltaCustomToolCallPayload  # ast-grep-ignore: no-long-names
    index: int
    defaults: ClassVar[dict[str, StoredValue]] = {"id": None, "type": None}


__all__ = [
    "ChatCompletionCustomToolCallPayload",
    "ChatCompletionDeltaCustomToolCall",
    "ChatCompletionDeltaCustomToolCallPayload",
    "ChatCompletionMessageCustomToolCall",
    "_CustomToolCallAccess",
]
