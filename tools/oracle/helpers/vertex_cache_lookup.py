from typing import Literal, TypeAlias, cast

import litellm
from litellm.llms.vertex_ai.context_caching.vertex_ai_context_caching import (
    ContextCachingEndpoints,
)

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Provider: TypeAlias = Literal["vertex_ai", "vertex_ai_beta", "gemini"]


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_field(params: Params, name: str) -> list[Json]:
    value = params.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(params: Params, name: str) -> str:
    value = params.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(params: Params, name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def target_row(item: Json) -> Json:
    fields = object_of(item, "target")
    try:
        auth, url = ContextCachingEndpoints()._get_token_and_url_context_caching(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            gemini_api_key=optional_string(fields, "gemini_api_key"),
            custom_llm_provider=cast(Provider, string_of(fields, "provider")),
            api_base=optional_string(fields, "api_base"),
            vertex_project=optional_string(fields, "project"),
            vertex_location=optional_string(fields, "location"),
            vertex_auth_header=optional_string(fields, "auth"),
            model=optional_string(fields, "model"),
        )
    except ValueError:
        return {"error": True}
    return {"url": url, "auth": cast(Json, auth)}


def run(input_value: dict[str, Json]) -> Json:
    return {"targets": [target_row(item) for item in list_field(input_value, "targets")]}
