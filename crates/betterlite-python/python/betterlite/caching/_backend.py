import json
import os
from typing import Final, Generic, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter

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


def _text(value: JsonValue) -> str:
    return json.dumps(value)


def _value(text: str) -> JsonValue:
    return _JSON.validate_json(text)


def _number(text: str) -> int | float:
    match _JSON.validate_json(text):
        case int() as whole:
            return whole
        case float() as fraction:
            return fraction
        case other:
            raise ValueError(f"the cache answered {other!r}, not a number")


def _ttl_of(kwargs: dict[str, JsonValue]) -> float | None:
    match kwargs.get("ttl"):
        case None:
            return None
        case int() as seconds:
            return float(seconds)
        case float() as seconds:
            return seconds
        case other:
            raise ValueError(f"ttl must be a number, not {type(other).__name__}")


class _BackendCache(BaseCache, Generic[_HandleT]):
    native: _HandleT

    # A memory tier that carries its own default ttl puts it on every write; the other tiers let Rust apply theirs.
    def _write_ttl(self, kwargs: dict[str, JsonValue]) -> float | None:
        return _ttl_of(kwargs)

    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        self.native.set(key, _text(value), self._write_ttl(kwargs))

    async def async_set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        await self.native.async_set(key, _text(value), self._write_ttl(kwargs))

    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        text = self.native.get(key)
        return None if text is None else _value(text)

    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        text = await self.native.async_get(key)
        return None if text is None else _value(text)

    def delete_cache(self, key: str) -> None:
        self.native.delete([key])

    async def async_delete_cache(self, key: str) -> None:
        await self.native.async_delete([key])

    async def async_delete_cache_keys(self, keys: list[str]) -> None:
        await self.native.async_delete(keys)

    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, JsonValue]], **kwargs: JsonValue
    ) -> None:
        for cache_key, cache_value in cache_list:
            await self.async_set_cache(cache_key, cache_value, **kwargs)

    def flush_cache(self) -> None:
        self.native.flush()

    # LiteLLM's BaseCache leaves batch_cache_write unimplemented outside Redis; the shared handle writes now.
    async def batch_cache_write(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        await self.async_set_cache(key, value, **kwargs)

    async def disconnect(self) -> None:
        await self.native.async_close()

    async def test_connection(self) -> dict[str, JsonValue]:
        alive = await self.native.async_ping()
        return {"status": "success" if alive else "failed"}
