import os
from collections.abc import Mapping, Sequence
from typing import Final, Literal, Never, Protocol, TypeAlias

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter

Document: TypeAlias = str | dict[str, object]
JsonObject: TypeAlias = dict[str, JsonValue]
Kind: TypeAlias = Literal["retrieval", "ranking"]
JSON: Final = TypeAdapter(JsonValue)


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self, non_default_params: JsonObject, optional_params: JsonObject, model: str,
        *, drop_params: bool
    ) -> JsonObject: ...


class EmbeddingConfig(Protocol):
    def get_supported_openai_params(self) -> list[str]: ...

    def map_openai_params(
        self, non_default_params: JsonObject, optional_params: JsonObject,
        kwargs: JsonObject | None
    ) -> JsonObject: ...


class RerankConfig(Protocol):
    def get_complete_url(self, api_base: str | None, model: str) -> str: ...

    def map_cohere_rerank_params(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/rerank/transformation.py#L112-L126
        self,
        *,
        non_default_params: JsonObject, model: str, drop_params: bool,
        query: str, documents: list[Document], top_n: int | None
    ) -> JsonObject: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None
    ) -> dict[str, str]: ...

    def transform_rerank_request(
        self, model: str, optional_rerank_params: JsonObject, headers: dict[str, str]
    ) -> JsonObject: ...

    def transform_rerank_response(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/rerank/transformation.py#L260-L270
        self,
        *,
        model: str, raw_response: httpx.Response, model_response: RerankResponse,
        logging_obj: Logging, request_data: JsonObject, optional_params: JsonObject
    ) -> RerankResponse: ...


class PassthroughConfig(Protocol):
    def get_complete_url(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/passthrough/transformation.py#L82-L91
        self,
        *,
        api_base: str | None, api_key: str | None, model: str, endpoint: str,
        request_query_params: JsonObject | None, litellm_params: JsonObject
    ) -> tuple[httpx.URL, str]: ...

    def is_streaming_request(self, endpoint: str, request_data: JsonObject) -> bool: ...

    def validate_environment(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_nim/passthrough/transformation.py#L98-L114
        self,
        *,
        headers: Mapping[str, str], model: str, messages: Sequence[Never],
        optional_params: Mapping[str, JsonValue], litellm_params: Mapping[str, JsonValue],
        api_key: str | None = None
    ) -> dict[str, str]: ...


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value
