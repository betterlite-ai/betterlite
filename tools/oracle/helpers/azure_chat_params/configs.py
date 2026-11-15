from typing import Final, Protocol

import httpx
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.azure.chat.gpt_5_transformation import AzureOpenAIGPT5Config
from litellm.llms.azure.chat.o_series_transformation import AzureOpenAIO1Config
from pydantic import BaseModel, JsonValue

from .views import CHAT_REPLY, OBJECT


class ParamsConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


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
O_SERIES_PARAMS: ParamsConfig = AzureOpenAIO1Config()
O_SERIES_STREAM: StreamConfig = AzureOpenAIO1Config()


class Recorder:
    def __init__(self) -> None:
        self.body: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.body = OBJECT.validate_json(request.content)
        return httpx.Response(200, json=CHAT_REPLY)
