import litellm
from pydantic import JsonValue

from azure_responses import run as responses_run
from azure_search import run as search_run

from .contracts import (
    BOOLEAN,
    FIELDS,
    RESPONSES_IN,
    SEARCH_IN,
    STRING,
    checked_url,
    first_step,
)


litellm.suppress_debug_info = True


def projected_of(row: dict[str, JsonValue]) -> JsonValue:
    projected: dict[str, JsonValue] = {}
    for name in ("model", "input", "tools"):
        if name in row:
            projected[name] = row[name]
    return projected


def responses_url(step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "function": "base_and_api_version",
        "default_api_version": "2025-04-01-preview",
        "steps": [
            {
                "api_base": step["api_base"],
                "api_version": step["api_version"],
                "headers": {},
                "api_key": "key-1",
            }
        ],
    }
    answer = responses_run(RESPONSES_IN.validate_python(payload))
    return checked_url(first_step(answer, "steps"))


def responses_transform(step: dict[str, JsonValue]) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "function": "strip_model_prefix",
        "steps": [{"model": step["model"], "input": step["input"]}],
    }
    answer = responses_run(RESPONSES_IN.validate_python(payload))
    return projected_of(first_step(answer, "steps"))


def resource_url(step: dict[str, JsonValue], kind: str) -> JsonValue:
    endpoint = (
        "list"
        if kind == "responses_input_items"
        else STRING.validate_python(step["endpoint"])
    )
    payload: dict[str, JsonValue] = {
        "endpoint": endpoint,
        "api_base": step["api_base"],
        "response_id": step["response_id"],
    }
    if endpoint == "list":
        for name in ("after", "before", "include", "limit", "order"):
            if name in step:
                payload[name] = step[name]
    call: dict[str, JsonValue] = {
        "function": "resource_id_before_query",
        "steps": [payload],
    }
    answer = responses_run(RESPONSES_IN.validate_python(call))
    row = first_step(answer, "steps")
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no responses request")
    return {"url": row["url"], "data": row["data"]}


def search_url(step: dict[str, JsonValue]) -> JsonValue:
    settings: dict[str, JsonValue] = {
        "urls": [{"environment": {}, "api_base": step["api_base"]}],
        "requests": [],
    }
    payload: dict[str, JsonValue] = {"settings": settings}
    answer = search_run(SEARCH_IN.validate_python(payload))
    return checked_url(first_step(answer, "urls"))


def search_request(step: dict[str, JsonValue]) -> JsonValue:
    model = STRING.validate_python(step["model"])
    query = STRING.validate_python(step["query"])
    scenario: dict[str, JsonValue] = {
        "environment": {"BING_GROUNDING_MODEL": model},
        "query": query,
    }
    payload: dict[str, JsonValue] = {"request": {"scenarios": [scenario]}}
    answer = search_run(SEARCH_IN.validate_python(payload))
    row = first_step(answer, "requests")
    if not BOOLEAN.validate_python(row["ok"]):
        raise ValueError("the fixture built no search request")
    body = FIELDS.validate_python(row["body"])
    return {"model": body["model"], "input": body["input"], "tools": body["tools"]}


def one_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    if kind == "responses_url":
        return responses_url(step)
    if kind == "responses_transform":
        return {"projected": responses_transform(step)}
    if kind == "search_url":
        return search_url(step)
    if kind == "search_request":
        return search_request(step)
    return resource_url(step, kind)
