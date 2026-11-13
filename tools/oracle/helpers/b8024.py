from typing import Final, TypeAlias, cast

from litellm.llms.google_pse.search.transformation import GooglePSESearchConfig
from litellm.llms.searchapi.search.transformation import SearchAPIConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

PROVIDERS: Final[frozenset[str]] = frozenset({"google_pse", "searchapi"})


def string_or_null(input_value: dict[str, object], name: str) -> str | None:
    value = input_value.get(name)
    if value is not None and not isinstance(value, str):
        raise ValueError(f"{name} must be a string or null")
    return value


def headers_value(input_value: dict[str, object]) -> dict[str, str]:
    raw = input_value.get("headers", {})
    if not isinstance(raw, dict):
        raise ValueError("headers must be an object")
    headers: dict[str, str] = {}
    for name, header in cast("dict[str, object]", raw).items():
        if not isinstance(header, str):
            raise ValueError("header values must be strings")
        headers[name] = header
    return headers


def optional_params(input_value: dict[str, object]) -> dict[str, object]:
    value = input_value.get("optional_params", {})
    if not isinstance(value, dict):
        raise ValueError("optional_params must be an object")
    return cast("dict[str, object]", value)


def query_value(input_value: dict[str, object]) -> str | list[str]:
    value = input_value["query"]
    if isinstance(value, str):
        return value
    if not isinstance(value, list):
        raise ValueError("query must be a string or a list of strings")
    items = cast("list[object]", value)
    if not all(isinstance(item, str) for item in items):
        raise ValueError("query must be a string or a list of strings")
    return cast("list[str]", items)


def run(input_value: dict[str, object]) -> dict[str, object]:
    provider = input_value["provider"]
    if not isinstance(provider, str) or provider not in PROVIDERS:
        raise ValueError(f"unknown provider {provider}")
    query = query_value(input_value)
    params = optional_params(input_value)
    api_key = string_or_null(input_value, "api_key")
    api_base = string_or_null(input_value, "api_base")
    search_engine_id = string_or_null(input_value, "search_engine_id")
    headers = headers_value(input_value)
    config: GooglePSESearchConfig | SearchAPIConfig = (
        GooglePSESearchConfig() if provider == "google_pse" else SearchAPIConfig()
    )
    validated = cast(
        "dict[str, object]",
        config.validate_environment(  # pyright: ignore[reportUnknownMemberType]
            headers=headers,
            api_key=api_key,
            api_base=api_base,
            search_engine_id=search_engine_id,
        ),
    )
    data = cast(
        "dict[str, object]",
        config.transform_search_request(  # pyright: ignore[reportUnknownMemberType]
            query=query,
            optional_params=params,
            api_key=api_key,
            api_base=api_base,
            search_engine_id=search_engine_id,
        ),
    )
    url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
        api_base=api_base,
        optional_params=params,
        data=data,
        api_key=api_key,
    )
    return {"headers": validated, "params": data, "url": url}
