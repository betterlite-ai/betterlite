from collections.abc import AsyncIterator, Awaitable
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
JSON: Final = TypeAdapter(JsonValue)
ConfigProduct: TypeAlias = (
    ModelResponse | Awaitable[ModelResponse] | AsyncIterator[ModelResponseStream]
)


class Step(TypedDict):
    status: int
    body: str


class RunCall(TypedDict):
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]
    api_base: str
    api_key: str
    mode: Literal["sync", "async"]
    supplied: bool
    steps: list[Step]


class DispatchCall(TypedDict):
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]
    api_base: str
    api_key: str | None
    token: str | None
    acompletion: bool
    stream: bool
    steps: list[Step]


class Input(TypedDict, total=False):
    runs: list[RunCall]
    dispatches: list[DispatchCall]


class MessageDump(TypedDict, total=False):
    role: JsonValue
    content: JsonValue
    annotations: JsonValue


class ChoiceDump(TypedDict):
    index: JsonValue
    finish_reason: JsonValue
    message: MessageDump


class UsageDump(TypedDict, total=False):
    prompt_tokens: JsonValue
    completion_tokens: JsonValue
    total_tokens: JsonValue


class ReplyDump(TypedDict):
    model: JsonValue
    choices: list[ChoiceDump]
    usage: NotRequired[UsageDump | None]


REPLY_DUMP: Final = TypeAdapter(ReplyDump)
