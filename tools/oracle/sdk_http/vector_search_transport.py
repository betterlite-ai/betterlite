import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.vector_store.transformation import (
    BaseVectorStoreConfig,
    VectorStoreEmbeddingExecutor,
)
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.vector_stores.transformation import OpenAIVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode = Literal["sync", "async"]
Query = str | list[str]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
# the handler takes dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


# LiteLLM's search params spell filters and ranking_options as bare dict, which mypy reads as Any
class SearchBody(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


SearchCall = Callable[
    [
        str,
        Query,
        SearchBody,
        BaseVectorStoreConfig,
        str,
        GenericLiteLLMParams,
        Logging,
        VectorStoreEmbeddingExecutor | None,
        dict[str, object] | None,
        dict[str, object] | None,
        float | None,
    ],
    object,
]

AsyncSearchCall = Callable[
    [
        str,
        Query,
        SearchBody,
        BaseVectorStoreConfig,
        str,
        GenericLiteLLMParams,
        Logging,
        VectorStoreEmbeddingExecutor | None,
        dict[str, object] | None,
        dict[str, object] | None,
        float | None,
    ],
    Coroutine[object, object, object],
]


class Handlers(Protocol):
    @property
    def vector_store_search_handler(self) -> SearchCall: ...

    @property
    def async_vector_store_search_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncSearchCall: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


class OptionalParams(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filters: dict[str, JsonValue] | None = None
    max_num_results: int | None = None
    ranking_options: dict[str, JsonValue] | None = None
    rewrite_query: bool | None = None


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    vector_store_id: str
    query: Query
    optional_params: OptionalParams = Field(default_factory=OptionalParams)
    extra_headers: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: int | None = None


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str]


class Outcome(BaseModel):
    result: JsonValue = None
    failure: Failure | None = None


class TransportValue(BaseModel):
    value: Outcome


class TransportEnvelope(BaseModel):
    vector_search_transport: TransportValue


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="vector_store_search",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="vector-search-transport",
        function_id="vector-search-transport",
    )


# VectorStoreSearchRequest reads each key with .get(key, None), so null and absent are one body
def requested_params(optional: OptionalParams) -> SearchBody:
    params: SearchBody = {
        "filters": optional.filters,
        "max_num_results": optional.max_num_results,
        "ranking_options": optional.ranking_options,
        "rewrite_query": optional.rewrite_query,
    }
    return params


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    config: BaseVectorStoreConfig = OpenAIVectorStoreConfig()
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    try:
        result = execute(call, config, timeout, api_base)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


def execute(
    call: TransportCall,
    config: BaseVectorStoreConfig,
    timeout: float | None,
    api_base: str,
) -> JsonValue:
    logging_obj = logging_fixture()
    litellm_params = GenericLiteLLMParams(api_base=api_base)
    extra_headers = OBJECTS.validate_python(call.extra_headers)
    extra_body = OBJECTS.validate_python(call.extra_body)
    optional = requested_params(call.optional_params)
    if call.mode == "sync":
        return JSON.validate_python(
            HANDLER.vector_store_search_handler(
                call.vector_store_id,
                call.query,
                optional,
                config,
                "openai",
                litellm_params,
                logging_obj,
                None,
                extra_headers,
                extra_body,
                timeout,
            )
        )
    return JSON.validate_python(
        asyncio.run(
            HANDLER.async_vector_store_search_handler(
                call.vector_store_id,
                call.query,
                optional,
                config,
                "openai",
                litellm_params,
                logging_obj,
                None,
                extra_headers,
                extra_body,
                timeout,
            )
        )
    )


class VectorSearchTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_search_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.vector_search_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(vector_search_transport=value).model_dump_json(by_alias=True)
