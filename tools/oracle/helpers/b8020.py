import os
from typing import Protocol, TypeAlias, cast

import httpx
from litellm.llms.exa_ai.search.transformation import ExaAISearchConfig
from litellm.llms.serper.search.transformation import SerperSearchConfig
from litellm.secret_managers.main import get_secret_str

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROVIDERS: dict[str, tuple[str, str, str]] = {
    "exa_ai": ("EXA_API_KEY", "EXA_API_BASE", "https://api.exa.ai"),
    "serper": ("SERPER_API_KEY", "SERPER_API_BASE", "https://google.serper.dev"),
}


class _SearchResponse(Protocol):
    def model_dump(self) -> dict[str, Json]: ...


class _SearchConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, str], api_key: str | None = None, api_base: str | None = None
    ) -> dict[str, str]: ...

    def get_complete_url(self, api_base: str | None, optional_params: dict[str, Json]) -> str: ...

    def transform_search_request(
        self, query: str | list[str], optional_params: dict[str, Json]
    ) -> Json: ...

    def transform_search_response(
        self, raw_response: httpx.Response, logging_obj: object
    ) -> _SearchResponse: ...


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(value: Json, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _string_map(value: Json, name: str) -> dict[str, str]:
    mapping = _object(value, name)
    result: dict[str, str] = {}
    for key, item in mapping.items():
        result[key] = _string(item, key)
    return result


def _query(value: Json) -> str | list[str]:
    if isinstance(value, str):
        return value
    if not isinstance(value, list):
        raise ValueError("query must be a string or a list of strings")
    texts: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ValueError("query must be a string or a list of strings")
        texts.append(item)
    return texts


def _config(provider: str) -> _SearchConfig:
    if provider == "exa_ai":
        return cast(_SearchConfig, ExaAISearchConfig())
    if provider == "serper":
        return cast(_SearchConfig, SerperSearchConfig())
    raise ValueError(f"unknown provider {provider}")


def _settings(provider: str) -> Json:
    api_key_var, api_base_var, default_base = _PROVIDERS[provider]
    return {
        "api_key": get_secret_str(api_key_var) or None,
        "api_base": get_secret_str(api_base_var) or default_base,
    }


def _call(provider: str, config: _SearchConfig, call: dict[str, Json]) -> Json:
    kind = _string(call.get("kind"), "kind")
    if kind == "settings":
        return _settings(provider)
    if kind == "validate_environment":
        headers = _string_map(call.get("headers", {}), "headers")
        api_key = _optional_string(call.get("api_key"), "api_key")
        api_base = _optional_string(call.get("api_base"), "api_base")
        return cast(Json, config.validate_environment(headers, api_key, api_base))
    if kind == "complete_url":
        api_base = _optional_string(call.get("api_base"), "api_base")
        return config.get_complete_url(api_base, {})
    if kind == "request":
        query = _query(call.get("query"))
        options = _object(call.get("optional_params", {}), "optional_params")
        return config.transform_search_request(query, options)
    if kind == "reply":
        body = _string(call.get("body"), "body")
        response = httpx.Response(status_code=200, content=body.encode())
        return config.transform_search_response(response, None).model_dump()
    raise ValueError(f"unknown call kind {kind}")


def run(input_value: dict[str, Json]) -> Json:
    environment = _string_map(input_value.get("environment", {}), "environment")
    os.environ.update(environment)
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    outcomes: list[Json] = []
    for call in calls:
        entry = _object(call, "call")
        provider = _string(entry.get("provider"), "provider")
        try:
            config = _config(provider)
            value = _call(provider, config, _object(entry.get("call"), "call"))
        except Exception:  # noqa: BLE001 - a provider failure is the outcome the case compares
            outcomes.append({"ok": False})
        else:
            outcomes.append({"ok": True, "value": value})
    return outcomes
