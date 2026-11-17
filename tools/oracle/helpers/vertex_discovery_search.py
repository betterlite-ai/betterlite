from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import LlmProviders
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
    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vector_stores/search_api/transformation.py#L394-L430
        self,
        vector_store_id: str,
        query: str | list[str],
        # ast-grep-ignore: no-long-names
        vector_store_search_optional_params: SearchOptions,
        api_base: str,
        litellm_logging_obj: Logging,
        litellm_params: dict[str, JsonValue],
        extra_body: dict[str, JsonValue] | None = None,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_response(
        self, response: httpx.Response, litellm_logging_obj: Logging
    ) -> SearchReply: ...

    def calculate_vector_store_cost(
        self, response: SearchReply
    ) -> tuple[float, float]: ...


class Reply(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class TargetCall(TypedDict):
    function: Literal["target"]
    api_base: NotRequired[str | None]
    litellm_params: NotRequired[dict[str, JsonValue]]


class SearchCall(TypedDict):
    function: Literal["search"]
    vector_store_id: str
    query: str | list[str]
    api_base: str
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[SearchOptions]
    extra_body: NotRequired[dict[str, JsonValue] | None]
    response: NotRequired[Reply]


class CostCall(TypedDict):
    function: Literal["cost"]


class Input(TypedDict):
    calls: list[TargetCall | SearchCall | CostCall]


def config() -> VertexConfig:
    found = ProviderConfigManager.get_provider_vector_stores_config(
        provider=LlmProviders.VERTEX_AI, api_type="search_api"
    )
    if found is None:
        raise ValueError("the Vertex AI Search vector store config is not registered")
    return cast(VertexConfig, found)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/vector_store/transformation.py#L147-L186


def target_outcome(call: TargetCall) -> JsonValue:
    litellm_params = call.get("litellm_params")
    url = config().get_complete_url(
        call.get("api_base"), {} if litellm_params is None else litellm_params
    )
    return {"url": url}


def search_outcome(call: SearchCall) -> JsonValue:
    logging = Logging(
        model="vertex_discovery_search",
        messages=None,
        stream=False,
        call_type="vector_store_search",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    optional = call.get("optional_params")
    litellm_params = call.get("litellm_params")
    url, body = config().transform_search_vector_store_request(
        vector_store_id=call["vector_store_id"],
        query=call["query"],
        vector_store_search_optional_params=SearchOptions() if optional is None else optional,
        api_base=call["api_base"],
        litellm_logging_obj=logging,
        litellm_params={} if litellm_params is None else litellm_params,
        extra_body=call.get("extra_body"),
    )
    outcome: dict[str, JsonValue] = {"url": url, "body": OBJECT.validate_python(body)}
    response = call.get("response")
    if response is None:
        return outcome
    reply = httpx.Response(
        response["status"],
        headers=response.get("headers"),
        content=response["body"].encode(),
    )
    decoded = config().transform_search_vector_store_response(reply, logging)
    outcome["results"] = results_value(decoded)
    return outcome


def result_value(hit: SearchHit) -> dict[str, JsonValue]:
    score = hit.get("score")
    return {
        "score": None if score is None else f"{score:.6f}",
        "content": JSON.validate_python(hit.get("content")),
        "file_id": hit.get("file_id"),
        "filename": hit.get("filename"),
        "attributes": OBJECT.validate_python(hit.get("attributes")),
    }


def results_value(reply: SearchReply) -> JsonValue:
    data = reply.get("data")
    return {
        "object": reply.get("object"),
        "search_query": reply.get("search_query"),
        "data": [] if data is None else [result_value(hit) for hit in data],
    }


def cost_outcome() -> JsonValue:
    input_cost, output_cost = config().calculate_vector_store_cost(SearchReply())
    return {"input": f"{input_cost:.6f}", "output": f"{output_cost:.6f}"}


def resolve(call: TargetCall | SearchCall | CostCall) -> JsonValue:
    if call["function"] == "target":
        return target_outcome(call)
    if call["function"] == "search":
        return search_outcome(call)
    return cost_outcome()


def run(input_value: Input) -> JsonValue:
    outcomes = [resolve(call) for call in input_value["calls"]]
    return {"calls": outcomes}
