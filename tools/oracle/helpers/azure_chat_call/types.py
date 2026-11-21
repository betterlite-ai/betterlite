from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ModelResponse
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

API_BASE: Final = "https://unit.test"
API_VERSION: Final = "2024-02-01"
MODEL: Final = "gpt-4o-deployment"
TIMEOUT: Final = 30.0
TIMEOUT_DIAGNOSTIC: Final = "- timeout value="
CHAT_BODY: Final = (
    '{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,'
    '"model":"gpt-4o","choices":[{"index":0,'
    '"message":{"role":"assistant","content":"Hello!"},"finish_reason":"stop"}],'
    '"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}'
)
PIN: Final = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/"
KINDS: Final[dict[str, str]] = {
    "AzureOpenAI": "azure_sync",
    "AsyncAzureOpenAI": "azure_async",
    "OpenAI": "openai_sync",
    "AsyncOpenAI": "openai_async",
}

AzureClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI
AnyClient: TypeAlias = AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI


class Turn(TypedDict, total=False):
    role: str
    content: str


class Answer(TypedDict, total=False):
    error: Literal["status", "bad_request", "timeout", "other", "pending"]
    status: int
    message: str
    headers: dict[str, str]
    body: str


class CallCase(TypedDict, total=False):
    model: str | None
    messages: list[Turn] | None
    api_key: str | None
    client_api_key: str | None
    azure_ad_token: str | None
    api_base: str
    api_version: str
    timeout: float
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    base_model: str | None
    organization: str | None
    drop_params: bool | None
    acompletion: bool
    client: Literal["sync", "async"]
    supplied: bool
    headers: dict[str, str] | None
    default_max_retries: int
    openai_system_messages_first: bool
    cancel: bool
    answer: Answer


class Input(TypedDict):
    function: Literal["completion", "o_series"]
    cases: list[CallCase]


class CompletionKwargs(TypedDict):
    model: str | None
    messages: list[Turn] | None
    model_response: ModelResponse
    api_key: str | None
    api_base: str
    api_version: str
    api_type: str
    azure_ad_token: str | None
    azure_ad_token_provider: Callable[[], str] | None
    dynamic_params: bool
    print_verbose: Callable[[str], None]
    timeout: float
    logging_obj: Logging
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    logger_fn: None
    acompletion: bool
    headers: dict[str, str] | None
    client: AzureClient | None


class OSeriesKwargs(TypedDict):
    model_response: ModelResponse
    timeout: float
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    logging_obj: Logging
    model: str | None
    messages: list[Turn] | None
    print_verbose: Callable[[str], None]
    api_key: str | None
    api_base: str
    api_version: str
    dynamic_params: bool | None
    azure_ad_token: str | None
    acompletion: bool
    logger_fn: None
    headers: dict[str, str] | None
    client: AnyClient | None
    organization: str | None
    custom_llm_provider: str | None
    drop_params: bool | None
    shared_session: None


class LiteLlmParamsView(TypedDict, total=False):
    azure_ad_token: JsonValue
    max_retries: JsonValue
    timeout: JsonValue


class CallArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    headers: dict[str, JsonValue] | None = None
    api_base: JsonValue = None
    api_version: str | None = None
    complete_input_dict: JsonValue = None
    openai_sdk: bool | None = None
    acompletion: bool = False


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    outcome: str
    reply: JsonValue = None
    error: JsonValue = None
    forward: JsonValue = None


class OriginalView(TypedDict, total=False):
    id: str


class MessageView(TypedDict, total=False):
    role: str
    content: str | None
    tool_calls: list[JsonValue] | None


class ChoiceView(TypedDict, total=False):
    index: int
    finish_reason: str | None
    message: MessageView


class UsageView(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyView(TypedDict, total=False):
    id: str
    model: str
    created: int
    choices: list[ChoiceView]
    usage: UsageView | None


class ErrorView(Protocol):
    status_code: int | None
    message: str
    headers: httpx.Headers | dict[str, str] | None
    body: JsonValue


class HiddenResponse(Protocol):
    _hidden_params: dict[str, JsonValue | httpx.Headers]


class JsonDump(Protocol):
    def model_dump_json(self) -> str: ...


class SyncChat(Protocol):
    def completion(self, **kwargs: Unpack[CompletionKwargs]) -> JsonDump: ...


class AsyncChat(Protocol):
    async def completion(self, **kwargs: Unpack[CompletionKwargs]) -> JsonDump: ...


class OSeriesChat(Protocol):
    def completion(self, **kwargs: Unpack[OSeriesKwargs]) -> JsonDump: ...


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
REPLY: Final[TypeAdapter[ReplyView]] = TypeAdapter(ReplyView)
ORIGINAL: Final[TypeAdapter[OriginalView]] = TypeAdapter(OriginalView)
CALL_ARGS: Final[TypeAdapter[CallArgs]] = TypeAdapter(CallArgs)
PARAMS_VIEW: Final[TypeAdapter[LiteLlmParamsView]] = TypeAdapter(LiteLlmParamsView)
