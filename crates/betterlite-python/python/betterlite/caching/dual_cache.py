import json
from asyncio import Future
from collections.abc import Generator, Sequence
from typing import Final

from pydantic import JsonValue
from typing_extensions import override

import betterlite
from betterlite._native import NativeDeclaredBatchRead, NativeDeferred, NativeDual
from betterlite.caching._backend import _number, _ttl_of, _value_or_none
from betterlite.caching.base_cache import BaseCache
from betterlite.caching.in_memory_cache import InMemoryCache
from betterlite.caching.redis_cache import RedisCache

DEFAULT_MAX_REDIS_BATCH_CACHE_SIZE: Final = 1000  # ast-grep-ignore: no-long-names
_DEFAULT_BATCH_CACHE_EXPIRY: Final = 10.0


class _DeferredResult:
    def __init__(self, native: NativeDeferred) -> None:
        self.native = native
        self.cancel = native.cancel

    def __await__(self) -> Generator[Future[None], None, None]:
        waiting: Generator[Future[None], None, None] = self.native.async_wait().__await__()
        yield from waiting


# LiteLLM's local_only is a bool; a value of another type names the field and fails.
def _local_only(value: JsonValue) -> bool:
    match value:
        case bool() as flag:
            return flag
        case int() | float() | str() | list() | dict() | None:
            raise ValueError(f"local_only must be a bool, not {type(value).__name__}")


def _batch_values(values: list[str | None] | None) -> list[JsonValue] | None:
    if values is None:
        return None
    return [_value_or_none(value) for value in values]


class DualCache(BaseCache):
    def __init__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L87-L95
        self,
        in_memory_cache: InMemoryCache | None = None,
        redis_cache: RedisCache | None = None,
        default_in_memory_ttl: float | None = None,
        default_redis_ttl: float | None = None,
        # ast-grep-ignore: no-long-names
        default_redis_batch_cache_expiry: float | None = None,
        # ast-grep-ignore: no-long-names
        default_max_redis_batch_cache_size: int = DEFAULT_MAX_REDIS_BATCH_CACHE_SIZE,
    ) -> None:
        super().__init__()
        self.in_memory_cache = InMemoryCache() if in_memory_cache is None else in_memory_cache
        self.redis_cache = redis_cache
        configured_expiry = default_redis_batch_cache_expiry  # ast-grep-ignore: no-long-names
        if configured_expiry is None:
            configured_expiry = betterlite.default_redis_batch_cache_expiry
        self.redis_batch_cache_expiry = (
            _DEFAULT_BATCH_CACHE_EXPIRY if configured_expiry is None else configured_expiry
        )
        configured_memory_ttl = (
            betterlite.default_in_memory_ttl
            if default_in_memory_ttl is None
            else default_in_memory_ttl
        )
        self.default_in_memory_ttl = configured_memory_ttl
        configured_redis_ttl = (
            betterlite.default_redis_ttl if default_redis_ttl is None else default_redis_ttl
        )
        self.default_redis_ttl = configured_redis_ttl
        max_batch = default_max_redis_batch_cache_size  # ast-grep-ignore: no-long-names
        settings: dict[str, JsonValue] = {
            "batch_cache_expiry": self.redis_batch_cache_expiry,
            "max_redis_batch_cache_size": max_batch,
        }
        if configured_memory_ttl is not None:
            settings["default_in_memory_ttl"] = configured_memory_ttl
        if configured_redis_ttl is not None:
            settings["default_redis_ttl"] = configured_redis_ttl
        self.native = NativeDual(
            self.in_memory_cache.native,
            None if redis_cache is None else redis_cache.native,
            json.dumps(settings),
        )

    # LiteLLM stamps default_in_memory_ttl on a memory write that carries no ttl of its own.
    def _memory_kwargs(self, kwargs: dict[str, JsonValue]) -> dict[str, JsonValue]:
        if "ttl" in kwargs or self.default_in_memory_ttl is None:
            return kwargs
        return {**kwargs, "ttl": self.default_in_memory_ttl}

    @override
    def set_cache(
        self,
        key: str,
        value: JsonValue,
        local_only: JsonValue = False,
        **kwargs: JsonValue,
    ) -> None:
        if _local_only(local_only):
            self.in_memory_cache.set_cache(key, value, **self._memory_kwargs(kwargs))
            return
        self.native.set(key, json.dumps(value), _ttl_of(kwargs))

    @override
    async def async_set_cache(
        self,
        key: str,
        value: JsonValue,
        local_only: JsonValue = False,
        **kwargs: JsonValue,
    ) -> None:
        if _local_only(local_only):
            await self.in_memory_cache.async_set_cache(
                key, value, **self._memory_kwargs(kwargs)
            )
            return
        await self.native.async_set(key, json.dumps(value), _ttl_of(kwargs))

    @override
    def get_cache(
        self,
        key: str,
        parent_otel_span: JsonValue = None,
        local_only: JsonValue = False,
        **kwargs: JsonValue,
    ) -> JsonValue:
        _unread = (parent_otel_span, kwargs)
        if _local_only(local_only):
            return self.in_memory_cache.get_cache(key)
        return _value_or_none(self.native.get(key))

    @override
    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: JsonValue = None,
        local_only: JsonValue = False,
        **kwargs: JsonValue,
    ) -> JsonValue:
        _unread = (parent_otel_span, kwargs)
        if not _local_only(local_only):
            return _value_or_none(await self.native.async_get(key))
        return await self.in_memory_cache.async_get_cache(key)

    def batch_get_cache(
        self,
        keys: list[str],
        parent_otel_span: JsonValue = None,
        local_only: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L218-L223
        **kwargs: JsonValue,
    ) -> list[JsonValue] | None:
        _unread = (parent_otel_span, kwargs)
        return _batch_values(self.native.batch_get(keys, local_only))

    async def async_batch_get_cache(
        self,
        keys: list[str],
        parent_otel_span: JsonValue = None,
        local_only: bool = False, throttle_redis: bool = True,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L401-L407
        **kwargs: JsonValue,
    ) -> list[JsonValue] | None:
        _unread = (parent_otel_span, kwargs)
        result = await self.native.async_batch_get(
            keys, local_only, throttle_redis
        )
        return _batch_values(result)

    @staticmethod
    async def async_batch_get_cache_shared(
        reads: Sequence[tuple["DualCache", list[str]]],
    ) -> list[list[JsonValue] | None]:
        native_reads = [(cache.native, keys) for cache, keys in reads]
        answers = await NativeDual.async_batch_get_shared(native_reads)
        return [_batch_values(answer) for answer in answers]

    def declare_batch_get(self, keys: Sequence[str]) -> NativeDeclaredBatchRead | None:
        return self.native.declare_batch_get(list(keys))

    async def async_resolve_batch_get(
        self, declared: NativeDeclaredBatchRead
    ) -> list[JsonValue]:
        answers = await declared.async_resolve()
        return [_value_or_none(value) for value in answers]

    async def async_set_cache_pre_call(
        self, key: str, value: JsonValue, ttl: float | None
    ) -> _DeferredResult | None:
        result = self.native.set_cache_pre_call(key, json.dumps(value), ttl)
        return None if result is None else _DeferredResult(result)

    async def async_delete_cache_pre_call(self, key: str) -> _DeferredResult | None:
        result = self.native.delete_cache_pre_call(key)
        return None if result is None else _DeferredResult(result)

    def increment_cache(
        self,
        key: str,
        value: int,
        local_only: JsonValue = False,  # noqa: FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L164
        **kwargs: JsonValue,
    ) -> int:
        if _local_only(local_only):
            return int(self.in_memory_cache.increment_cache(key, value, **kwargs))
        return int(_number(self.native.increment(key, value, _ttl_of(kwargs))))

    async def async_increment_cache(
        self,
        key: str,
        value: float,
        parent_otel_span: JsonValue = None,
        local_only: JsonValue = False,  # noqa: FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L570
        refresh_ttl: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L571
        **kwargs: JsonValue,
    ) -> float | None:
        _unread = (parent_otel_span, refresh_ttl, kwargs)
        if not _local_only(local_only):
            stepped = await self.native.async_increment(key, value, _ttl_of(kwargs))
            return float(_number(stepped))
        return await self.in_memory_cache.async_increment(key, value, **kwargs)

    @override
    async def async_set_cache_pipeline(
        self,
        cache_list: Sequence[tuple[str, JsonValue]],
        local_only: JsonValue = False,
        **kwargs: JsonValue,
    ) -> None:
        for cache_key, cache_value in cache_list:
            await self.async_set_cache(cache_key, cache_value, local_only, **kwargs)

    def flush_cache(self) -> None:
        self.native.flush()

    def delete_cache(self, key: str) -> None:
        self.native.delete([key])

    async def async_delete_cache(self, key: str) -> None:
        await self.native.async_delete([key])

    async def async_delete_cache_keys(self, keys: Sequence[str]) -> None:
        await self.native.async_delete(list(keys))

    async def async_get_ttl(self, key: str) -> int | float | None:
        text = await self.native.async_ttl(key)
        return None if text is None else _number(text)
