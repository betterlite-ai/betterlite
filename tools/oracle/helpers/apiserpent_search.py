import os
from collections.abc import Callable
from typing import Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.apiserpent.search.transformation import APISerpentSearchConfig
from litellm.llms.base_llm.search.transformation import SearchResponse
from pydantic import JsonValue, TypeAdapter

REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

ValidateEnvironment = Callable[[dict[str, str], str | None, str | None], dict[str, str]]
CompleteUrl = Callable[[str | None, dict[str, JsonValue], dict[str, JsonValue]], str]
TransformRequest = Callable[[str | list[str], dict[str, JsonValue]], dict[str, JsonValue]]
TransformResponse = Callable[[httpx.Response, Logging | None], SearchResponse]


class ApiserpentConfig(Protocol):
    validate_environment: ValidateEnvironment
    get_complete_url: CompleteUrl
    transform_search_request: TransformRequest
    transform_search_response: TransformResponse


class Input(TypedDict):
    query: str | list[str]
    optional_params: dict[str, JsonValue]
    api_key: str | None
    api_base: str | None
    environment: dict[str, str]
    headers: dict[str, str]
    reply: str | None


def environment(values: dict[str, str]) -> None:
    os.environ.pop("APISERPENT_API_KEY", None)
    os.environ.pop("APISERPENT_API_BASE", None)
    for name, value in values.items():
        os.environ[name] = value


def reply_value(text: str, view: ApiserpentConfig) -> JsonValue:
    response = httpx.Response(
        200,
        content=text.encode(),
        request=httpx.Request("GET", "https://apiserpent.com/api/search"),
    )
    return REPLY.validate_json(view.transform_search_response(response, None).model_dump_json())


def run(input_value: Input) -> JsonValue:
    environment(input_value["environment"])
    view: ApiserpentConfig = APISerpentSearchConfig()
    headers = view.validate_environment(
        input_value["headers"],
        input_value["api_key"],
        input_value["api_base"],
    )
    params = view.transform_search_request(input_value["query"], input_value["optional_params"])
    url = view.get_complete_url(input_value["api_base"], input_value["optional_params"], params)
    text = input_value["reply"]
    response = None if text is None else reply_value(text, view)
    decoded_headers = REPLY.validate_python(headers)
    decoded_params = REPLY.validate_python(params)
    return {
        "headers": decoded_headers,
        "params": decoded_params,
        "url": url,
        "response": response,
    }
