from collections.abc import Callable
from datetime import UTC, datetime
from typing import Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.hosted_vllm.chat.transformation import HostedVLLMChatConfig
from litellm.llms.hosted_vllm.embedding.transformation import HostedVLLMEmbeddingConfig
from litellm.types.llms.openai import AllEmbeddingInputValues, AllMessageValues
from litellm.types.utils import CallTypes, EmbeddingResponse
from pydantic import JsonValue

from .views import CHAT_REPLY, OBJECT


class NoMessages(list[AllMessageValues]):
    pass


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class EmbeddingConfig(Protocol):
    def validate_environment(
        self,
        _headers: dict[str, str],
        _model: str,
        _messages: list[AllMessageValues],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _api_key: str | None = None,
        _api_base: str | None = None,
        /,
    ) -> dict[str, JsonValue]: ...  # positional-only: LiteLLM names them api_key and api_base

    def get_complete_url(
        self,
        _api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
        *,
        stream: bool | None = None,
    ) -> str: ...

    def transform_embedding_request(
        self,
        model: str,
        _input: AllEmbeddingInputValues,
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def transform_embedding_response(
        self,
        _model: str,
        _raw_response: httpx.Response,
        _model_response: EmbeddingResponse,
        _logging_obj: Logging,
        _api_key: str | None,
        _request_data: dict[str, JsonValue],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
    ) -> EmbeddingResponse: ...


CHAT: ChatConfig = HostedVLLMChatConfig()
# the embedding config inherits an untyped __init__, so bind the class as a typed factory
EMBEDDING_FACTORY: Callable[[], EmbeddingConfig] = HostedVLLMEmbeddingConfig
EMBEDDING: EmbeddingConfig = EMBEDDING_FACTORY()


class Recorder:
    def __init__(self) -> None:
        self.body: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.body = OBJECT.validate_json(request.content)
        return httpx.Response(200, json=CHAT_REPLY)


def logging_object() -> Logging:
    return Logging(
        model="m",
        messages=None,
        stream=False,
        call_type=CallTypes.embedding,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b8150",
        function_id="b8150",
    )
