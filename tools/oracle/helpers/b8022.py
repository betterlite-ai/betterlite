from typing import Protocol, TypeAlias, cast

import httpx
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.llms.fastcrw.search.transformation import FastCRWSearchConfig
from litellm.llms.firecrawl.search.transformation import FirecrawlSearchConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class SearchConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, optional_params: dict[str, Json]
    ) -> str: ...

    def transform_search_request(
        self, query: str | list[str], optional_params: dict[str, Json]
    ) -> dict[str, Json]: ...

    def transform_search_response(
        self, raw_response: httpx.Response, logging_obj: object
    ) -> SearchResponse: ...


def _config(provider: str) -> SearchConfig:
    if provider == "fastcrw":
        return cast(SearchConfig, FastCRWSearchConfig())
    if provider == "firecrawl":
        return cast(SearchConfig, FirecrawlSearchConfig())
    raise ValueError(f"unknown provider {provider}")


def _object(input_value: Object, name: str) -> Object:
    value = input_value.get(name)
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(Object, value)


def _optional_string(input_value: Object, name: str) -> str | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _query(input_value: Object) -> str | list[str]:
    value = input_value.get("query")
    if isinstance(value, str):
        return value
    if isinstance(value, list) and all(isinstance(item, str) for item in value):
        return cast("list[str]", value)
    raise ValueError("query must be a string or a list of strings")


def _request(input_value: Object, config: SearchConfig) -> Json:
    api_key = _optional_string(input_value, "api_key")
    api_base = _optional_string(input_value, "api_base")
    optional_params = _object(input_value, "optional_params")
    headers = config.validate_environment(
        headers={}, api_key=api_key, api_base=api_base
    )
    url = config.get_complete_url(api_base=api_base, optional_params=optional_params)
    body = config.transform_search_request(
        query=_query(input_value), optional_params=optional_params
    )
    return {"url": url, "headers": cast(Json, headers), "body": cast(Json, body)}


def _responses(input_value: Object, config: SearchConfig) -> Json:
    replies = input_value.get("replies", [])
    if not isinstance(replies, list):
        raise ValueError("replies must be a list")
    responses: list[Json] = []
    for reply in replies:
        request = httpx.Request("POST", "https://oracle.test/search")
        response = httpx.Response(httpx.codes.OK, json=reply, request=request)
        parsed = config.transform_search_response(response, None)
        responses.append(cast(Json, parsed.model_dump()))
    return {"responses": responses}


def run(input_value: Object) -> Json:
    config = _config(cast(str, input_value.get("provider")))
    function = cast(str, input_value.get("function"))
    if function == "transform_search_request":
        return _request(input_value, config)
    if function == "transform_search_response":
        return _responses(input_value, config)
    raise ValueError(f"unknown function {function}")
