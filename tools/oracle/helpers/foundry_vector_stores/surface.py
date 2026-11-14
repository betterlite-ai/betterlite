import asyncio
import datetime as dt
from collections.abc import Callable, Mapping
from typing import Protocol, TypeAlias

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.vector_stores.transformation import AzureAIVectorStoreConfig
from litellm.llms.base_llm.vector_store.transformation import VectorStoreEmbeddingExecutor
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

litellm.enable_azure_ad_token_refresh = False

Fields: TypeAlias = Mapping[str, object]
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
DETAILS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Settings(Protocol):
    def get_auth_credentials(self, litellm_params: dict[str, JsonValue]) -> Fields: ...

    def validate_environment(
        self, headers: dict[str, str], litellm_params: GenericLiteLLMParams
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_search_vector_store_response(
        self, response: httpx.Response, litellm_logging_obj: Logging
    ) -> Fields: ...


make_config: Callable[[], AzureAIVectorStoreConfig] = AzureAIVectorStoreConfig
CONFIG: AzureAIVectorStoreConfig = make_config()
SETTINGS: Settings = CONFIG


class CallDetails(BaseModel):
    input: str | None = None
    embedding_model: str | None = None
    top_k: int | None = None


class Logged(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: CallDetails


class Fixture(VectorStoreEmbeddingExecutor):
    def __init__(self, vectors: list[list[float]]) -> None:
        self.vectors = vectors
        self.calls: list[JsonValue] = []

    def record(self, model: str, query: str, configuration: Fields) -> EmbeddingResponse:
        recorded = DETAILS.validate_python(dict(configuration))
        call: dict[str, JsonValue] = {
            "model": model,
            "query": query,
            "configuration": recorded,
        }
        self.calls.append(call)
        index = len(self.calls) - 1
        vector: list[float] = []
        if index < len(self.vectors):
            vector = self.vectors[index]
        elif self.vectors:
            vector = self.vectors[-1]
        embedding: list[JsonValue] = list(vector)
        row: dict[str, JsonValue] = {"object": "embedding", "embedding": embedding, "index": 0}
        data: list[dict[str, JsonValue]] = [row]
        return EmbeddingResponse(model="fixture-model", data=data)

    @override
    def embed(self, model: str, query: str, configuration: Fields) -> EmbeddingResponse:
        return self.record(model, query, configuration)

    @override
    async def aembed(self, model: str, query: str, configuration: Fields) -> EmbeddingResponse:
        await asyncio.sleep(0)
        return self.record(model, query, configuration)


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="vector_store_search",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="foundry-vector-stores",
        function_id="foundry-vector-stores",
    )
