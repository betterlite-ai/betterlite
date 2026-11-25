import asyncio
import importlib
import time
from pathlib import Path
from typing import Final, Protocol, TypedDict

import fake_redis_server
import pytest
from fake_redis_server import FakeRedisServer
from pydantic import JsonValue
from typing_extensions import override

from betterlite._cache_ops.types import CacheStore
from betterlite.caching import Cache, DiskCache, DualCache, InMemoryCache, RedisCache
from betterlite.caching.base_cache import BaseCache


class ChatArgs(TypedDict):
    model: str
    messages: JsonValue


CACHED: Final[dict[str, JsonValue]] = {"answer": 1, "text": "hello"}
CHAT: Final[ChatArgs] = {
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "hello"}],
}
CHAT_RESPONSE: Final[dict[str, JsonValue]] = {"id": "chatcmpl-1", "content": "hello"}
ASYNC_RESPONSE: Final[dict[str, JsonValue]] = {"id": "chatcmpl-2", "content": "async"}
LARGE_COUNT: Final = 9007199254740993
CUSTOM_MEMORY_TTL: Final = 1
MEMORY_TTL_WAIT: Final = 1.5
IN_MEMORY_TTL: Final = 120.0
TTL_ALLOWANCE: Final = 30.0
MEMORY_INCREMENT: Final = 1.5
MEMORY_COUNT: Final = 3.0
REDIS_INCREMENT: Final = 2
REDIS_SECOND_INCREMENT: Final = 3
REDIS_COUNT: Final = 5
REDIS_TTL: Final = 90
REDIS_TTL_ALLOWANCE: Final = 20
DUAL_INCREMENT: Final = 2
DUAL_COUNT: Final = 4
DISK_INCREMENT: Final = 2
DISK_SECOND_INCREMENT: Final = 3
DISK_COUNT: Final = 5


class ReferenceBackend(Protocol):
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue: ...

    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None: ...

    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue: ...

    async def async_set_cache(
        self, key: str, value: JsonValue, **kwargs: JsonValue
    ) -> bool | None: ...

    def increment_cache(self, key: str, value: float, **kwargs: JsonValue) -> float: ...

    async def async_increment(self, key: str, value: float, **kwargs: JsonValue) -> float: ...

    async def async_get_ttl(self, key: str) -> float | None: ...

    def delete_cache(self, key: str) -> None: ...

    async def async_delete_cache(self, key: str) -> None: ...

    def flush_cache(self) -> None: ...

    async def ping(self) -> bool: ...

    async def disconnect(self) -> None: ...


class ReferenceStore(Protocol):
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue: ...

    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None: ...

    def delete_cache(self, key: str) -> None: ...

    def flush_cache(self) -> None: ...


class BackendFactory(Protocol):
    def __call__(self, **kwargs: JsonValue) -> ReferenceBackend: ...


class BackendModule(Protocol):
    def __getattr__(self, name: str) -> BackendFactory: ...


class DualCacheFactory(Protocol):
    def __call__(
        self,
        in_memory_cache: ReferenceBackend | None = None,
        redis_cache: ReferenceBackend | None = None,
    ) -> ReferenceBackend: ...


class DualCacheModule(Protocol):
    def __getattr__(self, name: str) -> DualCacheFactory: ...


class CacheWithBackendFactory(Protocol):
    def __call__(
        self, namespace: str, _backend: ReferenceBackend, **kwargs: JsonValue
    ) -> None: ...


class CacheWithBackendModule(Protocol):
    def __getattr__(self, name: str) -> CacheWithBackendFactory: ...


# litellm ships these classes untyped, so its backends come through this typed shape.
LITELLM_CACHING: BackendModule = importlib.import_module("litellm.caching")
LITELLM_DUAL: DualCacheModule = importlib.import_module("litellm.caching")
LITELLM_WITH_BACKEND: CacheWithBackendModule = importlib.import_module("litellm.caching")

# The shared fake-server fixture, registered here under the name the tests ask for.
redis_server = fake_redis_server.redis_server


async def _in_memory_round_trip(
    ours: InMemoryCache, theirs: ReferenceBackend
) -> tuple[JsonValue, JsonValue, float | None, float | None]:
    await ours.async_set_cache("ours:memory-async", CACHED)
    await theirs.async_set_cache("litellm:memory-async", CACHED)
    await ours.async_set_cache("ours:memory-ttl", CACHED, ttl=IN_MEMORY_TTL)
    await theirs.async_set_cache("litellm:memory-ttl", CACHED, ttl=IN_MEMORY_TTL)
    return (
        await ours.async_get_cache("ours:memory-async"),
        await theirs.async_get_cache("litellm:memory-async"),
        await ours.async_get_ttl("ours:memory-ttl"),
        await theirs.async_get_ttl("litellm:memory-ttl"),
    )


async def _redis_scenario(
    ours: RedisCache, theirs: ReferenceBackend
) -> tuple[
    JsonValue,
    JsonValue,
    float | None,
    float | None,
    float,
    float,
    bool,
    bool,
    JsonValue,
    JsonValue,
]:
    await ours.async_set_cache("ours:redis-async", CACHED)
    await theirs.async_set_cache("litellm:redis-async", CACHED)
    await ours.async_set_cache("ours:redis-ttl", CACHED, ttl=REDIS_TTL)
    await theirs.async_set_cache("litellm:redis-ttl", CACHED, ttl=REDIS_TTL)
    ours_value = await ours.async_get_cache("ours:redis-async")
    theirs_value = await theirs.async_get_cache("litellm:redis-async")
    ours_ttl = await ours.async_get_ttl("ours:redis-ttl")
    theirs_ttl = await theirs.async_get_ttl("litellm:redis-ttl")
    ours_count = await ours.async_increment("ours:redis-count", REDIS_INCREMENT, ttl=REDIS_TTL)
    theirs_count = await theirs.async_increment(
        "litellm:redis-count", REDIS_INCREMENT, ttl=REDIS_TTL
    )
    await ours.async_delete_cache("ours:redis-async")
    await theirs.async_delete_cache("litellm:redis-async")
    ours_deleted = await ours.async_get_cache("ours:redis-async")
    theirs_deleted = await theirs.async_get_cache("litellm:redis-async")
    ours_ping = await ours.ping()
    theirs_ping = await theirs.ping()
    await ours.disconnect()
    await theirs.disconnect()
    return (
        ours_value,
        theirs_value,
        ours_ttl,
        theirs_ttl,
        ours_count,
        theirs_count,
        ours_ping,
        theirs_ping,
        ours_deleted,
        theirs_deleted,
    )


class DynamicCache(BaseCache):
    def __init__(self) -> None:
        super().__init__()
        self.values: dict[str, JsonValue] = {}

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: JsonValue, **_kwargs: JsonValue) -> None:
        self.values[key] = value

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.values.get(key)

    @override
    async def async_set_cache(
        self, key: str, value: JsonValue, **_kwargs: JsonValue
    ) -> bool | None:
        self.values[key] = value

    @override
    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, JsonValue]], **_kwargs: JsonValue
    ) -> None:
        for key, value in cache_list:
            self.values[key] = value


def _check_store(ours: ReferenceStore, theirs: ReferenceStore, name: str) -> None:
    ours.set_cache(f"ours:{name}", CACHED)
    theirs.set_cache(f"litellm:{name}", CACHED)
    assert ours.get_cache(f"ours:{name}") == CACHED
    assert theirs.get_cache(f"litellm:{name}") == CACHED
    assert ours.get_cache(f"ours:{name}-missing") is None
    assert theirs.get_cache(f"litellm:{name}-missing") is None
    ours.delete_cache(f"ours:{name}")
    theirs.delete_cache(f"litellm:{name}")
    assert ours.get_cache(f"ours:{name}") is None
    assert theirs.get_cache(f"litellm:{name}") is None


def _check_flushed(ours: ReferenceStore, theirs: ReferenceStore, name: str) -> None:
    ours.flush_cache()
    theirs.flush_cache()
    assert ours.get_cache(f"ours:{name}") is None
    assert theirs.get_cache(f"litellm:{name}") is None


def _count_memory(ours: InMemoryCache, theirs: ReferenceBackend) -> None:
    assert ours.increment_cache("ours:memory-count", MEMORY_INCREMENT) == MEMORY_INCREMENT
    assert theirs.increment_cache("litellm:memory-count", MEMORY_INCREMENT) == MEMORY_INCREMENT
    assert ours.increment_cache("ours:memory-count", MEMORY_INCREMENT) == MEMORY_COUNT
    assert theirs.increment_cache("litellm:memory-count", MEMORY_INCREMENT) == MEMORY_COUNT
    _check_flushed(ours, theirs, "memory-count")


def _count_redis(ours: RedisCache, theirs: ReferenceBackend) -> None:
    ttl_write = ours.increment_cache("ours:redis-count", REDIS_INCREMENT, ttl=REDIS_TTL)
    assert ttl_write == REDIS_INCREMENT
    theirs_first = theirs.increment_cache("litellm:redis-count", REDIS_INCREMENT)
    assert theirs_first == REDIS_INCREMENT
    assert ours.increment_cache("ours:redis-count", REDIS_SECOND_INCREMENT) == REDIS_COUNT
    assert theirs.increment_cache("litellm:redis-count", REDIS_SECOND_INCREMENT) == REDIS_COUNT
    _check_flushed(ours, theirs, "redis-count")


def _memory_store(cache: Cache) -> InMemoryCache:
    match cache.cache:
        case InMemoryCache() as store:
            return store
        case CacheStore():
            raise AssertionError(f"{type(cache.cache).__name__} is not an InMemoryCache")


def test_memory_matches_litellm() -> None:
    ours = InMemoryCache()
    theirs = LITELLM_CACHING.InMemoryCache()
    _check_store(ours, theirs, "memory")
    _count_memory(ours, theirs)


def test_memory_async_matches() -> None:
    ours = InMemoryCache()
    theirs = LITELLM_CACHING.InMemoryCache()
    before = time.time()
    ours_value, theirs_value, ours_ttl, theirs_ttl = asyncio.run(
        _in_memory_round_trip(ours, theirs)
    )
    assert ours_value == theirs_value == CACHED
    assert ours_ttl is not None
    assert theirs_ttl is not None
    assert before + IN_MEMORY_TTL - TTL_ALLOWANCE <= ours_ttl
    assert ours_ttl <= before + IN_MEMORY_TTL + TTL_ALLOWANCE
    assert before + IN_MEMORY_TTL - TTL_ALLOWANCE <= theirs_ttl
    assert theirs_ttl <= before + IN_MEMORY_TTL + TTL_ALLOWANCE


def test_redis_matches_litellm(redis_server: FakeRedisServer) -> None:
    ours = RedisCache(host=redis_server.host, port=redis_server.port)
    theirs = LITELLM_CACHING.RedisCache(host=redis_server.host, port=redis_server.port)
    _check_store(ours, theirs, "redis")
    _count_redis(ours, theirs)


def test_redis_async_matches(redis_server: FakeRedisServer) -> None:
    ours = RedisCache(host=redis_server.host, port=redis_server.port)
    theirs = LITELLM_CACHING.RedisCache(host=redis_server.host, port=redis_server.port)
    (
        ours_value,
        theirs_value,
        ours_ttl,
        theirs_ttl,
        ours_count,
        theirs_count,
        ours_ping,
        theirs_ping,
        ours_deleted,
        theirs_deleted,
    ) = asyncio.run(_redis_scenario(ours, theirs))
    assert ours_value == theirs_value == CACHED
    assert ours_ttl is not None
    assert theirs_ttl is not None
    assert ours_ttl >= REDIS_TTL - REDIS_TTL_ALLOWANCE
    assert ours_ttl <= REDIS_TTL
    assert theirs_ttl >= REDIS_TTL - REDIS_TTL_ALLOWANCE
    assert theirs_ttl <= REDIS_TTL
    assert ours_count == theirs_count == float(REDIS_INCREMENT)
    assert ours_ping is True
    assert theirs_ping is True
    assert ours_deleted is None
    assert theirs_deleted is None


def test_redis_counts_large_ints(redis_server: FakeRedisServer) -> None:
    ours = RedisCache(host=redis_server.host, port=redis_server.port)
    theirs = LITELLM_CACHING.RedisCache(host=redis_server.host, port=redis_server.port)
    assert ours.increment_cache("ours:large", LARGE_COUNT) == LARGE_COUNT
    assert theirs.increment_cache("litellm:large", LARGE_COUNT) == LARGE_COUNT


def test_redis_async_set_passes_nx(redis_server: FakeRedisServer) -> None:
    ours = RedisCache(host=redis_server.host, port=redis_server.port)
    assert asyncio.run(ours.async_set_cache("ours:nx", CACHED, nx=True)) is True
    assert asyncio.run(ours.async_set_cache("ours:nx", "replaced", nx=True)) is None
    assert ours.get_cache("ours:nx") == CACHED
    asyncio.run(ours.async_set_cache("ours:nx", "replaced"))
    assert ours.get_cache("ours:nx") == "replaced"
    with pytest.raises(ValueError, match="nx must be a bool"):
        asyncio.run(ours.async_set_cache("ours:nx", CACHED, nx=1))


def test_namespace_reaches_redis(redis_server: FakeRedisServer) -> None:
    port = str(redis_server.port)
    ours_backend = RedisCache(host=redis_server.host, port=port, namespace="old")
    Cache(type="redis", namespace="new", _backend=ours_backend)
    theirs_backend = LITELLM_CACHING.RedisCache(
        host=redis_server.host, port=port, namespace="old"
    )
    LITELLM_WITH_BACKEND.Cache(type="redis", namespace="new", _backend=theirs_backend)
    ours_backend.set_cache("ours:key", CACHED)
    theirs_backend.set_cache("litellm:key", CACHED)
    assert redis_server.stored("new:ours:key") is not None
    assert redis_server.stored("old:ours:key") is None
    assert redis_server.stored("new:litellm:key") is not None
    assert redis_server.stored("old:litellm:key") is None


def test_dual_matches_litellm() -> None:
    ours = DualCache()
    theirs = LITELLM_CACHING.DualCache()
    assert ours.redis_cache is None
    ours.set_cache("ours:dual", CACHED)
    theirs.set_cache("litellm:dual", CACHED)
    assert ours.get_cache("ours:dual") == CACHED
    assert theirs.get_cache("litellm:dual") == CACHED
    assert ours.get_cache("ours:dual-missing") is None
    assert theirs.get_cache("litellm:dual-missing") is None
    ours.increment_cache("ours:dual-count", DUAL_INCREMENT)
    theirs.increment_cache("litellm:dual-count", DUAL_INCREMENT)
    assert float(ours.increment_cache("ours:dual-count", DUAL_INCREMENT)) == DUAL_COUNT
    assert float(theirs.increment_cache("litellm:dual-count", DUAL_INCREMENT)) == DUAL_COUNT
    ours.delete_cache("ours:dual")
    theirs.delete_cache("litellm:dual")
    assert ours.get_cache("ours:dual") is None
    assert theirs.get_cache("litellm:dual") is None


def test_dual_local_only_stays(redis_server: FakeRedisServer) -> None:
    redis_ours = RedisCache(host=redis_server.host, port=redis_server.port)
    ours = DualCache(in_memory_cache=InMemoryCache(), redis_cache=redis_ours)
    redis_theirs = LITELLM_CACHING.RedisCache(host=redis_server.host, port=redis_server.port)
    theirs = LITELLM_DUAL.DualCache(
        in_memory_cache=LITELLM_CACHING.InMemoryCache(), redis_cache=redis_theirs
    )
    ours.set_cache("ours:dual-local", CACHED, local_only=True)
    theirs.set_cache("litellm:dual-local", CACHED, local_only=True)
    assert ours.get_cache("ours:dual-local") == CACHED
    assert theirs.get_cache("litellm:dual-local") == CACHED
    fresh_ours = DualCache(in_memory_cache=InMemoryCache(), redis_cache=redis_ours)
    fresh_theirs = LITELLM_DUAL.DualCache(
        in_memory_cache=LITELLM_CACHING.InMemoryCache(), redis_cache=redis_theirs
    )
    assert fresh_ours.get_cache("ours:dual-local") is None
    assert fresh_theirs.get_cache("litellm:dual-local") is None


def test_dual_keeps_memory_ttl(redis_server: FakeRedisServer) -> None:
    redis_ours = RedisCache(host=redis_server.host, port=redis_server.port)
    ours = DualCache(
        in_memory_cache=InMemoryCache(default_ttl=CUSTOM_MEMORY_TTL), redis_cache=redis_ours
    )
    redis_theirs = LITELLM_CACHING.RedisCache(host=redis_server.host, port=redis_server.port)
    theirs = LITELLM_DUAL.DualCache(
        in_memory_cache=LITELLM_CACHING.InMemoryCache(default_ttl=CUSTOM_MEMORY_TTL),
        redis_cache=redis_theirs,
    )
    ours.set_cache("ours:ttl-write", CACHED)
    theirs.set_cache("litellm:ttl-write", CACHED)
    assert ours.increment_cache("ours:ttl-count", 1) == 1
    assert theirs.increment_cache("litellm:ttl-count", 1) == 1
    redis_ours.set_cache("ours:ttl-backfill", CACHED)
    redis_theirs.set_cache("litellm:ttl-backfill", CACHED)
    assert ours.get_cache("ours:ttl-backfill") == CACHED
    assert theirs.get_cache("litellm:ttl-backfill") == CACHED
    time.sleep(MEMORY_TTL_WAIT)
    assert ours.get_cache("ours:ttl-write", local_only=True) is None
    assert theirs.get_cache("litellm:ttl-write", local_only=True) is None
    assert ours.get_cache("ours:ttl-count", local_only=True) is None
    assert theirs.get_cache("litellm:ttl-count", local_only=True) is None
    assert ours.get_cache("ours:ttl-backfill", local_only=True) is None
    assert theirs.get_cache("litellm:ttl-backfill", local_only=True) is None


def test_dual_batch_reads_include_memory_and_redis(redis_server: FakeRedisServer) -> None:
    ours_redis = RedisCache(host=redis_server.host, port=redis_server.port)
    ours = DualCache(in_memory_cache=InMemoryCache(), redis_cache=ours_redis)
    ours.set_cache("ours:batch-redis", CACHED)
    ours.in_memory_cache.set_cache("ours:batch-memory", CHAT_RESPONSE)
    sync_values = ours.batch_get_cache(["ours:batch-memory", "ours:batch-redis"])
    assert sync_values == [CHAT_RESPONSE, CACHED]

    ours_redis.set_cache("ours:async-batch", CACHED)
    async_values = asyncio.run(
        ours.async_batch_get_cache(["ours:batch-memory", "ours:async-batch"])
    )
    assert async_values == [CHAT_RESPONSE, CACHED]

    second = DualCache(in_memory_cache=InMemoryCache(), redis_cache=ours_redis)
    shared = asyncio.run(
        DualCache.async_batch_get_cache_shared(
            [(ours, ["ours:batch-redis"]), (second, ["ours:async-batch"])]
        )
    )
    assert shared == [[CACHED], [CACHED]]


def test_dual_deferred_methods_without_request_batch() -> None:
    cache = DualCache()
    assert cache.declare_batch_get(["key"]) is None
    assert asyncio.run(cache.async_set_cache_pre_call("key", CACHED, None)) is None
    assert asyncio.run(cache.async_delete_cache_pre_call("key")) is None


# LiteLLM's DiskCache needs the diskcache package, which the oracle environment does not install.
def test_disk_backend_contract(tmp_path: Path) -> None:
    ours = DiskCache(disk_cache_dir=str(tmp_path))
    ours.set_cache("disk", CACHED)
    assert ours.get_cache("disk") == CACHED
    assert ours.get_cache("disk-missing") is None
    assert ours.increment_cache("disk-count", DISK_INCREMENT) == DISK_INCREMENT
    assert ours.increment_cache("disk-count", DISK_SECOND_INCREMENT) == DISK_COUNT
    ours.set_cache("disk-text", "hello")
    assert ours.increment_cache("disk-text", DISK_INCREMENT) == DISK_INCREMENT
    ours.set_cache("disk-zero", 0)
    assert ours.get_cache("disk-zero") is None
    ours.delete_cache("disk")
    assert ours.get_cache("disk") is None
    ours.flush_cache()
    assert ours.get_cache("disk-text") is None


def test_disk_counts_large_ints(tmp_path: Path) -> None:
    ours = DiskCache(disk_cache_dir=str(tmp_path))
    assert ours.increment_cache("large", LARGE_COUNT) == LARGE_COUNT
    assert ours.increment_cache("large", 1) == LARGE_COUNT + 1


def test_lookups_share_the_store() -> None:
    cache = Cache()
    store = _memory_store(cache)
    cache.add_cache(CHAT_RESPONSE, model=CHAT["model"], messages=CHAT["messages"])
    assert cache.get_cache(model=CHAT["model"], messages=CHAT["messages"]) == CHAT_RESPONSE
    store.flush_cache()
    assert cache.get_cache(model=CHAT["model"], messages=CHAT["messages"]) is None
    cache.add_cache(CHAT_RESPONSE, model=CHAT["model"], messages=CHAT["messages"])
    store.delete_cache(cache.get_cache_key(model=CHAT["model"], messages=CHAT["messages"]))
    assert cache.get_cache(model=CHAT["model"], messages=CHAT["messages"]) is None


def test_dynamic_cache_object() -> None:
    dynamic = DynamicCache()
    cache = Cache(_backend=dynamic)
    cache.add_cache(
        CHAT_RESPONSE, dynamic, model=CHAT["model"], messages=CHAT["messages"]
    )
    assert cache.get_cache(dynamic, model=CHAT["model"], messages=CHAT["messages"]) == CHAT_RESPONSE
    assert asyncio.run(
        cache.async_get_cache(dynamic, model=CHAT["model"], messages=CHAT["messages"])
    ) == CHAT_RESPONSE
    dynamic.values.clear()
    asyncio.run(
        cache.async_add_cache(
            ASYNC_RESPONSE, dynamic, model=CHAT["model"], messages=CHAT["messages"]
        )
    )
    assert asyncio.run(
        cache.async_get_cache(dynamic, model=CHAT["model"], messages=CHAT["messages"])
    ) == ASYNC_RESPONSE
