from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import LlmProviders
from litellm.types.vector_stores import (
    VectorStoreChunkingStrategy,
    VectorStoreCreateResponse,
    VectorStoreExpirationPolicy,
)
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])


class SearchOptions(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


class CreateOptions(TypedDict, total=False):
    name: str | None
    file_ids: list[str] | None
    expires_after: VectorStoreExpirationPolicy | None
    chunking_strategy: VectorStoreChunkingStrategy | None
    metadata: dict[str, str] | None


class SearchContent(TypedDict, total=False):
    text: str | None
    type: str | None


class SearchHit(TypedDict, total=False):
    score: float | None
    content: list[SearchContent] | None
    file_id: str | None
    filename: str | None
    attributes: dict[str, JsonValue] | None


class SearchReply(TypedDict, total=False):
    object: Literal["vector_store.search_results.page"]
    search_query: str | None
    data: list[SearchHit] | None


class VertexConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vector_stores/rag_api/transformation.py#L155-L216
        self,
        vector_store_id: str,
        query: str | list[str],
        # ast-grep-ignore: no-long-names
        vector_store_search_optional_params: SearchOptions,
        api_base: str,
        litellm_logging_obj: Logging,
        litellm_params: dict[str, JsonValue],
    ) -> tuple[str, dict[str, object]]: ...  # ast-grep-ignore: no-loose-types

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_response(
        self, response: httpx.Response, litellm_logging_obj: Logging
    ) -> SearchReply: ...

    # ast-grep-ignore: no-long-names
    def transform_create_vector_store_request(
        self,
        # ast-grep-ignore: no-long-names
        vector_store_create_optional_params: CreateOptions,
        api_base: str,
    ) -> tuple[str, dict[str, object]]: ...  # ast-grep-ignore: no-loose-types

    # ast-grep-ignore: no-long-names
    def transform_create_vector_store_response(
        self, response: httpx.Response
    ) -> VectorStoreCreateResponse: ...


def config() -> VertexConfig:
    found = ProviderConfigManager.get_provider_vector_stores_config(
        provider=LlmProviders.VERTEX_AI, api_type="rag_api"
    )
    if found is None:
        raise ValueError("the Vertex RAG API vector store config is not registered")
    return found


class Reply(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class SearchCall(TypedDict):
    function: Literal["search"]
    vector_store_id: str
    query: str | list[str]
    api_base: str
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[SearchOptions]
    response: NotRequired[Reply]


class CreateRequestCall(TypedDict):
    function: Literal["create_request"]
    api_base: str
    optional_params: NotRequired[CreateOptions]


class CreateResponseCall(TypedDict):
    function: Literal["create_response"]
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class Input(TypedDict):
    calls: list[SearchCall | CreateRequestCall | CreateResponseCall]


def logging_object() -> Logging:
    return Logging(
        model="vertex_rag_store",
        messages=None,
        stream=False,
        call_type="vector_store_search",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def search_outcome(call: SearchCall) -> JsonValue:
    logging = logging_object()
    optional = call.get("optional_params")
    litellm_params = call.get("litellm_params")
    url, body = config().transform_search_vector_store_request(
        vector_store_id=call["vector_store_id"],
        query=call["query"],
        vector_store_search_optional_params=SearchOptions() if optional is None else optional,
        api_base=call["api_base"],
        litellm_logging_obj=logging,
        litellm_params={} if litellm_params is None else litellm_params,
    )
    outcome: dict[str, JsonValue] = {"url": url, "body": OBJECT.validate_python(body)}
    response = call.get("response")
    if response is not None:
        reply = httpx.Response(
            response["status"],
            headers=response.get("headers"),
            content=response["body"].encode(),
        )
        outcome["results"] = JSON.validate_python(
            config().transform_search_vector_store_response(reply, logging)
        )
    return outcome


def resolve(call: SearchCall | CreateRequestCall | CreateResponseCall) -> JsonValue:
    if call["function"] == "search":
        return search_outcome(call)
    if call["function"] == "create_request":
        optional = call.get("optional_params")
        params = CreateOptions() if optional is None else optional
        url, body = config().transform_create_vector_store_request(
            vector_store_create_optional_params=params,
            api_base=call["api_base"],
        )
        return {"url": url, "body": OBJECT.validate_python(body)}
    reply = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode()
    )
    return {"created": JSON.validate_python(config().transform_create_vector_store_response(reply))}


def run(input_value: Input) -> JsonValue:
    outcomes = [resolve(call) for call in input_value["calls"]]
    return {"calls": outcomes}
