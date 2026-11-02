import asyncio
from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.proxy.public_endpoints import public_endpoints as pe

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def get_supported_providers(input_value: dict[str, Json]) -> Json:
    del input_value
    return cast(Json, asyncio.run(pe.get_supported_providers()))


def get_provider_fields(input_value: dict[str, Json]) -> Json:
    del input_value
    fields = asyncio.run(pe.get_provider_fields())
    return [field.model_dump() for field in fields]


def get_supported_endpoints(input_value: dict[str, Json]) -> Json:
    del input_value
    response = asyncio.run(pe.get_supported_endpoints())
    return response.model_dump()


def get_supported_endpoints_twice(input_value: dict[str, Json]) -> Json:
    del input_value
    first = asyncio.run(pe.get_supported_endpoints())
    second = asyncio.run(pe.get_supported_endpoints())
    return [first.model_dump(), second.model_dump()]


def get_endpoint_keys_in_order(input_value: dict[str, Json]) -> Json:
    del input_value
    response = asyncio.run(pe.get_supported_endpoints())
    return [endpoint.key for endpoint in response.endpoints]


def build_endpoints(input_value: dict[str, Json]) -> Json:
    raw = input_value["raw"]
    if not isinstance(raw, dict):
        raise ValueError("raw must be an object")
    parsed = pe._PROVIDERS_FILE_ADAPTER.validate_python(raw)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    result = pe._build_endpoints(parsed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(Json, result)


def clean_display_name(input_value: dict[str, Json]) -> Json:
    raw = input_value["raw"]
    if not isinstance(raw, str):
        raise ValueError("raw must be a string")
    return pe._clean_display_name(raw)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def provider_catalog_raw(input_value: dict[str, Json]) -> Json:
    del input_value
    fields = asyncio.run(pe.get_provider_fields())
    return [field.model_dump() for field in fields]


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "get_supported_providers": get_supported_providers,
    "get_provider_fields": get_provider_fields,
    "get_supported_endpoints": get_supported_endpoints,
    "get_supported_endpoints_twice": get_supported_endpoints_twice,
    "get_endpoint_keys_in_order": get_endpoint_keys_in_order,
    "build_endpoints": build_endpoints,
    "clean_display_name": clean_display_name,
    "provider_catalog_raw": provider_catalog_raw,
}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function not in HANDLERS:
        raise ValueError(f"unknown b3190 function: {function}")
    return HANDLERS[function](input_value)
