from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.vector_stores.rag_api.transformation import VertexVectorStoreConfig
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]

CONFIG: Final = VertexVectorStoreConfig()
JSON: Final = TypeAdapter(JsonValue)


# the optional params the RAG transform reads, typed so no case input reaches it unvalidated
class SearchParams(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


class CreateParams(TypedDict, total=False):
    name: str | None
    metadata: dict[str, str] | None


SEARCH_PARAMS: Final = TypeAdapter(SearchParams)
CREATE_PARAMS: Final = TypeAdapter(CreateParams)


class ResponseCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[Headers]


class SearchCall(TypedDict):
    function: Literal["search"]
    vector_store_id: str
    query: str | list[str]
    api_base: str
    litellm_params: NotRequired[Object]
    optional_params: NotRequired[Object]
    response: NotRequired[ResponseCall]


class CreateRequestCall(TypedDict):
    function: Literal["create_request"]
    api_base: str
    optional_params: NotRequired[Object]


class CreateResponseCall(TypedDict):
    function: Literal["create_response"]
    status: int
    body: str
    headers: NotRequired[Headers]


Call: TypeAlias = SearchCall | CreateRequestCall | CreateResponseCall


class Input(TypedDict):
    calls: list[Call]


INPUT: Final = TypeAdapter(Input)


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


def search_results(logging: Logging, response: ResponseCall) -> JsonValue:
    reply = httpx.Response(
        response["status"],
        headers=response.get("headers"),
        content=response["body"].encode(),
    )
    return JSON.validate_python(CONFIG.transform_search_vector_store_response(reply, logging))  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vector_stores/rag_api/transformation.py#L218


def search_outcome(call: SearchCall) -> JsonValue:
    raw_params = call.get("optional_params")
    search_params = SEARCH_PARAMS.validate_python(SearchParams() if raw_params is None else raw_params)
    raw_litellm_params = call.get("litellm_params")
    litellm_params = Object() if raw_litellm_params is None else dict(raw_litellm_params)
    logging = logging_object()
    url, body = CONFIG.transform_search_vector_store_request(
        vector_store_id=call["vector_store_id"],
        query=call["query"],
        vector_store_search_optional_params=search_params,
        api_base=call["api_base"],
        litellm_logging_obj=logging,
        litellm_params=litellm_params,
    )
    outcome: dict[str, JsonValue] = {"url": url, "body": JSON.validate_python(body)}
    response = call.get("response")
    if response is not None:
        outcome["results"] = search_results(logging, response)
    return outcome


def create_request_outcome(call: CreateRequestCall) -> JsonValue:
    raw_params = call.get("optional_params")
    create_params = CREATE_PARAMS.validate_python(CreateParams() if raw_params is None else raw_params)
    url, body = CONFIG.transform_create_vector_store_request(
        vector_store_create_optional_params=create_params,
        api_base=call["api_base"],
    )
    return {"url": url, "body": JSON.validate_python(body)}


def create_response_outcome(call: CreateResponseCall) -> JsonValue:
    reply = httpx.Response(
        call["status"],
        headers=call.get("headers"),
        content=call["body"].encode(),
    )
    return {"created": JSON.validate_python(CONFIG.transform_create_vector_store_response(reply))}


def resolve(call: Call) -> JsonValue:
    if call["function"] == "search":
        return search_outcome(call)
    if call["function"] == "create_request":
        return create_request_outcome(call)
    return create_response_outcome(call)


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
