import json
from typing import Final

from pydantic import JsonValue
from typing_extensions import override

import betterlite
from betterlite._native import NativeRedis
from betterlite.caching._backend import (
    _BackendCache,
    _number,
    _nx,
    _ttl_of,
    cluster_nodes_are_set,
)

_DEFAULT_TTL: Final = 60
_DEFAULT_FLUSH_SIZE: Final = 100


class RedisCache(_BackendCache[NativeRedis]):
    _native_ready: bool = False

    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L607-L617
        self,
        host: str | None = None,
        port: int | str | None = None,
        password: str | None = None,
        redis_flush_size: int | None = _DEFAULT_FLUSH_SIZE,
        namespace: str | None = None,
        startup_nodes: list[JsonValue] | None = None,
        socket_timeout: float | None = 5.0,
        **kwargs: JsonValue,
    ) -> None:
        if startup_nodes is not None or cluster_nodes_are_set():
            raise NotImplementedError("the redis cluster cache is not built yet")
        configured: float | None = betterlite.default_redis_ttl
        resolved_ttl = _DEFAULT_TTL if configured is None else int(configured)
        super().__init__(default_ttl=resolved_ttl)
        self._namespace = namespace
        self.redis_flush_size = (
            _DEFAULT_FLUSH_SIZE if redis_flush_size is None else redis_flush_size
        )
        overrides: dict[str, JsonValue] = dict(kwargs)
        if host is not None:
            overrides["host"] = host
        if port is not None:
            overrides["port"] = port
        if password is not None:
            overrides["password"] = password
        if socket_timeout is not None:
            overrides["socket_timeout"] = str(socket_timeout)
        settings: dict[str, JsonValue] = {
            "overrides": overrides,
            "default_ttl": resolved_ttl,
            "redis_flush_size": self.redis_flush_size,
        }
        if namespace is not None:
            settings["namespace"] = namespace
        self.native = NativeRedis(json.dumps(settings))
        self._native_ready = True

    # LiteLLM's Cache assigns a RedisCache's namespace and later keys carry the new prefix.
    @property
    def namespace(self) -> str | None:
        return self._namespace

    @namespace.setter
    def namespace(self, value: str | None) -> None:
        self._namespace = value
        if self._native_ready:
            self.native.set_namespace(value)

    # LiteLLM passes nx to the Redis SET; another type names the field and fails.
    @override
    async def async_set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> bool | None:
        return await self.native.async_set(
            key, json.dumps(value), self._write_ttl(kwargs), _nx(kwargs)
        )

    # LiteLLM's increment is typed int and uses INCR; a missing ttl takes the cache's own default.
    def increment_cache(
        self, key: str, value: int, ttl: float | None = None, **kwargs: JsonValue
    ) -> int:
        effective = ttl if ttl is not None else _ttl_of(kwargs)
        return int(_number(self.native.increment(key, value, effective)))

    async def async_increment(
        self,
        key: str,
        value: float,
        ttl: int | None = None,
        parent_otel_span: JsonValue = None,
        refresh_ttl: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L1419-L1426
    ) -> float:
        _unread = (parent_otel_span, refresh_ttl)
        seconds = None if ttl is None else float(ttl)
        return float(_number(await self.native.async_increment(key, value, seconds)))

    async def delete_cache_keys(self, keys: list[str]) -> None:
        await self.native.async_delete(keys)

    async def ping(self) -> bool:
        return await self.native.async_ping()

    # LiteLLM answers the remaining seconds and None for a key without one.
    async def async_get_ttl(self, key: str) -> int | None:
        text = await self.native.async_ttl(key)
        return None if text is None else int(text)

    # LiteLLM buffers these writes until redis_flush_size; the shared handle writes each one now.
    @override
    async def batch_cache_write(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        await self.async_set_cache(key, value, **kwargs)
