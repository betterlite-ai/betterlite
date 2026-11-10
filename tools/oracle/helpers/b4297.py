import asyncio
from typing import TypeAlias, cast

from litellm.caching.base_cache import BaseCache
from litellm.caching.caching import Cache, CacheMode
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.types.caching import LiteLLMCacheType

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


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


def number_value(value: Json, name: str) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ValueError(f"{name} must be a number")
    return float(value)


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


class RecordingBackend(BaseCache):
    def __init__(self, cached: Json, stamp: float | None) -> None:
        super().__init__()
        self.cached: Json = cached
        self.stamp: float | None = stamp
        self.lookup: dict[str, Json] | None = None
        self.write: dict[str, Json] | None = None

    def _record_lookup(self, kwargs: dict[str, object]) -> None:
        metadata = kwargs.get("metadata")
        if self.stamp is not None and isinstance(metadata, dict):
            cast(dict[str, Json], metadata)["semantic-similarity"] = self.stamp
        recorded: dict[str, Json] = {}
        for name in ("messages", "input", "metadata"):
            if name in kwargs:
                recorded[name] = cast(Json, kwargs[name])
        self.lookup = recorded

    def _record_write(
        self, method: str, key: str, value: object, kwargs: dict[str, object]
    ) -> None:
        ttl_value = kwargs.get("ttl")
        ttl: Json = None
        if isinstance(ttl_value, (int, float)) and not isinstance(ttl_value, bool):
            ttl = float(ttl_value)
        self.write = {
            "method": method,
            "key": key,
            "cached": cast(Json, value),
            "ttl": ttl,
        }

    def get_cache(self, key: str, **kwargs: object) -> object:  # noqa: ARG002
        self._record_lookup(kwargs)
        return self.cached

    async def async_get_cache(self, key: str, **kwargs: object) -> object:  # noqa: ARG002
        self._record_lookup(kwargs)
        return self.cached

    def set_cache(self, key: str, value: object, **kwargs: object) -> None:
        self._record_write("set_cache", key, value, kwargs)

    async def async_set_cache(self, key: str, value: object, **kwargs: object) -> None:
        self._record_write("async_set_cache", key, value, kwargs)

    async def batch_cache_write(self, key: str, value: object, **kwargs: object) -> None:
        self._record_write("batch_cache_write", key, value, kwargs)

    async def async_set_cache_pipeline(self, cache_list: object, **kwargs: object) -> None:  # noqa: ARG002
        return None


def _request_kwargs(request: dict[str, Json]) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    key = object_value(request.get("key"), "key")
    for entry in list_value(key.get("params", []), "params"):
        record = object_value(entry, "param")
        name = string_value(record.get("name"), "name")
        kwargs[name] = record.get("value")
    metadata = optional_object(key.get("metadata"), "metadata")
    if metadata is not None:
        kwargs["metadata"] = metadata
    litellm_metadata = optional_object(key.get("litellm_metadata"), "litellm_metadata")
    if litellm_metadata is not None:
        kwargs["litellm_metadata"] = litellm_metadata
    litellm_params = optional_object(key.get("litellm_params"), "litellm_params")
    if litellm_params is not None:
        kwargs["litellm_params"] = litellm_params
    control: dict[str, Json] = {}
    key_control = optional_object(key.get("cache_control"), "cache_control")
    if key_control is not None:
        control.update(key_control)
    envelope_control = optional_object(request.get("control"), "control")
    if envelope_control is not None:
        control.update(envelope_control)
    if control:
        kwargs["cache"] = control
    cache_key = optional_string(request.get("cache_key"), "cache_key")
    if cache_key is not None:
        kwargs["cache_key"] = cache_key
    return kwargs


def _cache(data: dict[str, Json], backend: BaseCache) -> Cache:
    cache_type = optional_string(data.get("cache_type"), "cache_type")
    cache_mode = optional_string(data.get("mode"), "mode")
    settings = optional_object(data.get("settings"), "settings")
    ttl = None if settings is None else settings.get("ttl")
    flush_size = None if settings is None else settings.get("redis_flush_size")
    redis_flush_size = None if flush_size is None else integer_value(flush_size, "redis_flush_size")
    return Cache(
        type=None if cache_type is None else LiteLLMCacheType(cache_type),
        mode=None if cache_mode is None else CacheMode(cache_mode),
        ttl=None if ttl is None else number_value(ttl, "ttl"),
        redis_flush_size=redis_flush_size,
        _backend=backend,
    )


def _get_cache_logic(data: dict[str, Json]) -> Json:
    cache = Cache(type=LiteLLMCacheType.LOCAL, _backend=InMemoryCache())
    cached: object = data.get("cached")
    max_age_value = data.get("max_age")
    max_age = None if max_age_value is None else number_value(max_age_value, "max_age")
    result = cast(
        Json,
        cache._get_cache_logic(cached_result=cached, max_age=max_age),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return cast(Json, {"result": result})


def _get_cache(data: dict[str, Json]) -> Json:
    request = object_value(data.get("request"), "request")
    stamp_value = data.get("stamp")
    stamp = None if stamp_value is None else number_value(stamp_value, "stamp")
    backend = RecordingBackend(cached=data.get("cached"), stamp=stamp)
    cache = _cache(data, backend)
    kwargs = _request_kwargs(request)
    result = cast(
        Json,
        cache.get_cache(**kwargs),  # pyright: ignore[reportUnknownMemberType, reportArgumentType]
    )
    return cast(
        Json,
        {
            "result": result,
            "metadata": kwargs.get("metadata"),
            "lookup": backend.lookup,
        },
    )


def _add_cache(data: dict[str, Json], *, async_mode: bool) -> Json:
    request = object_value(data.get("request"), "request")
    result = data.get("result")
    backend = RecordingBackend(cached=None, stamp=None)
    cache = _cache(data, backend)
    kwargs = _request_kwargs(request)
    if async_mode:
        asyncio.run(cache.async_add_cache(result=result, **kwargs))  # pyright: ignore[reportUnknownMemberType, reportArgumentType]
    else:
        cache.add_cache(result=result, **kwargs)  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, backend.write)


def run(input_value: dict[str, Json]) -> Json:
    (function, payload) = next(iter(input_value.items()))
    if len(input_value) != 1:
        raise ValueError("a b4297 call names exactly one operation")
    data = {} if payload is None else object_value(payload, function)
    match function:
        case "get_cache_logic":
            return _get_cache_logic(data)
        case "get_cache":
            return _get_cache(data)
        case "add_cache":
            return _add_cache(data, async_mode=False)
        case "async_add_cache":
            return _add_cache(data, async_mode=True)
        case _:
            raise ValueError(f"unknown function {function}")
