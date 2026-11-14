from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.vector_stores.rag_api.transformation import VertexVectorStoreConfig
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreSearchOptionalRequestParams,
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

CONFIG: Final = VertexVectorStoreConfig()
JSON: Final = TypeAdapter(JsonValue)


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
    optional_params: NotRequired[VectorStoreSearchOptionalRequestParams]
    response: NotRequired[Reply]


class CreateRequestCall(TypedDict):
    function: Literal["create_request"]
    api_base: str
    optional_params: NotRequired[VectorStoreCreateOptionalRequestParams]


class CreateResponseCall(TypedDict):
    function: Literal["create_response"]
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class Input(TypedDict):
    calls: list[SearchCall | CreateRequestCall | CreateResponseCall]


def reply_of(status: int, body: str, headers: dict[str, str] | None) -> httpx.Response:
    return httpx.Response(status, headers=headers, content=body.encode())


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
    url, body = CONFIG.transform_search_vector_store_request(
        vector_store_id=call["vector_store_id"],
        query=call["query"],
        vector_store_search_optional_params=VectorStoreSearchOptionalRequestParams()
        if optional is None
        else optional,
        api_base=call["api_base"],
        litellm_logging_obj=logging,
        litellm_params={} if litellm_params is None else litellm_params,
    )
    outcome: dict[str, JsonValue] = {"url": url, "body": JSON.validate_python(body)}
    response = call.get("response")
    if response is not None:
        reply = reply_of(response["status"], response["body"], response.get("headers"))
        outcome["results"] = JSON.validate_python(
            CONFIG.transform_search_vector_store_response(reply, logging)
        )
    return outcome


def resolve(call: SearchCall | CreateRequestCall | CreateResponseCall) -> JsonValue:
    if call["function"] == "search":
        return search_outcome(call)
    if call["function"] == "create_request":
        optional = call.get("optional_params")
        params = VectorStoreCreateOptionalRequestParams() if optional is None else optional
        url, body = CONFIG.transform_create_vector_store_request(
            vector_store_create_optional_params=params,
            api_base=call["api_base"],
        )
        return {"url": url, "body": JSON.validate_python(body)}
    reply = reply_of(call["status"], call["body"], call.get("headers"))
    return {"created": JSON.validate_python(CONFIG.transform_create_vector_store_response(reply))}


def run(input_value: Input) -> JsonValue:
    return {"calls": [resolve(call) for call in input_value["calls"]]}
