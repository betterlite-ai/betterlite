import os
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.llms.duckduckgo.search.transformation import DuckDuckGoSearchConfig
from pydantic import JsonValue, TypeAdapter

# a provider banner prints on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ResultView(TypedDict):
    title: str
    url: str
    snippet: str
    date: str | None
    last_updated: str | None


class SearchReply(TypedDict):
    results: list[ResultView]
    object: str


class UrlView(TypedDict):
    address: str
    params: list[str]


class HeadersCall(TypedDict):
    kind: Literal["headers"]
    headers: dict[str, str]


class RequestCall(TypedDict):
    kind: Literal["request"]
    query: str | list[str]
    optional_params: dict[str, JsonValue]


class UrlCall(TypedDict):
    kind: Literal["url"]
    api_base: str | None
    query: str | list[str]
    optional_params: dict[str, JsonValue]


class ReplyCall(TypedDict):
    kind: Literal["reply"]
    query: str | list[str]
    optional_params: dict[str, JsonValue]
    body: str


Call: TypeAlias = HeadersCall | RequestCall | UrlCall | ReplyCall


class Input(TypedDict):
    environment: dict[str, str]
    calls: list[Call]


class Config(Protocol):
    def validate_environment(
        self, headers: dict[str, str]
    ) -> dict[str, str]: ...

    def transform_search_request(
        self, query: str | list[str], optional_params: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        optional_params: dict[str, JsonValue],
        data: dict[str, JsonValue],
    ) -> str: ...

    def transform_search_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> SearchResponse: ...


# the source leaves these methods bare, so this typed view is its public request and reply entry
CONFIG: Config = DuckDuckGoSearchConfig()


def logging_object() -> Logging:
    return Logging(
        model="duckduckgo/search",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b1555",
        function_id="b1555",
    )


def result_value(result: ResultView) -> JsonValue:
    view: ResultView = {
        "title": result["title"],
        "url": result["url"],
        "snippet": result["snippet"],
        "date": result["date"],
        "last_updated": result["last_updated"],
    }
    return JSON.validate_python(view)


def split_url(url: str) -> JsonValue:
    address, separator, query = url.partition("?")
    parts = query.split("&") if separator else []
    ordered = sorted(parts)
    view: UrlView = {"address": address, "params": ordered}
    return JSON.validate_python(view)


def headers_value(call: HeadersCall) -> JsonValue:
    headers = dict(call["headers"])
    return JSON.validate_python(CONFIG.validate_environment(headers))


def request_value(call: RequestCall) -> JsonValue:
    return JSON.validate_python(
        CONFIG.transform_search_request(call["query"], call["optional_params"])
    )


def url_value(call: UrlCall) -> JsonValue:
    body = CONFIG.transform_search_request(call["query"], call["optional_params"])
    url = CONFIG.get_complete_url(call["api_base"], call["optional_params"], data=body)
    return split_url(url)


def reply_value(call: ReplyCall) -> JsonValue:
    body = CONFIG.transform_search_request(call["query"], call["optional_params"])
    url = CONFIG.get_complete_url(None, call["optional_params"], data=body)
    request = httpx.Request("GET", url)
    response = httpx.Response(200, content=call["body"].encode(), request=request)
    transformed = CONFIG.transform_search_response(
        raw_response=response, logging_obj=logging_object()
    )
    view = TypeAdapter(SearchReply).validate_json(transformed.model_dump_json())
    reply: dict[str, JsonValue] = {
        "results": [result_value(item) for item in view["results"]],
        "object": view["object"],
    }
    return JSON.validate_python(reply)


def answer(call: Call) -> JsonValue:
    if call["kind"] == "headers":
        return headers_value(call)
    if call["kind"] == "request":
        return request_value(call)
    if call["kind"] == "url":
        return url_value(call)
    return reply_value(call)


def outcome(call: Call) -> JsonValue:
    try:
        value: dict[str, JsonValue] = {"ok": True, "value": answer(call)}
        return JSON.validate_python(value)
    # the source raises these for a reply that is not a JSON object or carries a wrong typed field
    except (AttributeError, TypeError, ValueError):
        failure: dict[str, JsonValue] = {"ok": False}
        return JSON.validate_python(failure)


def run(input_value: Input) -> JsonValue:
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(input_value["environment"])
    try:
        calls = [outcome(call) for call in input_value["calls"]]
        value: dict[str, JsonValue] = {"calls": calls}
        return JSON.validate_python(value)
    finally:
        os.environ.clear()
        os.environ.update(previous)
