from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
from pydantic import JsonValue

from .wire import ReplyCase, RequestInput, ResponseInput

_ = litellm


class ChatParamsCase(TypedDict):
    model: str
    requested: dict[str, JsonValue]


class ChatParamsInput(TypedDict):
    op: Literal["chat_params"]
    cases: list[ChatParamsCase]


class ChunkCase(TypedDict):
    chunk: dict[str, JsonValue]


class ChunkInput(TypedDict):
    op: Literal["chunk"]
    cases: list[ChunkCase]


class FunctionOut(TypedDict):
    name: str
    arguments: str


class ToolCallOut(TypedDict):
    index: int
    type: str
    id: str
    function: FunctionOut


class DeltaOut(TypedDict):
    content: str | None
    reasoning_content: NotRequired[str | None]
    tool_calls: list[ToolCallOut] | None


class ChoiceOut(TypedDict):
    index: int
    finish_reason: str | None
    delta: DeltaOut


class ChunkOut(TypedDict):
    id: str
    object: str
    created: int
    model: str
    choices: list[ChoiceOut]
    usage: dict[str, JsonValue] | None


class EmbeddingCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    environment: dict[str, str]
    api_base: str | None
    model: str
    input: str | list[str] | list[int] | list[list[int]]
    optional_params: dict[str, JsonValue]
    non_default_params: dict[str, JsonValue]


class EmbeddingInput(TypedDict):
    op: Literal["embedding"]
    cases: list[EmbeddingCase]
    replies: list[ReplyCase]


class ResponsesCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    global_api_key: str | None
    environment: dict[str, str]
    api_base: str | None
    global_api_base: str | None


class ResponsesInput(TypedDict):
    op: Literal["responses"]
    cases: list[ResponsesCase]


class DecisionsCase(TypedDict):
    cost: JsonValue


class DecisionsInput(TypedDict):
    op: Literal["decisions"]
    cases: list[DecisionsCase]


Input: TypeAlias = (
    ChatParamsInput | ChunkInput | EmbeddingInput | ResponsesInput
    | DecisionsInput | RequestInput | ResponseInput
)
