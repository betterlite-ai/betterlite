import json
from collections.abc import Callable
from importlib.resources import files
from typing import TypeAlias, TypedDict, cast

from litellm.proxy.public_endpoints.public_endpoints import (
    _build_endpoints,  # pyright: ignore[reportPrivateUsage]
    _load_endpoints,  # pyright: ignore[reportPrivateUsage]
)
from litellm.utils import get_max_tokens, get_model_info

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

CAPABILITY_FLAGS = (
    "supports_vision",
    "supports_function_calling",
    "supports_tool_choice",
    "supports_response_schema",
    "supports_prompt_caching",
    "supports_reasoning",
    "supports_web_search",
    "supports_pdf_input",
)


class EndpointProvider(TypedDict):
    slug: str
    display_name: str


class EndpointEntry(TypedDict):
    key: str
    label: str
    endpoint: str
    providers: list[EndpointProvider]


Projection = Callable[[dict[str, Json]], list[EndpointEntry]]
Loader = Callable[[], list[EndpointEntry]]

build_endpoints: Projection = cast(Projection, _build_endpoints)
load_endpoints: Loader = cast(Loader, _load_endpoints)


def run(input_value: dict[str, Json]) -> Json:
    operation = _text(_field(input_value, "operation"), "operation")
    if operation == "root_registry":
        source = _object(_field(input_value, "source"), "source")
        return _registry_projection(source, build_endpoints(source))
    if operation == "packaged_registry":
        return _packaged_registry()
    if operation == "model_info":
        return _model_info(_objects(input_value.get("queries"), "queries"))
    if operation == "max_tokens":
        return _max_tokens(_texts(input_value.get("models"), "models"))
    if operation == "provider_endpoint":
        return _routes(_objects(input_value.get("routes"), "routes"))
    raise ValueError(f"unknown operation {operation}")


def _field(value: dict[str, Json], name: str) -> Json:
    if name not in value:
        raise ValueError(f"{name} is required")
    return value[name]


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _objects(value: Json, name: str) -> list[dict[str, Json]]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [_object(item, name) for item in value]


def _texts(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [_text(item, name) for item in value]


def _backup_registry() -> dict[str, Json]:
    resource = files("litellm").joinpath("provider_endpoints_support_backup.json")
    return cast(dict[str, Json], json.loads(resource.read_text(encoding="utf-8")))


def _registry_projection(source: dict[str, Json], entries: list[EndpointEntry]) -> Json:
    providers = source.get("providers")
    count = len(providers) if isinstance(providers, dict) else 0
    return {"provider_count": count, "endpoints": [_endpoint(entry) for entry in entries]}


def _packaged_registry() -> Json:
    return _registry_projection(_backup_registry(), load_endpoints())


def _endpoint(entry: EndpointEntry) -> Json:
    providers: list[Json] = [
        {"slug": provider["slug"], "display_name": provider["display_name"]}
        for provider in entry["providers"]
    ]
    return {
        "key": entry["key"],
        "label": entry["label"],
        "endpoint": entry["endpoint"],
        "providers": providers,
    }


def _model_info(queries: list[dict[str, Json]]) -> Json:
    results: list[Json] = []
    for query in queries:
        model = _text(_field(query, "model"), "model")
        provider_value = query.get("provider")
        provider = None if provider_value is None else _text(provider_value, "provider")
        info = cast(dict[str, Json], get_model_info(model=model, custom_llm_provider=provider))
        result: dict[str, Json] = {
            "key": info.get("key"),
            "provider": info.get("litellm_provider"),
            "mode": info.get("mode"),
            "max_tokens": info.get("max_tokens"),
            "max_input_tokens": info.get("max_input_tokens"),
            "max_output_tokens": info.get("max_output_tokens"),
        }
        for flag in CAPABILITY_FLAGS:
            result[flag] = info.get(flag)
        results.append(result)
    return {"results": results}


def _max_tokens(models: list[str]) -> Json:
    results: list[Json] = [
        {"model": model, "max_tokens": get_max_tokens(model)} for model in models
    ]
    return {"results": results}


def _routes(routes: list[dict[str, Json]]) -> Json:
    providers = _backup_registry().get("providers")
    entries = load_endpoints()
    results: list[Json] = []
    for route in routes:
        provider = _text(_field(route, "provider"), "provider")
        endpoint = _text(_field(route, "endpoint"), "endpoint")
        supporting: list[Json] = [
            provider_entry["slug"]
            for entry in entries
            if entry["key"] == endpoint
            for provider_entry in entry["providers"]
        ]
        results.append(
            {
                "provider": provider,
                "endpoint": endpoint,
                "support": _route_support(providers, provider, endpoint),
                "providers": supporting,
            }
        )
    return {"results": results}


def _route_support(providers: Json, provider: str, endpoint: str) -> Json:
    if not isinstance(providers, dict):
        return None
    provider_entry = providers.get(provider)
    if not isinstance(provider_entry, dict):
        return None
    endpoints = provider_entry.get("endpoints")
    if not isinstance(endpoints, dict):
        return None
    return endpoints.get(endpoint)
