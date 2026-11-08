import asyncio
from typing import TypeAlias, cast

from litellm.caching.base_cache import BaseCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.types.caching import (
    DEFAULT_CACHING_SUPPORTED_CALL_TYPES,
    EMBEDDING_CACHE_FORMAT_VERSION,
    CachePingResponse,
    CachedEmbedding,
    DynamicCacheControl,
    HealthCheckCacheParams,
    LiteLLMCacheType,
    RedisPipelineIncrementOperation,
    RedisPipelineLpopOperation,
    RedisPipelineRpushOperation,
    RedisPipelineSetOperation,
    SemanticCacheScope,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_DYNAMIC_CACHE_CONTROL_KEYS = (
    "ttl",
    "namespace",
    "s-maxage",
    "s-max-age",
    "no-cache",
    "no-store",
)


class _BaseProbe(BaseCache):
    async def async_set_cache_pipeline(self, _cache_list: object, **_kwargs: object) -> None:
        return None


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
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def optional_bool(value: Json, name: str) -> bool | None:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def string_or_integer(value: Json, name: str) -> str | int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        raise ValueError(f"{name} must be a string or an integer")
    return value


def string_or_number(value: Json, name: str) -> str | int | float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (str, int, float)):
        raise ValueError(f"{name} must be a string or a number")
    return value


def _memory(clock: float) -> InMemoryCache:
    return InMemoryCache(clock=lambda: clock)


async def _stored_ttl(cache: InMemoryCache, key: str, value: str, ttl: float) -> float | None:
    await cache.async_set_cache(key, value, ttl=ttl)
    return await cache.async_get_ttl(key)


def _base_cache_contract_get() -> Json:
    probe = _BaseProbe()
    return cast(Json, probe.get_cache("key"))


def _base_cache_contract_set() -> Json:
    probe = _BaseProbe()
    return cast(Json, probe.set_cache("key", 1))


def _base_cache_test_connection_unsupported() -> Json:
    return cast(Json, asyncio.run(_BaseProbe().test_connection()))


def _memory_async_get_absolute_ttl(input_value: dict[str, Json]) -> Json:
    clock = number_value(input_value.get("clock"), "clock")
    ttl = number_value(input_value.get("ttl"), "ttl")
    key = string_value(input_value.get("key"), "key")
    value = string_value(input_value.get("value"), "value")
    stored = asyncio.run(_stored_ttl(_memory(clock), key, value, ttl))
    return cast(Json, {"ttl": stored})


def _increment_operation(entry: Json) -> RedisPipelineIncrementOperation:
    operation = object_value(entry, "increment")
    return {
        "key": string_value(operation.get("key"), "key"),
        "increment_value": number_value(
            operation.get("increment_value"), "increment_value"
        ),
        "ttl": None,
    }


def _memory_async_increment_pipeline(input_value: dict[str, Json]) -> Json:
    clock = number_value(input_value.get("clock"), "clock")
    raw = list_value(input_value.get("increments"), "increments")
    operations = [_increment_operation(entry) for entry in raw]
    results = asyncio.run(_memory(clock).async_increment_pipeline(operations))
    return cast(Json, {"results": results})


async def _oldest_keys(cache: InMemoryCache, entries: list[Json], limit: int) -> list[str]:
    for entry in entries:
        record = object_value(entry, "entry")
        key = string_value(record.get("key"), "key")
        await cache.async_set_cache(key, key, ttl=number_value(record.get("ttl"), "ttl"))
    return await cache.async_get_oldest_n_keys(limit)


def _async_get_oldest_n_keys_ordering(input_value: dict[str, Json]) -> Json:
    clock = number_value(input_value.get("clock"), "clock")
    entries = list_value(input_value.get("entries"), "entries")
    limit = optional_integer(input_value.get("limit"), "limit")
    if limit is None:
        raise ValueError("limit must be an integer")
    keys = asyncio.run(_oldest_keys(_memory(clock), entries, limit))
    return cast(Json, {"keys": keys})


def _dynamic_cache_control_ttl(input_value: dict[str, Json]) -> Json:
    cache = object_value(input_value.get("cache"), "cache")
    control = cast(
        DynamicCacheControl,
        {key: cache[key] for key in _DYNAMIC_CACHE_CONTROL_KEYS if key in cache},
    )
    return cast(Json, {"cache": control})


def _semantic_scope_key_and_user() -> Json:
    scopes = [SemanticCacheScope.KEY.value, SemanticCacheScope.END_USER.value]
    return cast(Json, {"scopes": scopes})


def _cached_embedding_format_version_2(input_value: dict[str, Json]) -> Json:
    record = cast(CachedEmbedding, object_value(input_value.get("embedding"), "embedding"))
    if record["format_version"] != EMBEDDING_CACHE_FORMAT_VERSION:
        raise ValueError("the cached embedding format version is not 2")
    return cast(Json, {"embedding": record})


def _supported_call_types() -> Json:
    call_types = list(DEFAULT_CACHING_SUPPORTED_CALL_TYPES)
    return cast(Json, {"call_types": call_types})


def _cache_type_round_trip() -> Json:
    wires = [cache_type.value for cache_type in LiteLLMCacheType]
    for wire in wires:
        if LiteLLMCacheType(wire).value != wire:
            raise ValueError(f"{wire} did not round trip")
    return cast(Json, {"cache_types": wires})


def _ping_response_health_check(input_value: dict[str, Json]) -> Json:
    ping = object_value(input_value.get("ping"), "ping")
    params = object_value(ping.get("health_check_cache_params"), "health_check_cache_params")
    health = HealthCheckCacheParams(
        host=optional_string(params.get("host"), "host"),
        port=string_or_integer(params.get("port"), "port"),
        redis_kwargs=optional_object(params.get("redis_kwargs"), "redis_kwargs"),
        namespace=optional_string(params.get("namespace"), "namespace"),
        redis_version=string_or_number(params.get("redis_version"), "redis_version"),
    )
    response = CachePingResponse(
        status=string_value(ping.get("status"), "status"),
        cache_type=string_value(ping.get("cache_type"), "cache_type"),
        ping_response=optional_bool(ping.get("ping_response"), "ping_response"),
        set_cache_response=optional_string(
            ping.get("set_cache_response"), "set_cache_response"
        ),
        litellm_cache_params=optional_string(
            ping.get("litellm_cache_params"), "litellm_cache_params"
        ),
        health_check_cache_params=health.model_dump(),
    )
    return cast(Json, {"ping": response.model_dump()})


def _redis_pipeline_increment(input_value: dict[str, Json]) -> Json:
    operation = object_value(input_value.get("operation"), "operation")
    record: RedisPipelineIncrementOperation = {
        "key": string_value(operation.get("key"), "key"),
        "increment_value": number_value(
            operation.get("increment_value"), "increment_value"
        ),
        "ttl": optional_integer(operation.get("ttl"), "ttl"),
    }
    return cast(Json, {"operation": record})


def _redis_pipeline_set(input_value: dict[str, Json]) -> Json:
    operation = object_value(input_value.get("operation"), "operation")
    record: RedisPipelineSetOperation = {
        "key": string_value(operation.get("key"), "key"),
        "value": operation.get("value"),
        "ttl": optional_integer(operation.get("ttl"), "ttl"),
    }
    return cast(Json, {"operation": record})


def _redis_pipeline_rpush(input_value: dict[str, Json]) -> Json:
    operation = object_value(input_value.get("operation"), "operation")
    record: RedisPipelineRpushOperation = {
        "key": string_value(operation.get("key"), "key"),
        "values": list_value(operation.get("values"), "values"),
    }
    return cast(Json, {"operation": record})


def _redis_pipeline_lpop(input_value: dict[str, Json]) -> Json:
    operation = object_value(input_value.get("operation"), "operation")
    record: RedisPipelineLpopOperation = {
        "key": string_value(operation.get("key"), "key"),
        "count": optional_integer(operation.get("count"), "count"),
    }
    return cast(Json, {"operation": record})


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    match function:
        case "base_cache_contract_get":
            return _base_cache_contract_get()
        case "base_cache_contract_set":
            return _base_cache_contract_set()
        case "base_cache_test_connection_unsupported":
            return _base_cache_test_connection_unsupported()
        case "memory_async_get_absolute_ttl":
            return _memory_async_get_absolute_ttl(input_value)
        case "memory_async_increment_pipeline":
            return _memory_async_increment_pipeline(input_value)
        case "dynamic_cache_control_ttl":
            return _dynamic_cache_control_ttl(input_value)
        case "semantic_scope_key_and_user":
            return _semantic_scope_key_and_user()
        case "cached_embedding_format_version_2":
            return _cached_embedding_format_version_2(input_value)
        case "supported_call_types":
            return _supported_call_types()
        case "cache_type_round_trip":
            return _cache_type_round_trip()
        case "ping_response_health_check":
            return _ping_response_health_check(input_value)
        case "redis_pipeline_increment":
            return _redis_pipeline_increment(input_value)
        case "redis_pipeline_set":
            return _redis_pipeline_set(input_value)
        case "redis_pipeline_rpush":
            return _redis_pipeline_rpush(input_value)
        case "redis_pipeline_lpop":
            return _redis_pipeline_lpop(input_value)
        case "async_get_oldest_n_keys_ordering":
            return _async_get_oldest_n_keys_ordering(input_value)
        case _:
            raise ValueError(f"unknown function {function}")
