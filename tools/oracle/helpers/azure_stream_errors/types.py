from collections.abc import AsyncIterator, Callable, Iterator
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

MODEL: Final = "deployment"
API_BASE: Final = "https://unit.test"
API_VERSION: Final = "2024-02-01"
CREATED_FLOOR: Final = 1_000_000_000
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
STATUS: Final[TypeAdapter[int | None]] = TypeAdapter(int | None)
TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
HEADER_MAP: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])
OPTIONAL_HEADER_MAP: Final[TypeAdapter[dict[str, str] | None]] = TypeAdapter(
    dict[str, str] | None
)


class Turn(TypedDict):
    role: str
    content: str


class Presence(TypedDict):
    present: bool
    value: NotRequired[JsonValue]


class SendSpec(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    events: list[str]
    body: str


class ExchangeSpec(TypedDict, total=False):
    environment: dict[str, str]
    status: int
    body: str
    assertion: str


class ErrorSpec(TypedDict, total=False):
    text: str
    status: Presence
    message: Presence
    headers: Presence
    body: Presence
    response: Presence


class ClientSpec(TypedDict, total=False):
    kind: Literal["azure_sync", "azure_async", "open_ai_sync", "open_ai_async"]
    api_key: str


class StreamCase(TypedDict, total=False):
    model: str
    api_base: str
    api_version: str
    api_key: str | None
    azure_ad_token: str | None
    azure_ad_token_provider: int
    timeout: float | None
    max_retries: int
    dynamic_params: bool
    data: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    supplied: ClientSpec
    get_error: ErrorSpec
    send: SendSpec
    exchange: ExchangeSpec


class Input(TypedDict):
    function: Literal["streaming", "async_streaming"]
    cases: list[StreamCase]


class GetRecord(TypedDict):
    api_version: str
    api_base: str
    api_key: str | None
    model: str
    is_async: bool
    litellm_params: JsonValue


class WrapperView(Protocol):
    model: str
    custom_llm_provider: str | None
    stream_options: dict[str, JsonValue] | None
    _response_headers: dict[str, str] | None
    _hidden_params: dict[str, JsonValue]


class ErrorView(Protocol):
    status_code: int | None
    message: str | None
    headers: httpx.Headers | dict[str, str] | None
    body: JsonValue


class PreArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_base: str | None = None
    acompletion: bool = False
    complete_input_dict: dict[str, JsonValue] | None = None


PRE_ARGS: Final[TypeAdapter[PreArgs]] = TypeAdapter(PreArgs)


class ChunkDelta(TypedDict, total=False):
    content: str
    role: str


class ChunkChoice(TypedDict):
    delta: ChunkDelta
    finish_reason: str | None
    index: int


class ChunkView(TypedDict):
    model: str
    created: int
    choices: list[ChunkChoice]


CHUNK: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)


class JsonDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncStream(Protocol):
    def __iter__(self) -> Iterator[JsonDump]: ...


class AsyncStream(Protocol):
    def __aiter__(self) -> AsyncIterator[JsonDump]: ...


class StreamKwargs(TypedDict):
    logging_obj: Logging
    api_base: str
    api_key: str | None
    api_version: str
    dynamic_params: bool
    data: dict[str, JsonValue]
    model: str
    timeout: float | None
    max_retries: int
    azure_ad_token: str | None
    azure_ad_token_provider: Callable[[], str] | None
    client: AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None
    litellm_params: dict[str, JsonValue]


class SyncChat(Protocol):
    def streaming(self, **kwargs: Unpack[StreamKwargs]) -> JsonDump: ...


class AsyncChat(Protocol):
    async def async_streaming(self, **kwargs: Unpack[StreamKwargs]) -> JsonDump: ...
