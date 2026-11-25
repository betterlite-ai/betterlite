import importlib
import time
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import Final, Protocol

import fake_redis_server
import pytest
from fake_redis_server import FakeRedisServer
from pydantic import JsonValue

import betterlite
from betterlite._cache_ops.types import CacheStore
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._settings.callbacks import Callback
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
from betterlite.types.caching import (
    DEFAULT_CACHING_SUPPORTED_CALL_TYPES as SUPPORTED_CALLS,
)
from betterlite.types.caching import CachingSupportedCallTypes as CallType

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
    type: str
    mode: str
    ttl: float | None
    namespace: str | None
    redis_flush_size: int | None
    supported_call_types: list[CallType] | None
    semantic_cache_scope: str


class ReferenceCacheFactory(Protocol):
    def __call__(self, **kwargs: JsonValue) -> ReferenceCache: ...


class ReferenceCachingModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCacheFactory: ...


class ReferenceSwitch(Protocol):
    def __call__(self, **kwargs: JsonValue) -> None: ...


class ReferenceSwitchModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceSwitch: ...


class ReferenceStateModule(Protocol):
    def __getattr__(self, name: str) -> JsonValue: ...


class ReferenceListModule(Protocol):
    def __getattr__(self, name: str) -> list[Callback]: ...


class CacheStateModule(Protocol):
    def __getattr__(self, name: str) -> Cache | None: ...


# litellm ships its cache types untyped, so its objects come through these typed shapes.
LITELLM_CACHING: ReferenceCachingModule = importlib.import_module("litellm.caching.caching")
LITELLM_SWITCHES: ReferenceSwitchModule = importlib.import_module("litellm.caching.caching")
LITELLM_STATE: ReferenceStateModule = importlib.import_module("litellm")
LITELLM_LISTS: ReferenceListModule = importlib.import_module("litellm")
OURS_STATE: CacheStateModule = importlib.import_module("betterlite")


@pytest.fixture(autouse=True)
def _isolated_cache_state() -> Iterator[None]:
    disable_cache()
    LITELLM_SWITCHES.disable_cache()
    yield
    disable_cache()
    LITELLM_SWITCHES.disable_cache()


# The shared fake-server fixture, registered here under the name the tests ask for.
redis_server = fake_redis_server.redis_server


def _litellm_callback_lists() -> tuple[Sequence[Callback], ...]:
    return (
        LITELLM_LISTS.input_callback,
        LITELLM_LISTS.success_callback,
        LITELLM_LISTS._async_success_callback,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L200
    )


def _betterlite_callback_lists() -> tuple[Sequence[Callback], ...]:
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


def _require_memory(backend: CacheStore) -> InMemoryCache:
    match backend:
        case InMemoryCache():
            return backend
        case CacheStore():
            raise AssertionError(f"{type(backend).__name__} is not an InMemoryCache")


def _require_redis(backend: CacheStore) -> RedisCache:
    match backend:
        case RedisCache():
            return backend
        case CacheStore():
            raise AssertionError(f"{type(backend).__name__} is not a RedisCache")


def _require_disk(backend: CacheStore) -> DiskCache:
    match backend:
        case DiskCache():
            return backend
        case CacheStore():
            raise AssertionError(f"{type(backend).__name__} is not a DiskCache")


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
    assert ours.supported_call_types == list(SUPPORTED_CALLS)
    assert reference.supported_call_types == list(SUPPORTED_CALLS)
    assert ours.semantic_cache_scope == "key"
    assert reference.semantic_cache_scope == "key"
    _require_memory(ours.cache)
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS


def test_cache_init_ttl_rules() -> None:
    reference = LITELLM_CACHING.Cache(ttl=CACHE_TTL, default_in_memory_ttl=MEMORY_TTL)
    ours = Cache(ttl=CACHE_TTL, default_in_memory_ttl=MEMORY_TTL)
    assert ours.ttl == reference.ttl == MEMORY_TTL
    reference_plain = LITELLM_CACHING.Cache(ttl=PLAIN_TTL)
    ours_plain = Cache(ttl=PLAIN_TTL)
    assert ours_plain.ttl == reference_plain.ttl == PLAIN_TTL


def test_init_redis_attributes(redis_server: FakeRedisServer) -> None:
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
    backend = _require_redis(ours.cache)
    assert backend.namespace == "tenant"
    assert backend.default_ttl == REDIS_DEFAULT_TTL
    assert backend.redis_flush_size == REDIS_DEFAULT_FLUSH_SIZE


def test_cache_init_disk_directory(tmp_path: Path) -> None:
    ours = Cache(type="disk", disk_cache_dir=str(tmp_path))
    assert ours.type == "disk"
    assert ours.ttl is None
    backend = _require_disk(ours.cache)
    backend.set_cache("key", CACHED)
    assert backend.get_cache("key") == CACHED
    assert backend.get_cache("missing") is None


def test_init_disk_default(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    backend = _require_disk(Cache(type="disk").cache)
    backend.set_cache("key", CACHED)
    assert backend.get_cache("key") == CACHED
    assert (tmp_path / ".litellm_cache").is_dir()


def test_init_unsupported_type() -> None:
    for cache_type in ("bogus", LiteLLMCacheType.S3, "redis-semantic"):
        with pytest.raises(NotImplementedError, match="cache type is not built yet"):
            Cache(type=cache_type)


def test_init_cluster_raises(monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(NotImplementedError, match="cluster"):
        Cache(type="redis", host="127.0.0.1", port="6379", redis_startup_nodes=STARTUP_NODES)
    monkeypatch.setenv("REDIS_CLUSTER_NODES", '[{"host": "127.0.0.1"}]')
    with pytest.raises(NotImplementedError, match="cluster"):
        Cache(type="redis", host="127.0.0.1", port="6379")
    with pytest.raises(NotImplementedError, match="cluster"):
        RedisCache(host="127.0.0.1", port=6379, startup_nodes=STARTUP_NODES)


def test_cache_init_clock_refused() -> None:
    with pytest.raises(UnsupportedCallArgument):
        InMemoryCache(clock=time.monotonic)


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


def test_repr_hides_credentials(redis_server: FakeRedisServer) -> None:
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
    assert "hunter2" not in repr(ours.cache)
    assert "hunter2" not in repr(reference)


def test_enable_adds_cache() -> None:
    enable_cache()
    LITELLM_SWITCHES.enable_cache()
    assert OURS_STATE.cache is not None
    assert LITELLM_STATE.cache is not None
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS
    enable_cache()
    LITELLM_SWITCHES.enable_cache()
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS


def test_verbose_prints_on_enable(capsys: pytest.CaptureFixture[str]) -> None:
    enable_cache()
    assert not capsys.readouterr().out
    betterlite.set_verbose = True
    try:
        enable_cache()
    finally:
        betterlite.set_verbose = False
    assert "LiteLLM: Enabling Cache" in capsys.readouterr().out


def test_enable_keeps_cache() -> None:
    ours = Cache()
    betterlite.cache = ours
    LITELLM_SWITCHES.update_cache()
    reference = LITELLM_STATE.cache
    enable_cache(type="disk")
    LITELLM_SWITCHES.enable_cache(type="disk")
    assert OURS_STATE.cache is ours
    assert LITELLM_STATE.cache is reference


def test_update_replaces_cache() -> None:
    update_cache()
    ours_first = OURS_STATE.cache
    LITELLM_SWITCHES.update_cache()
    reference_first = LITELLM_STATE.cache
    update_cache()
    LITELLM_SWITCHES.update_cache()
    assert OURS_STATE.cache is not None
    assert OURS_STATE.cache is not ours_first
    assert LITELLM_STATE.cache is not reference_first
    assert LITELLM_STATE.cache is not None
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == CACHE_CALLBACK_COUNTS


def test_disable_removes_all() -> None:
    enable_cache()
    LITELLM_SWITCHES.enable_cache()
    disable_cache()
    LITELLM_SWITCHES.disable_cache()
    assert OURS_STATE.cache is None
    assert LITELLM_STATE.cache is None
    ours_counts, theirs_counts = _cache_callback_counts()
    assert ours_counts == theirs_counts == EMPTY_CALLBACK_COUNTS
