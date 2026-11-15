from typing import Literal, TypeAlias, TypedDict

from litellm.types.llms.openai import AllEmbeddingInputValues
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

OBJECT = TypeAdapter(dict[str, JsonValue])
CHAT_REPLY: dict[str, JsonValue] = {
    "choices": [
        {
            "index": 0,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": "hi"},
        }
    ],
    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
}


class SupportedCall(TypedDict):
    model: str


class MapCall(TypedDict):
    model: str
    params: dict[str, JsonValue]
    preset: dict[str, JsonValue]


class MessagesCall(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]


class HeadersCall(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    env: dict[str, str]


class UrlCall(TypedDict):
    api_base: str | None
    env: dict[str, str]


class RequestCall(TypedDict):
    model: str
    input: AllEmbeddingInputValues
    optional_params: dict[str, JsonValue]


class ResponseCall(TypedDict):
    body: str


class ChatSupportedInput(TypedDict):
    function: Literal["chat_supported"]
    calls: list[SupportedCall]


class ChatMapInput(TypedDict):
    function: Literal["chat_map"]
    calls: list[MapCall]


class ChatMessagesInput(TypedDict):
    function: Literal["chat_messages"]
    calls: list[MessagesCall]


class EmbeddingHeadersInput(TypedDict):
    function: Literal["embedding_headers"]
    calls: list[HeadersCall]


class EmbeddingUrlInput(TypedDict):
    function: Literal["embedding_url"]
    calls: list[UrlCall]


class EmbeddingRequestInput(TypedDict):
    function: Literal["embedding_request"]
    calls: list[RequestCall]


class EmbeddingResponseInput(TypedDict):
    function: Literal["embedding_response"]
    calls: list[ResponseCall]


Input: TypeAlias = (
    ChatSupportedInput
    | ChatMapInput
    | ChatMessagesInput
    | EmbeddingHeadersInput
    | EmbeddingUrlInput
    | EmbeddingRequestInput
    | EmbeddingResponseInput
)
INPUT = TypeAdapter(Input)


class UsageCounters(BaseModel):
    model_config = ConfigDict(extra="ignore")
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0


class EmbeddingReply(BaseModel):
    model_config = ConfigDict(extra="ignore")
    model: str
    object: str
    data: list[JsonValue]
    usage: JsonValue = None


REPLY = TypeAdapter(EmbeddingReply)
USAGE = TypeAdapter(UsageCounters | None)
