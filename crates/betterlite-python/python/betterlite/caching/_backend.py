import json
import os
from typing import Final, Generic, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite._native import NativeDisk, NativeDual, NativeMemory, NativeRedis
from betterlite.caching.base_cache import BaseCache

_NATIVE_HANDLE: TypeAlias = NativeMemory | NativeDisk | NativeRedis | NativeDual
_HandleT = TypeVar("_HandleT", bound=_NATIVE_HANDLE)
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_CLUSTER_NODES_VARIABLE: Final = "REDIS_CLUSTER_NODES"


# LiteLLM selects the cluster cache when redis_startup_nodes or REDIS_CLUSTER_NODES is set.
def cluster_nodes_are_set() -> bool:
    raw = os.environ.get(_CLUSTER_NODES_VARIABLE)
    return raw is not None and raw.strip() != ""


def value_or_none(text: str | None) -> JsonValue:
    if text is None:
        return None
    return _JSON.validate_json(text)


def number_value(text: str) -> int | float:
    decoded = _JSON.validate_json(text)
    match decoded:
        case bool() | str() | list() | dict() | None:
            raise ValueError(f"the cache answered {decoded!r}, not a number")
        case int() as whole:
            return whole
        case float() as fraction:
            return fraction


def ttl_of(kwargs: dict[str, JsonValue]) -> float | None:
    raw = kwargs.get("ttl")
    if raw is None:
        return None
    match raw:
        case bool() as flag:
            return float(flag)
        case int() | float() as seconds:
            return float(seconds)
        case str() | list() | dict():
            raise ValueError(f"ttl must be a number, not {type(raw).__name__}")


# LiteLLM reads nx with kwargs.get("nx", False); another type names the field and fails.
def nx_value(kwargs: dict[str, JsonValue]) -> bool:
    raw = kwargs.get("nx")
    if raw is None:
        return False
    match raw:
        case bool() as flag:
            return flag
        case int() | float() | str() | list() | dict():
            raise ValueError(f"nx must be a bool, not {type(raw).__name__}")


class BackendCache(BaseCache, Generic[_HandleT]):
    native: _HandleT

    # A tier with its own default ttl puts it on every write; the others let Rust apply theirs.
    def _write_ttl(self, kwargs: dict[str, JsonValue]) -> float | None:
        return ttl_of(kwargs)

    @override
    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        self.native.set(key, json.dumps(value), self._write_ttl(kwargs))

    @override
    async def async_set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> bool | None:
        await self.native.async_set(key, json.dumps(value), self._write_ttl(kwargs))
        return None

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return value_or_none(self.native.get(key))

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return value_or_none(await self.native.async_get(key))

    def delete_cache(self, key: str) -> None:
        self.native.delete([key])

    async def async_delete_cache(self, key: str) -> None:
        await self.native.async_delete([key])

    async def async_delete_cache_keys(self, keys: list[str]) -> None:
        await self.native.async_delete(keys)

    @override
    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, JsonValue]], **kwargs: JsonValue
    ) -> None:
        for cache_key, cache_value in cache_list:
            await self.async_set_cache(cache_key, cache_value, **kwargs)

    def flush_cache(self) -> None:
        self.native.flush()

    # LiteLLM leaves batch_cache_write unimplemented outside Redis; the shared handle writes now.
    @override
    async def batch_cache_write(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        await self.async_set_cache(key, value, **kwargs)

    @override
    async def disconnect(self) -> None:
        await self.native.async_close()
