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
    VectorStoreSearchOptionalRequestParams as SearchParams,
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Headers: TypeAlias = dict[str, str]
JSON: Final = TypeAdapter(JsonValue)
ANY: Final = TypeAdapter(object)
START_TIME: Final = datetime.fromisoformat("2026-01-01T00:00:00+00:00")
LOGGING: Final = Logging(
    model="", messages=[], stream=False, call_type="vector_store_search",
    start_time=START_TIME, litellm_call_id="", function_id="",
)
CONFIG: Final = GeminiVectorStoreConfig()


class Call(TypedDict):
    function: Literal[
        "capabilities", "environment", "url", "search", "search_reply", "create", "create_reply"
    ]
    headers: NotRequired[Headers]
    params: NotRequired[dict[str, str]]
    environment: NotRequired[Headers]
    vector_store_id: NotRequired[str]
    query: NotRequired[str | list[str]]
    filters: NotRequired[dict[str, JsonValue]]
    max_num_results: NotRequired[int]
    extra_body: NotRequired[dict[str, JsonValue]]
    api_key: NotRequired[str]
    model: NotRequired[str]
    api_base: NotRequired[str]
    name: NotRequired[str]
    status: NotRequired[int]
    body: NotRequired[str]
    timezone: NotRequired[str]


INPUT: Final = TypeAdapter(dict[str, list[Call]])


def environment(given: Headers | None) -> None:
    os.environ.clear()
    if given is not None:
        os.environ.update(given)


def raw_response(call: Call) -> httpx.Response:
    status = call.get("status")
    body = call.get("body")
    content = ("" if body is None else body).encode("utf-8")
    headers = call.get("headers")
    return httpx.Response(0 if status is None else status, content=content, headers=headers)


def capabilities() -> JsonValue:
    found = {
        "endpoints": CONFIG.get_vector_store_endpoints_by_type(),
        "credentials": CONFIG.get_auth_credentials({}),
        "supported": CONFIG.get_supported_openai_params(""),
    }
    return JSON.validate_json(ANY.dump_json(found))


def environment_outcome(call: Call) -> JsonValue:
    environment(call.get("environment"))
    given = call.get("headers")
    params = call.get("params")
    api_key = None if params is None else params.get("api_key")
    litellm_params = None if params is None else GenericLiteLLMParams(api_key=api_key)
    headers = {} if given is None else given
    return {"headers": JSON.validate_python(CONFIG.validate_environment(headers, litellm_params))}


def url_outcome(call: Call) -> JsonValue:
    environment(call.get("environment"))
    return {"url": CONFIG.get_complete_url(call.get("api_base"), {})}


def search_params(call: Call) -> SearchParams:
    filters = call.get("filters")
    limit = call.get("max_num_results")
    return {"filters": filters, "max_num_results": limit}


def search_outcome(call: Call) -> JsonValue:
    environment(call.get("environment"))
    litellm_params: dict[str, JsonValue] = {
        "api_key": call.get("api_key"),
        "model": call.get("model"),
    }
    query = call.get("query")
    store_id = call.get("vector_store_id")
    api_base = CONFIG.get_complete_url(call.get("api_base"), {})
    url, body = CONFIG.transform_search_vector_store_request(
        "" if store_id is None else store_id,
        "" if query is None else query,
        search_params(call),
        api_base,
        LOGGING,
        litellm_params,
        extra_body=call.get("extra_body"),
    )
    return {"url": url, "body": JSON.validate_python(body)}


def search_reply(call: Call) -> JsonValue:
    query = call.get("query")
    LOGGING.model_call_details["query"] = "" if query is None else query
    response = raw_response(call)
    record = CONFIG.transform_search_vector_store_response(response, LOGGING)
    return JSON.validate_python(record)


def create_outcome(call: Call) -> JsonValue:
    environment(call.get("environment"))
    params = VectorStoreCreateOptionalRequestParams(name=call.get("name"))
    api_base = CONFIG.get_complete_url(call.get("api_base"), {})
    url, body = CONFIG.transform_create_vector_store_request(params, api_base)
    return {"url": url, "body": JSON.validate_python(body)}


def create_reply(call: Call) -> JsonValue:
    timezone = call.get("timezone")
    os.environ["TZ"] = "" if timezone is None else timezone
    time.tzset()
    record = CONFIG.transform_create_vector_store_response(raw_response(call))
    return JSON.validate_python(record)


def resolve(call: Call) -> JsonValue:
    function = call["function"]
    if function == "environment":
        return environment_outcome(call)
    if function == "url":
        return url_outcome(call)
    if function == "search":
        return search_outcome(call)
    if function == "search_reply":
        return search_reply(call)
    if function == "create" or function == "create_reply":
        return create_outcome(call) if function == "create" else create_reply(call)
    return capabilities()


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
