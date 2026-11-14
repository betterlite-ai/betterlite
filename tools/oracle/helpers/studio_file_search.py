import os
import time
from collections.abc import Mapping, Sequence
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.vector_stores.transformation import GeminiVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreCreateResponse,
    VectorStoreIndexEndpoints,
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Headers: TypeAlias = dict[str, str]
JSON: Final = TypeAdapter(JsonValue)
ANY: Final = TypeAdapter(object)


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


class Input(TypedDict):
    calls: list[Call]


class Credentials(TypedDict, total=False):
    headers: dict[str, str]
    query_params: dict[str, str]


class SearchOptions(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


class Content(TypedDict, total=False):
    text: str | None
    type: str | None


class Result(TypedDict, total=False):
    score: float | None
    content: list[Content] | None
    file_id: str | None
    filename: str | None
    attributes: dict[str, JsonValue] | None


class Reply(TypedDict, total=False):
    object: Literal["vector_store.search_results.page"]
    search_query: str | None
    data: list[Result] | None


class LoggingRecord(Logging):
    model_call_details: dict[str, JsonValue]

    def __init__(self) -> None:
        self.model_call_details = {}


LOGGING: Final = LoggingRecord()


class VectorStoreConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def get_vector_store_endpoints_by_type(self) -> VectorStoreIndexEndpoints: ...

    def get_auth_credentials(self, litellm_params: dict[str, JsonValue]) -> Credentials: ...

    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...

    def validate_environment(
        self, headers: Headers, litellm_params: GenericLiteLLMParams | None
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/vector_stores/transformation.py#L162-L171
        self,
        vector_store_id: str,
        query: str | list[str],
        # ast-grep-ignore: no-long-names
        vector_store_search_optional_params: SearchOptions,
        api_base: str,
        litellm_logging_obj: Logging,
        litellm_params: dict[str, JsonValue],
        extra_body: Mapping[str, JsonValue] | None = None,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_response(
        self, response: httpx.Response, litellm_logging_obj: Logging
    ) -> Reply: ...

    # ast-grep-ignore: no-long-names
    def transform_create_vector_store_request(
        self,
        # ast-grep-ignore: no-long-names
        vector_store_create_optional_params: VectorStoreCreateOptionalRequestParams,
        api_base: str,
    ) -> tuple[str, dict[str, JsonValue]]: ...

    # ast-grep-ignore: no-long-names
    def transform_create_vector_store_response(
        self, response: httpx.Response
    ) -> VectorStoreCreateResponse: ...


CONFIG: VectorStoreConfig = GeminiVectorStoreConfig()


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
    validated = CONFIG.validate_environment(headers, litellm_params)
    return {"headers": JSON.validate_python(validated)}


def url_outcome(call: Call) -> JsonValue:
    environment(call.get("environment"))
    return {"url": CONFIG.get_complete_url(call.get("api_base"), {})}


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
        {"filters": call.get("filters"), "max_num_results": call.get("max_num_results")},
        api_base,
        LOGGING,
        litellm_params,
        extra_body=call.get("extra_body"),
    )
    return {"url": url, "body": JSON.validate_python(body)}


def search_reply(call: Call) -> JsonValue:
    query = call.get("query")
    LOGGING.model_call_details["query"] = JSON.validate_python("" if query is None else query)
    record = CONFIG.transform_search_vector_store_response(raw_response(call), LOGGING)
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
    if function in {"create", "create_reply"}:
        return create_outcome(call) if function == "create" else create_reply(call)
    return capabilities()


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [resolve(call) for call in input_value["calls"]]
    return {"calls": answers}
