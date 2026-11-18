import os
from typing import Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.apiserpent.search.transformation import APISerpentSearchConfig
from litellm.llms.base_llm.search.transformation import SearchResponse
from pydantic import JsonValue, TypeAdapter

REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ApiserpentConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        api_key: str | None,
        api_base: str | None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        optional_params: dict[str, JsonValue],
        data: dict[str, JsonValue],
    ) -> str: ...

    def transform_search_request(
        self,
        query: str | list[str],
        optional_params: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...

    def transform_search_response(
        self,
        raw_response: httpx.Response,
        logging_obj: Logging | None,
    ) -> SearchResponse: ...


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


def request_values(
    view: ApiserpentConfig,
    input_value: Input,
) -> tuple[dict[str, str], dict[str, JsonValue], str]:
    headers = view.validate_environment(
        input_value["headers"],
        input_value["api_key"],
        input_value["api_base"],
    )
    params = view.transform_search_request(input_value["query"], input_value["optional_params"])
    url = view.get_complete_url(input_value["api_base"], input_value["optional_params"], params)
    return headers, params, url


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
    headers, params, url = request_values(view, input_value)
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
