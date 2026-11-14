import os
import time
from datetime import datetime
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.vector_stores.transformation import GeminiVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreSearchOptionalRequestParams,
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Headers: TypeAlias = dict[str, str]

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
JSON: Final = TypeAdapter(JsonValue)
MESSAGES: Final[list[str]] = []
START_TIME: Final = datetime.fromisoformat("2026-01-01T00:00:00+00:00")

LOGGING: Final = Logging(
    model="",
    messages=MESSAGES,
    stream=False,
    call_type="vector_store_search",
    start_time=START_TIME,
    litellm_call_id="",
    function_id="",
)

CONFIG: Final = GeminiVectorStoreConfig()


class Params(TypedDict):
    api_key: NotRequired[str]


class CapabilityCall(TypedDict):
    function: Literal["capabilities"]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    headers: NotRequired[Headers]
    params: NotRequired[Params]
    environment: NotRequired[Headers]


class UrlCall(TypedDict):
    function: Literal["url"]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]


class SearchCall(TypedDict):
    function: Literal["search"]
    vector_store_id: str
    query: str | list[str]
    filters: NotRequired[dict[str, JsonValue]]
    max_num_results: NotRequired[int]
    extra_body: NotRequired[dict[str, JsonValue]]
    api_key: NotRequired[str]
    model: NotRequired[str]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]


class ReplyCall(TypedDict):
    function: Literal["search_reply"]
    status: int
    headers: NotRequired[Headers]
    body: str
    query: str


class CreateCall(TypedDict):
    function: Literal["create"]
    name: NotRequired[str]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]


class CreateReplyCall(TypedDict):
    function: Literal["create_reply"]
    status: int
    headers: NotRequired[Headers]
    body: str
    timezone: str


Call: TypeAlias = (
    CapabilityCall
    | EnvironmentCall
    | UrlCall
    | SearchCall
    | ReplyCall
    | CreateCall
    | CreateReplyCall
)


class Input(TypedDict):
    calls: list[Call]


def environment(given: Headers | None) -> None:
    os.environ.clear()
    if given is not None:
        os.environ.update(given)


def raw_response(status: int, headers: Headers | None, body: str) -> httpx.Response:
    return httpx.Response(status, content=body.encode("utf-8"), headers=headers)


def capabilities() -> JsonValue:
    return {
        "endpoints": OBJECT.validate_python(CONFIG.get_vector_store_endpoints_by_type()),
        "credentials": OBJECT.validate_python(CONFIG.get_auth_credentials({})),
        "supported": JSON.validate_python(CONFIG.get_supported_openai_params("")),
    }


def environment_outcome(call: EnvironmentCall) -> JsonValue:
    environment(call.get("environment"))
    given = call.get("headers")
    headers: Headers = {} if given is None else dict(given)
    params = call.get("params")
    litellm_params = None if params is None else GenericLiteLLMParams(api_key=params.get("api_key"))
    return {
        "headers": OBJECT.validate_python(
            CONFIG.validate_environment(headers, litellm_params)
        )
    }


def url_outcome(call: UrlCall) -> JsonValue:
    environment(call.get("environment"))
    return {"url": CONFIG.get_complete_url(call.get("api_base"), {})}


def search_params(call: SearchCall) -> VectorStoreSearchOptionalRequestParams:
    params: VectorStoreSearchOptionalRequestParams = {}
    filters = call.get("filters")
    if filters is not None:
        params["filters"] = filters
    limit = call.get("max_num_results")
    if limit is not None:
        params["max_num_results"] = limit
    return params


def search_outcome(call: SearchCall) -> JsonValue:
    environment(call.get("environment"))
    litellm_params: dict[str, JsonValue] = {}
    api_key = call.get("api_key")
    if api_key is not None:
        litellm_params["api_key"] = api_key
    model = call.get("model")
    if model is not None:
        litellm_params["model"] = model
    api_base = CONFIG.get_complete_url(call.get("api_base"), {})
    url, body = CONFIG.transform_search_vector_store_request(
        call["vector_store_id"],
        call["query"],
        search_params(call),
        api_base,
        LOGGING,
        litellm_params,
        extra_body=call.get("extra_body"),
    )
    return {"url": url, "body": OBJECT.validate_python(body)}


def search_reply(call: ReplyCall) -> JsonValue:
    LOGGING.model_call_details["query"] = call["query"]
    response = raw_response(call["status"], call.get("headers"), call["body"])
    return OBJECT.validate_python(CONFIG.transform_search_vector_store_response(response, LOGGING))


def create_outcome(call: CreateCall) -> JsonValue:
    environment(call.get("environment"))
    api_base = CONFIG.get_complete_url(call.get("api_base"), {})
    request = VectorStoreCreateOptionalRequestParams(name=call.get("name"))
    url, body = CONFIG.transform_create_vector_store_request(request, api_base)
    return {"url": url, "body": OBJECT.validate_python(body)}


def create_reply(call: CreateReplyCall) -> JsonValue:
    os.environ["TZ"] = call["timezone"]
    time.tzset()
    response = raw_response(call["status"], call.get("headers"), call["body"])
    return OBJECT.validate_python(CONFIG.transform_create_vector_store_response(response))


def resolve(call: Call) -> JsonValue:
    if call["function"] == "capabilities":
        return capabilities()
    if call["function"] == "environment":
        return environment_outcome(call)
    if call["function"] == "url":
        return url_outcome(call)
    if call["function"] == "search":
        return search_outcome(call)
    if call["function"] == "search_reply":
        return search_reply(call)
    if call["function"] == "create":
        return create_outcome(call)
    return create_reply(call)


def run(input_value: Input) -> JsonValue:
    return {"calls": [resolve(call) for call in input_value["calls"]]}
