from typing import Final, NotRequired, TypeAlias, TypedDict

from litellm.llms.azure_ai.agents.handler import (
    _AzureMessagesData,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L78-L79
    _AzureRawAnnotation,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L54-L57
)
from litellm.types.llms.openai import ChatCompletionAnnotation
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
JSON: Final = TypeAdapter(JsonValue)


class ExtractionCall(TypedDict):
    messages: _AzureMessagesData


class AnnotationCall(TypedDict):
    annotations: list[_AzureRawAnnotation]


class ExistingUsage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyCall(TypedDict):
    model: str
    content: str
    thread_id: str
    messages: list[JsonObject]
    annotations: list[ChatCompletionAnnotation] | None
    existing_usage: ExistingUsage | None
    hidden_params: JsonObject | None
    fail: bool


class StreamCall(TypedDict):
    model: str
    body: str
    id: str
    created: int
    split: int | None


class RequestCall(TypedDict):
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]
    api_base: str
    api_key: str
    status: int
    body: str
    id: str
    created: int


class StreamPlan(TypedDict):
    model: str
    messages: list[JsonObject]
    api_base: str
    api_key: str
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]


class Input(TypedDict, total=False):
    extractions: list[ExtractionCall]
    annotations: list[AnnotationCall]
    replies: list[ReplyCall]
    streams: list[StreamCall]
    requests: list[RequestCall]


class FieldsJson(TypedDict):
    role: NotRequired[JsonValue]
    content: NotRequired[JsonValue]
    annotations: NotRequired[JsonValue]


class StreamingChoiceJson(TypedDict):
    index: JsonValue
    finish_reason: JsonValue
    delta: FieldsJson


class ChunkJson(TypedDict):
    id: JsonValue
    created: JsonValue
    model: JsonValue
    object: JsonValue
    choices: list[StreamingChoiceJson]


class ReplyChoiceJson(TypedDict):
    index: JsonValue
    finish_reason: JsonValue
    message: FieldsJson


class ReplyJson(TypedDict):
    model: JsonValue
    choices: list[ReplyChoiceJson]
    usage: NotRequired[JsonObject]


CHUNK_JSON: Final = TypeAdapter(ChunkJson)
REPLY_JSON: Final = TypeAdapter(ReplyJson)
