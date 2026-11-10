import contextlib
import io
from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.caching.caching import (
    Cache,
    CacheMode,
)
from litellm.caching.caching import (
    print_verbose as raw_caching_print_verbose,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.types.caching import LiteLLMCacheType, SemanticCacheScope

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

caching_print_verbose = cast(Callable[[str], None], raw_caching_print_verbose)


class _FileObject:
    def __init__(self, name: str | None) -> None:
        if name is not None:
            self.name = name


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_object(value: Json, name: str) -> dict[str, Json] | None:
    if value is None:
        return None
    return object_value(value, name)


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def bool_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _cache(data: dict[str, Json]) -> Cache:
    cache_type = optional_string(data.get("cache_type"), "cache_type")
    mode = optional_string(data.get("mode"), "mode")
    namespace = optional_string(data.get("namespace"), "namespace")
    semantic_cache_scope = optional_string(data.get("semantic_cache_scope"), "semantic_cache_scope")
    return Cache(
        type=None if cache_type is None else LiteLLMCacheType(cache_type),
        mode=None if mode is None else CacheMode(mode),
        namespace=namespace,
        semantic_cache_scope="key" if semantic_cache_scope is None else semantic_cache_scope,
        _backend=InMemoryCache(),
    )


def _file_object(value: Json) -> object:
    if value is None:
        return _FileObject(None)
    return _FileObject(string_value(value, "file"))


def _build_kwargs(
    data: dict[str, Json], litellm_params: dict[str, Json] | None
) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    for entry in list_value(data.get("params"), "params"):
        record = object_value(entry, "param")
        name = string_value(record.get("name"), "name")
        value = record.get("value")
        kwargs[name] = _file_object(value) if name == "file" else value
    metadata = optional_object(data.get("metadata"), "metadata")
    if metadata is not None:
        kwargs["metadata"] = metadata
    litellm_metadata = optional_object(data.get("litellm_metadata"), "litellm_metadata")
    if litellm_metadata is not None:
        kwargs["litellm_metadata"] = litellm_metadata
    if litellm_params is not None:
        kwargs["litellm_params"] = litellm_params
    cache_control = optional_object(data.get("cache_control"), "cache_control")
    if cache_control is not None:
        kwargs["cache"] = cache_control
    return kwargs


def _get_cache_key(data: dict[str, Json]) -> Json:
    cache = _cache(data)
    litellm.enable_caching_on_provider_specific_optional_params = (
        data.get("include_provider_params") is True
    )
    litellm_params = optional_object(data.get("litellm_params"), "litellm_params")
    kwargs = _build_kwargs(data, litellm_params)
    key = cache.get_cache_key(**kwargs)  # pyright: ignore[reportUnknownMemberType]
    preset: Json = None if litellm_params is None else litellm_params.get("preset_cache_key")
    return cast(Json, {"cache_key": key, "preset_cache_key": preset})


def _should_use_cache(data: dict[str, Json]) -> Json:
    cache = _cache(data)
    cache_control = optional_object(data.get("cache_control"), "cache_control")
    kwargs: dict[str, object] = {}
    if cache_control is not None:
        kwargs["cache"] = cache_control
    used = cache.should_use_cache(**kwargs)  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"use_cache": used})


def _print_verbose(data: dict[str, Json]) -> Json:
    message = string_value(data.get("message"), "message")
    litellm.set_verbose = bool_value(data.get("set_verbose"), "set_verbose")  # pyright: ignore[reportPrivateImportUsage]
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        caching_print_verbose(message)
    return cast(Json, {"console": output.getvalue().splitlines()})


def _cache_mode_values() -> Json:
    modes = [mode.value for mode in (CacheMode.default_on, CacheMode.default_off)]
    return cast(Json, {"modes": modes})


def _semantic_scope_values() -> Json:
    cache = Cache
    tenant_fields = list(cache._SEMANTIC_CACHE_TENANT_SCOPE_FIELDS)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    end_user_field = cache._SEMANTIC_CACHE_END_USER_SCOPE_FIELD  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    excluded = sorted(cast(frozenset[str], cache._SEMANTIC_CACHE_SCOPE_EXCLUDED_PARAMS))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(
        Json,
        {
            "scopes": [scope.value for scope in SemanticCacheScope],
            "tenant_fields": tenant_fields,
            "end_user_field": end_user_field,
            "excluded_params": excluded,
        },
    )


def run(input_value: dict[str, Json]) -> Json:
    (function, payload) = next(iter(input_value.items()))
    if len(input_value) != 1:
        raise ValueError("a b4296 call names exactly one operation")
    data = {} if payload is None else object_value(payload, function)
    match function:
        case "get_cache_key":
            return _get_cache_key(data)
        case "should_use_cache":
            return _should_use_cache(data)
        case "print_verbose":
            return _print_verbose(data)
        case "cache_mode_values":
            return _cache_mode_values()
        case "semantic_scope_values":
            return _semantic_scope_values()
        case _:
            raise ValueError(f"unknown function {function}")
