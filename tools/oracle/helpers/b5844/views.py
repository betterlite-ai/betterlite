from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, overload

import httpx
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue

Object: TypeAlias = dict[str, JsonValue]


class UrlStep(TypedDict):
    kind: Literal["url"]
    model: str
    embedding: bool
    api_base: str | None
    account_env: str | None
    params: Object


class HeadersStep(TypedDict):
    kind: Literal["headers"]
    model: str
    api_key: str | None
    headers: Object


class ParamsStep(TypedDict):
    kind: Literal["params"]
    model: str
    embedding: bool
    non_default: Object


class InfoStep(TypedDict):
    kind: Literal["info"]
    api_base: str | None
    api_key: str | None
    jwt_env: str | None


class RequestStep(TypedDict):
    kind: Literal["request"]
    model: str
    messages: list[Object]
    options: Object


class ResponseStep(TypedDict):
    kind: Literal["response"]
    model: str
    body: str
    request_data: Object
    messages: list[Object]


class EmbeddingRequestStep(TypedDict):
    kind: Literal["embedding_request"]
    model: str
    text: str | list[str]
    options: Object


class EmbeddingResponseStep(TypedDict):
    kind: Literal["embedding_response"]
    model: str
    body: str


class ChoicesChunk(TypedDict):
    choices: list[Object]


class ChunkStep(TypedDict):
    kind: Literal["chunk"]
    chunk: ChoicesChunk


class EventStep(TypedDict):
    kind: Literal["event"]
    chunk: Object


Step: TypeAlias = (
    UrlStep
    | HeadersStep
    | ParamsStep
    | InfoStep
    | RequestStep
    | ResponseStep
    | EmbeddingRequestStep
    | EmbeddingResponseStep
    | ChunkStep
    | EventStep
)


class Input(TypedDict):
    steps: list[Step]


class UrlArgs(TypedDict):
    api_base: str | None
    api_key: None
    model: str
    optional_params: Object
    litellm_params: Object


class HeaderArgs(TypedDict):
    headers: Object
    model: str
    messages: list[Object]
    optional_params: Object
    litellm_params: Object
    api_key: str | None


class MapArgs(TypedDict):
    non_default_params: Object
    optional_params: Object
    model: str
    drop_params: bool


class RequestArgs(TypedDict):
    model: str
    messages: list[Object]
    optional_params: Object
    litellm_params: Object
    headers: Object


class PostCallArgs(TypedDict):
    input: JsonValue
    api_key: str
    original_response: JsonValue
    additional_args: Object


class Recorder:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []

    def post_call(self, **kwargs: Unpack[PostCallArgs]) -> None:
        self.calls.append(
            {
                "input": kwargs["input"],
                "api_key": kwargs["api_key"],
                "original_response": kwargs["original_response"],
                "complete_input_dict": kwargs["additional_args"]["complete_input_dict"],
            }
        )


class ResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Recorder
    request_data: Object
    messages: list[Object]
    optional_params: Object
    litellm_params: Object
    encoding: None


class EmbedRequestArgs(TypedDict):
    model: str
    input: str | list[str]
    optional_params: Object
    headers: Object


class EmbedResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: EmbeddingResponse
    logging_obj: None
    api_key: None
    request_data: Object
    optional_params: Object
    litellm_params: Object


class ToolFunctionView(TypedDict):
    name: str | None
    arguments: str | None


class ToolCallView(TypedDict):
    id: str | None
    type: str
    function: ToolFunctionView


class MessageView(TypedDict):
    role: str
    content: str | None
    tool_calls: list[ToolCallView] | None


class ChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageView


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyView(TypedDict):
    id: str
    model: str
    choices: list[ChoiceView]
    usage: UsageView


class EmbeddingItemView(TypedDict):
    index: int
    embedding: list[JsonValue]


class EmbeddingReplyView(TypedDict):
    model: str
    data: list[EmbeddingItemView]
    usage: UsageView


class OpenAiChunkView(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    usage: JsonValue | None
    index: int
    tool_use: JsonValue | None


class ToolDeltaView(TypedDict):
    id: str | None
    type: str
    index: int
    function: ToolFunctionView


class DeltaView(TypedDict):
    content: str | None
    tool_calls: list[ToolDeltaView] | None


class EventChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    delta: DeltaView


class EventView(TypedDict):
    model: str | None
    choices: list[EventChoiceView]
    usage: UsageView | None


class Reply(Protocol):
    def model_dump_json(self) -> str: ...

    def get(self, key: Literal["_hidden_params"]) -> Object: ...


class StreamHandler(Protocol):
    @overload
    def chunk_parser(self, chunk: ChoicesChunk) -> OpenAiChunkView: ...

    @overload
    def chunk_parser(self, chunk: Object) -> Reply: ...


class UrlConfig(Protocol):
    def get_complete_url(self, **kwargs: Unpack[UrlArgs]) -> str: ...


class ParamsConfig(Protocol):
    def get_supported_openai_params(self, *, model: str) -> list[JsonValue]: ...

    def map_openai_params(self, **kwargs: Unpack[MapArgs]) -> Object: ...


class ChatView(UrlConfig, ParamsConfig, Protocol):
    def validate_environment(self, **kwargs: Unpack[HeaderArgs]) -> Object: ...

    def transform_request(self, **kwargs: Unpack[RequestArgs]) -> Object: ...

    def transform_response(self, **kwargs: Unpack[ResponseArgs]) -> Reply: ...

    def get_model_response_iterator(
        self, *, streaming_response: None, sync_stream: bool, json_mode: bool
    ) -> StreamHandler: ...


class EmbeddingView(UrlConfig, ParamsConfig, Protocol):
    def transform_embedding_request(self, **kwargs: Unpack[EmbedRequestArgs]) -> Object: ...

    def transform_embedding_response(self, **kwargs: Unpack[EmbedResponseArgs]) -> Reply: ...


Pair: TypeAlias = tuple[ChatView, EmbeddingView]
