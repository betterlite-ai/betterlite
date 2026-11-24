import importlib
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import Final, Protocol

import pytest
from fake_redis_server import FakeRedisServer
from pydantic import JsonValue

import betterlite
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.caching import (
    BaseCache,
    Cache,
    CacheMode,
    DiskCache,
    DualCache,
    InMemoryCache,
    LiteLLMCacheType,
    RedisCache,
)
from betterlite.caching.caching import disable_cache, enable_cache, update_cache
from betterlite.types.caching import DEFAULT_CACHING_SUPPORTED_CALL_TYPES

CACHED: Final[dict[str, JsonValue]] = {"answer": 1}
STARTUP_NODES: Final[list[JsonValue]] = [{"host": "127.0.0.1", "port": 6379}]
CACHE_TTL: Final = 5.0
MEMORY_TTL: Final = 30.0
PLAIN_TTL: Final = 7.5
REDIS_TTL: Final = 45.0
REDIS_DEFAULT_TTL: Final = 60
REDIS_DEFAULT_FLUSH_SIZE: Final = 100
CACHE_CALLBACK_COUNTS: Final[list[int]] = [1, 1, 1]
EMPTY_CALLBACK_COUNTS: Final[list[int]] = [0, 0, 0]


class ReferenceCache(Protocol):
    type: object
    mode: object
    ttl: object
    namespace: object
    redis_flush_size: object
    supported_call_types: object
    semantic_cache_scope: object
    cache: object


class ReferenceCacheFactory(Protocol):
    def __call__(self, **kwargs: object) -> ReferenceCache: ...


class ReferenceCachingModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCacheFactory: ...


class ReferenceSwitch(Protocol):
    def __call__(self, **kwargs: object) -> None: ...


class ReferenceSwitchModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceSwitch: ...


class ReferenceStateModule(Protocol):
    def __getattr__(self, name: str) -> object: ...


class ReferenceListModule(Protocol):
    def __getattr__(self, name: str) -> list[object]: ...


# litellm ships its cache types untyped, so its objects come through these typed shapes.
LITELLM_CACHING: ReferenceCachingModule = importlib.import_module("litellm.caching.caching")
LITELLM_SWITCHES: ReferenceSwitchModule = importlib.import_module("litellm.caching.caching")
LITELLM_STATE: ReferenceStateModule = importlib.import_module("litellm")
LITELLM_LISTS: ReferenceListModule = importlib.import_module("litellm")


@pytest.fixture(autouse=True)
def _isolated_cache_state() -> Iterator[None]:
    disable_cache()
    LITELLM_SWITCHES.disable_cache()
    yield
    disable_cache()
    LITELLM_SWITCHES.disable_cache()


@pytest.fixture
def redis_server() -> Iterator[FakeRedisServer]:
    server = FakeRedisServer()
    server.start()
    yield server
    server.stop()
    assert server.unknown_commands == []


def _litellm_callback_lists() -> tuple[Sequence[object], Sequence[object], Sequence[object]]:
    return (
        LITELLM_LISTS.input_callback,
        LITELLM_LISTS.success_callback,
        LITELLM_LISTS._async_success_callback,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L200
    )


def _betterlite_callback_lists() -> tuple[Sequence[object], Sequence[object], Sequence[object]]:
    return (
        betterlite.input_callback,
        betterlite.success_callback,
        betterlite._async_success_callback,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L1044-L1047
    )


def _cache_callback_counts() -> tuple[list[int], list[int]]:
    ours = [
        sum(1 for entry in entries if entry == "cache") for entries in _betterlite_callback_lists()
    ]
    theirs = [
        sum(1 for entry in entries if entry == "cache") for entries in _litellm_callback_lists()
    ]
    return ours, theirs


def test_cache_init_local_defaults() -> None:
    reference = LITELLM_CACHING.Cache()
    ours = Cache()
    assert ours.type == "local"
    assert reference.type == "local"
    assert ours.mode == reference.mode == "default_on"
    assert ours.ttl is None
    assert reference.ttl is None
    assert ours.namespace is None
    assert reference.namespace is None
    assert ours.redis_flush_size is None
    assert reference.redis_flush_size is None
    assert ours.supported_call_types == list(DEFAULT_CACHING_SUPPORTED_CALL_TYPES)
    assert reference.supported_call_types == list(DEFAULT_CACHING_SUPPORTED_CALL_TYPES)
    assert ours.semantic_cache_scope == "key"
    assert reference.semantic_cache_scope == "key"
    backend = ours.cache
    assert isinstance(backend, InMemoryCache)
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS


def test_cache_init_ttl_rules() -> None:
    reference = LITELLM_CACHING.Cache(ttl=CACHE_TTL, default_in_memory_ttl=MEMORY_TTL)
    ours = Cache(ttl=CACHE_TTL, default_in_memory_ttl=MEMORY_TTL)
    assert ours.ttl == reference.ttl == MEMORY_TTL
    reference_plain = LITELLM_CACHING.Cache(ttl=PLAIN_TTL)
    ours_plain = Cache(ttl=PLAIN_TTL)
    assert ours_plain.ttl == reference_plain.ttl == PLAIN_TTL


def test_cache_init_redis_attributes(redis_server: FakeRedisServer) -> None:
    port = str(redis_server.port)
    reference = LITELLM_CACHING.Cache(
        type="redis",
        host=redis_server.host,
        port=port,
        namespace="tenant",
        default_in_redis_ttl=REDIS_TTL,
    )
    ours = Cache(
        type="redis",
        host=redis_server.host,
        port=port,
        namespace="tenant",
        default_in_redis_ttl=REDIS_TTL,
    )
    assert ours.type == "redis"
    assert reference.type == "redis"
    assert ours.namespace == reference.namespace == "tenant"
    assert ours.ttl == reference.ttl == REDIS_TTL
    backend = ours.cache
    assert isinstance(backend, RedisCache)
    assert backend.namespace == "tenant"
    assert backend.default_ttl == REDIS_DEFAULT_TTL
    assert backend.redis_flush_size == REDIS_DEFAULT_FLUSH_SIZE


def test_cache_init_disk_directory(tmp_path: Path) -> None:
    ours = Cache(type="disk", disk_cache_dir=str(tmp_path))
    assert ours.type == "disk"
    assert ours.ttl is None
    backend = ours.cache
    assert isinstance(backend, DiskCache)
    backend.set_cache("key", CACHED)
    assert backend.get_cache("key") == CACHED
    assert backend.get_cache("missing") is None


def test_cache_init_unsupported_type_raises() -> None:
    for cache_type in ("bogus", LiteLLMCacheType.S3, "redis-semantic"):
        with pytest.raises(NotImplementedError, match="cache type is not built yet"):
            Cache(type=cache_type)


def test_cache_init_cluster_settings_raise(monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(NotImplementedError, match="cluster"):
        Cache(type="redis", host="127.0.0.1", port="6379", redis_startup_nodes=STARTUP_NODES)
    monkeypatch.setenv("REDIS_CLUSTER_NODES", '[{"host": "127.0.0.1"}]')
    with pytest.raises(NotImplementedError, match="cluster"):
        Cache(type="redis", host="127.0.0.1", port="6379")
    with pytest.raises(NotImplementedError, match="cluster"):
        RedisCache(host="127.0.0.1", port=6379, startup_nodes=STARTUP_NODES)


def test_cache_init_clock_refused() -> None:
    with pytest.raises(UnsupportedCallArgument):
        InMemoryCache(clock=lambda: 1.0)


def test_cache_import_exports() -> None:
    assert betterlite.Cache is Cache
    assert betterlite.BaseCache is BaseCache
    assert betterlite.CacheMode is CacheMode
    assert betterlite.DiskCache is DiskCache
    assert betterlite.DualCache is DualCache
    assert betterlite.InMemoryCache is InMemoryCache
    assert betterlite.LiteLLMCacheType is LiteLLMCacheType
    assert betterlite.RedisCache is RedisCache
    assert {member.value for member in LiteLLMCacheType} == {
        "local",
        "redis",
        "redis-semantic",
        "valkey-semantic",
        "s3",
        "disk",
        "qdrant-semantic",
        "azure-blob",
        "gcs",
    }
    assert [member.value for member in CacheMode] == ["default_on", "default_off"]


def test_cache_repr_hides_credentials(redis_server: FakeRedisServer) -> None:
    credential = "hunter" + "2"
    port = str(redis_server.port)
    ours = Cache(
        type="redis",
        host=redis_server.host,
        port=port,
        password=credential,
        namespace="tenant",
    )
    reference = LITELLM_CACHING.Cache(
        type="redis",
        host=redis_server.host,
        port=port,
        password=credential,
        namespace="tenant",
    )
    assert "hunter2" not in repr(ours)
    assert "hunter2" not in repr(vars(ours))
    assert "hunter2" not in repr(reference)


def test_enable_cache_adds_cache_and_callbacks() -> None:
    enable_cache()
    LITELLM_SWITCHES.enable_cache()
    assert betterlite.cache is not None
    assert LITELLM_STATE.cache is not None
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS
    enable_cache()
    LITELLM_SWITCHES.enable_cache()
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS


def test_enable_cache_keeps_existing_cache() -> None:
    ours = Cache()
    betterlite.cache = ours
    LITELLM_SWITCHES.update_cache()
    reference = LITELLM_STATE.cache
    enable_cache(type="disk")
    LITELLM_SWITCHES.enable_cache(type="disk")
    assert betterlite.cache is ours
    assert LITELLM_STATE.cache is reference


def test_update_cache_replaces_the_cache() -> None:
    update_cache()
    ours_first = betterlite.cache
    LITELLM_SWITCHES.update_cache()
    reference_first = LITELLM_STATE.cache
    update_cache()
    LITELLM_SWITCHES.update_cache()
    assert isinstance(betterlite.cache, Cache)
    assert betterlite.cache is not ours_first
    assert LITELLM_STATE.cache is not reference_first
    assert LITELLM_STATE.cache is not None
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS


def test_disable_cache_removes_cache_and_callbacks() -> None:
    enable_cache()
    LITELLM_SWITCHES.enable_cache()
    disable_cache()
    LITELLM_SWITCHES.disable_cache()
    assert betterlite.cache is None
    assert LITELLM_STATE.cache is None
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == EMPTY_CALLBACK_COUNTS
