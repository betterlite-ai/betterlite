import os
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.brave.search.transformation import BraveSearchConfig, to_yyyy_mm_dd
from litellm.llms.base_llm.search.transformation import SearchResponse
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


class RequestCall(TypedDict):
    kind: Literal["request"]
    headers: dict[str, str]
    api_key: str | None
    api_base: str | None
    query: str | list[str]
    optional_params: dict[str, JsonValue]


class ReplyCall(TypedDict):
    kind: Literal["reply"]
    query: str | list[str]
    optional_params: dict[str, JsonValue]
    body: str


class DateCall(TypedDict):
    kind: Literal["date"]
    value: str | float | None
    dayfirst: bool
    yearfirst: bool


Call: TypeAlias = RequestCall | ReplyCall | DateCall


class Input(TypedDict):
    environment: dict[str, str]
    calls: list[Call]


CONFIG: Final = BraveSearchConfig()


def result_value(result: ResultView) -> JsonValue:
    view: ResultView = {
        "title": result["title"],
        "url": result["url"],
        "snippet": result["snippet"],
        "date": result["date"],
        "last_updated": result["last_updated"],
    }
    return JSON.validate_python(view)


def request_value(call: RequestCall) -> JsonValue:
    headers = CONFIG.validate_environment(
        dict(call["headers"]), api_key=call["api_key"], api_base=call["api_base"]
    )
    params = CONFIG.transform_search_request(call["query"], call["optional_params"])
    url = CONFIG.get_complete_url(call["api_base"], call["optional_params"], data=params)
    view: dict[str, JsonValue] = {
        "headers": JSON.validate_python(headers),
        "params": JSON.validate_python(params),
        "url": url,
    }
    return JSON.validate_python(view)


def reply_value(call: ReplyCall) -> JsonValue:
    params = CONFIG.transform_search_request(call["query"], call["optional_params"])
    url = CONFIG.get_complete_url(None, call["optional_params"], data=params)
    request = httpx.Request("GET", url)
    response = httpx.Response(200, content=call["body"].encode(), request=request)
    transformed: SearchResponse = CONFIG.transform_search_response(
        raw_response=response, logging_obj=None
    )
    view = TypeAdapter(SearchReply).validate_json(transformed.model_dump_json())
    reply: dict[str, JsonValue] = {
        "results": [result_value(item) for item in view["results"]],
        "object": view["object"],
    }
    return JSON.validate_python(reply)


def answer(call: Call) -> JsonValue:
    if call["kind"] == "request":
        return request_value(call)
    if call["kind"] == "reply":
        return reply_value(call)
    return JSON.validate_python(
        to_yyyy_mm_dd(call["value"], dayfirst=call["dayfirst"], yearfirst=call["yearfirst"])
    )


def outcome(call: Call) -> JsonValue:
    try:
        return JSON.validate_python({"ok": True, "value": answer(call)})
    # the source raises these for a missing key, an untrusted base or a reply it cannot read
    except (AttributeError, TypeError, ValueError):
        return JSON.validate_python({"ok": False})


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
