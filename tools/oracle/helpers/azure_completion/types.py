from collections.abc import AsyncIterator, Callable, Iterator
from typing import Final, Literal, Protocol, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ModelResponse
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

MODEL: Final = "gpt-35-turbo-instruct"
API_BASE: Final = "https://unit.test"
API_VERSION: Final = "2024-02-01"
CREATED_FLOOR: Final = 1_000_000_000
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Turn(TypedDict):
    role: str
    content: str | list[dict[str, JsonValue]]


class Answer(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    body: str
    events: list[str]


class CallCase(TypedDict, total=False):
    model: str | None
    messages: list[Turn] | None
    api_key: str | None
    client_api_key: str | None
    azure_ad_token: str | None
    api_base: str | None
    api_version: str | None
    timeout: float | None
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    acompletion: bool
    client: Literal["sync", "async"]
    headers: dict[str, str] | None
    data: dict[str, JsonValue]
    answer: Answer


class CompletionInput(TypedDict):
    function: Literal["completion"]
    cases: list[CallCase]


class StreamInput(TypedDict):
    function: Literal["stream"]
    cases: list[CallCase]


class AuthCase(TypedDict):
    api_key: str | None
    azure_ad_token: str | None


class AuthInput(TypedDict):
    function: Literal["auth"]
    cases: list[AuthCase]


class ConfigConstructor(TypedDict, total=False):
    frequency_penalty: int | None
    logit_bias: dict[str, int] | None
    max_tokens: int | None
    n: int | None
    presence_penalty: int | None
    stop: str | list[str] | None
    temperature: int | None
    top_p: int | None


class ConfigInput(TypedDict):
    function: Literal["config"]
    constructor: ConfigConstructor


Input = CompletionInput | StreamInput | AuthInput | ConfigInput


class ConfigView(Protocol):
    frequency_penalty: int | None
    logit_bias: dict[str, int] | None
    max_tokens: int | None
    n: int | None
    presence_penalty: int | None
    stop: str | list[str] | None
    temperature: int | None
    top_p: int | None


class PreArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_version: str | None = None
    api_base: str | tuple[str | int | None, ...] | None = None
    complete_input_dict: dict[str, JsonValue] | None = None
    acompletion: bool = False


class PostArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_version: str | None = None
    api_base: str | None = None


class OriginalView(TypedDict):
    id: str


class ErrorView(Protocol):
    status_code: int | None
    message: str
    headers: httpx.Headers | dict[str, str] | None


class JsonDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncStream(Protocol):
    def __iter__(self) -> Iterator[JsonDump]: ...

    def __next__(self) -> JsonDump: ...


class AsyncStream(Protocol):
    def __aiter__(self) -> AsyncIterator[JsonDump]: ...

    async def __anext__(self) -> JsonDump: ...


class ReplyChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    message: dict[str, JsonValue]


class DetailsView(TypedDict, total=False):
    reasoning_tokens: JsonValue


class UsageView(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    reasoning_tokens: JsonValue
    queue_time: JsonValue
    completion_tokens_details: DetailsView | None


class ReplyView(TypedDict):
    id: str
    model: str
    created: int
    choices: list[ReplyChoiceView]
    usage: UsageView | None


class DeltaView(TypedDict, total=False):
    content: str | None
    role: str | None


class ChunkChoiceView(TypedDict):
    index: int
    delta: DeltaView
    finish_reason: str | None


class ChunkView(TypedDict):
    model: str
    created: int
    choices: list[ChunkChoiceView]


class CompletionKwargs(TypedDict):
    model: str
    messages: list[Turn]
    model_response: ModelResponse
    api_key: str | None
    api_base: str
    api_version: str
    api_type: str
    azure_ad_token: str | None
    azure_ad_token_provider: None
    print_verbose: Callable[[str], None]
    timeout: float | None
    logging_obj: Logging
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    logger_fn: None
    acompletion: bool
    headers: dict[str, str] | None
    client: AzureOpenAI | AsyncAzureOpenAI


class StreamKwargs(TypedDict):
    logging_obj: Logging
    api_base: str
    api_key: str | None
    api_version: str
    data: dict[str, JsonValue]
    model: str
    timeout: float | None
    azure_ad_token: str | None
    client: AzureOpenAI | AsyncAzureOpenAI
    litellm_params: dict[str, JsonValue]


class SyncTextCompletion(Protocol):
    def validate_environment(
        self, api_key: str | None, azure_ad_token: str | None
    ) -> dict[str, JsonValue]: ...

    def completion(self, **kwargs: Unpack[CompletionKwargs]) -> SyncStream | JsonDump: ...

    def streaming(self, **kwargs: Unpack[StreamKwargs]) -> SyncStream: ...


class AsyncTextCompletion(Protocol):
    async def completion(self, **kwargs: Unpack[CompletionKwargs]) -> AsyncStream | JsonDump: ...

    async def async_streaming(self, **kwargs: Unpack[StreamKwargs]) -> AsyncStream: ...


REPLY: Final[TypeAdapter[ReplyView]] = TypeAdapter(ReplyView)
CHUNK: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)
ORIGINAL: Final[TypeAdapter[OriginalView]] = TypeAdapter(OriginalView)
PRE_ARGS: Final[TypeAdapter[PreArgs]] = TypeAdapter(PreArgs)
POST_ARGS: Final[TypeAdapter[PostArgs]] = TypeAdapter(PostArgs)
