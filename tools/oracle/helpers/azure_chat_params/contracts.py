from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.azure.chat.gpt_5_transformation import AzureOpenAIGPT5Config
from litellm.llms.azure.chat.o_series_transformation import AzureOpenAIO1Config
from pydantic import BaseModel, JsonValue, TypeAdapter

OBJECT = TypeAdapter(dict[str, JsonValue])
CHAT_REPLY: dict[str, JsonValue] = {
    "id": "chatcmpl-b532",
    "object": "chat.completion",
    "created": 1,
    "model": "m",
    "choices": [
        {
            "index": 0,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": "hi"},
        }
    ],
    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
}
AZURE_ENDPOINT = "https://azure-b532.invalid"
AZURE_VERSION = "2024-10-21"
AZURE_KEY = "b532-key"
PROJECTED: tuple[str, ...] = (
    "model",
    "messages",
    "tools",
    "tool_choice",
    "reasoning_effort",
)


class Install(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    chat_models: list[str]
    text_models: list[str]
    drop_params: bool
    openai_system_messages_first: bool


class AliasStep(TypedDict):
    model: str


class AliasCall(TypedDict):
    function: Literal["gpt5_alias_catalog"]
    install: Install
    steps: list[AliasStep]


class StreamStep(TypedDict):
    model: str | None
    stream: bool | None
    provider: str | None


class StreamCall(TypedDict):
    function: Literal["native_stream"]
    install: Install
    steps: list[StreamStep]


class AllowlistCall(TypedDict):
    function: Literal["gpt5_allowlist", "o_series_allowlist"]
    install: Install
    models: list[str]


class MapStep(TypedDict):
    model: str
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]
    drop_params: bool
    global_drop_params: bool


class MapCall(TypedDict):
    function: Literal["parameter_mapping"]
    install: Install
    steps: list[MapStep]


class TransformStep(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    tools: list[dict[str, JsonValue]] | None
    tool_choice: str | None
    reasoning_effort: (
        Literal["none", "minimal", "low", "medium", "high", "xhigh", "max", "default"]
        | None
    )


class TransformCall(TypedDict):
    function: Literal["request_transform"]
    install: Install
    steps: list[TransformStep]


Input: TypeAlias = AliasCall | StreamCall | AllowlistCall | MapCall | TransformCall
INPUT = TypeAdapter(Input)


class AllowlistConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...


class ParamsConfig(AllowlistConfig, Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def is_model_gpt_5_model(self, model: str) -> bool: ...

    def effort_resolves_to_none(self, model: str, effective_effort: str | None) -> bool: ...


class StreamConfig(Protocol):
    def should_fake_stream(
        self,
        *,
        model: str | None,
        stream: bool | None,
        custom_llm_provider: str | None = None,
    ) -> bool: ...


class Failure(BaseModel):
    status_code: int | None


# LiteLLM's exception class carries Any, so the catch names it through a typed alias
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError

GPT5: ParamsConfig = AzureOpenAIGPT5Config()
O_SERIES_PARAMS: AllowlistConfig = AzureOpenAIO1Config()
O_SERIES_STREAM: StreamConfig = AzureOpenAIO1Config()


class Recorder:
    def __init__(self) -> None:
        self.body: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.body = OBJECT.validate_json(request.content)
        return httpx.Response(200, json=CHAT_REPLY)
