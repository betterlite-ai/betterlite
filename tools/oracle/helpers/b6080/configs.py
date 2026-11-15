from collections.abc import Callable
from typing import Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.openrouter.chat.transformation import OpenrouterConfig
from litellm.llms.openrouter.decisions.transformation import OpenRouterDecisionsConfig
from litellm.llms.openrouter.embedding.transformation import OpenrouterEmbeddingConfig
from litellm.llms.openrouter.responses.transformation import OpenRouterResponsesAPIConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import EmbeddingResponse, LlmProviders, ModelResponse
from pydantic import JsonValue

MODEL = "m"


# LiteLLM declares non_default_params as dict[str, object]
class Params(dict[str, object]): ...


class SupportedConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: Params,
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class ChatConfig(SupportedConfig, Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def transform_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/chat/transformation.py#L165-L224
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: LiteLLMLoggingObj,
        request_data: dict[str, JsonValue],
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
    ) -> ModelResponse: ...


class EmbeddingConfig(SupportedConfig, Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def transform_embedding_request(
        self,
        model: str,
        input_values: str | list[str] | list[int] | list[list[int]],
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    @property
    def validate_environment(self) -> Callable[
        [
            dict[str, str], str, list[JsonValue], dict[str, JsonValue],
            dict[str, JsonValue], str | None, str | None,
        ],
        dict[str, str],
    ]: ...

    def transform_embedding_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openrouter/embedding/transformation.py#L122-L145
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: EmbeddingResponse,
        logging_obj: LiteLLMLoggingObj,
        api_key: str | None,
        request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> EmbeddingResponse: ...


class ResponsesConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, str], model: str, litellm_params: GenericLiteLLMParams | None
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def supports_native_websocket(self) -> bool: ...

    @property
    def custom_llm_provider(self) -> LlmProviders: ...


CHAT: ChatConfig = OpenrouterConfig()
EMBEDDING: EmbeddingConfig = object.__new__(OpenrouterEmbeddingConfig)
RESPONSES: ResponsesConfig = object.__new__(OpenRouterResponsesAPIConfig)
DECISIONS = OpenRouterDecisionsConfig()
