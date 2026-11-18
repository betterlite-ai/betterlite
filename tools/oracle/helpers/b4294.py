import asyncio
import json
from collections.abc import Callable, Coroutine
from functools import partial
from typing import Literal, NamedTuple, Protocol, TypedDict, cast

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from pydantic import JsonValue
from typing_extensions import override

Client = Literal["shared", "own", "rejecting", "none"]


class Entry(TypedDict):
    key: str
    value: JsonValue


class CacheFields(TypedDict):
    memory: list[Entry]
    keys: list[str]
    client: Client
    fail_reads: bool
    fail_writes: bool


class Fields(TypedDict):
    clock: float
    default_in_memory_ttl: float | None
    memory: list[Entry]
    keys: list[str]
    read_keys: list[str]
    ttl_keys: list[str]
    due_keys: list[str]
    write_keys: list[str]
    duplicate_keys: list[str]
    merge_keys: list[str]
    replies: list[list[JsonValue]]
    own_replies: list[list[JsonValue]]
    caches: list[CacheFields]
    breaker_caches: list[CacheFields]
    prepare_caches: list[CacheFields]
    apply_caches: list[CacheFields]


class TypedCache(Protocol):
    def get_cache(self, key: str, *, local_only: bool = False) -> JsonValue: ...
    def set_cache(self, key: str, value: JsonValue, *, local_only: bool = False) -> None: ...
    def batch_get_cache(self, keys: list[str], *, local_only: bool = False) -> JsonValue: ...
    def reserve_redis_batch_reads(
        self, keys: list[str]
    ) -> tuple[list[JsonValue], dict[str, JsonValue]]: ...
    async def async_batch_get_cache(
        self, keys: list[str], *, local_only: bool = False, throttle_redis: bool = True
    ) -> list[JsonValue] | None: ...
    async def async_get_ttl(self, key: str) -> JsonValue: ...
    @staticmethod
    async def async_batch_get_cache_shared(
        reads: list[tuple["TypedCache", list[str]]],
    ) -> list[JsonValue]: ...


class Logger:
    def service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    def service_failure_hook(self, **_kwargs: JsonValue) -> None: ...
    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class Mget(Protocol):
    def mget(self, keys: list[str]) -> list[bytes | None]: ...
    async def async_mget(self, keys: list[str]) -> list[bytes | None]: ...


class Recorder(Mget):
    def __init__(self, replies: list[list[JsonValue]]) -> None:
        self.replies = replies
        self.keys: list[JsonValue] = []

    @override
    def mget(self, keys: list[str]) -> list[bytes | None]:
        recorded: list[JsonValue] = list(keys)
        self.keys.append(recorded)
        scripted = self.replies.pop(0)
        return [None if reply is None else json.dumps(reply).encode() for reply in scripted]

    @override
    async def async_mget(self, keys: list[str]) -> list[bytes | None]:
        return self.mget(keys)


class ReadFailureMemory(InMemoryCache):
    @override
    def batch_get_cache(self, keys: list[str], **_kwargs: JsonValue) -> list[JsonValue]:
        raise RuntimeError(f"the memory read failed for {keys!r}")

    @override
    async def async_batch_get_cache(self, keys: list[str], **_kwargs: JsonValue) -> list[JsonValue]:
        raise RuntimeError(f"the memory read failed for {keys!r}")


class WriteFailureMemory(InMemoryCache):
    @override
    def set_cache(self, key: str, value: JsonValue, **_kwargs: JsonValue) -> None:
        raise RuntimeError(f"the memory write failed for {key!r}={value!r}")

    @override
    async def async_set_cache(self, key: str, value: JsonValue, **_kwargs: JsonValue) -> None:
        raise RuntimeError(f"the memory write failed for {key!r}={value!r}")


class Fakes(NamedTuple):
    recording: RedisCache
    own: RedisCache
    rejecting: RedisCache
    shared: Recorder
    separate: Recorder

    def redis(self, client: Client) -> RedisCache | None:
        caches = {"own": self.own, "rejecting": self.rejecting, "shared": self.recording}
        return None if client == "none" else caches[client]

    def cache(self, call: Fields, client: Client) -> TypedCache:
        return seed(dual(call, memory_for(call), self.redis(client)), call["memory"])

    def pair(self, call: Fields, row: CacheFields) -> TypedCache:
        memory = memory_for(call, fail_reads=row["fail_reads"], fail_writes=row["fail_writes"])
        return seed(dual(call, memory, self.redis(row["client"])), row["memory"])


def fake_redis(recorder: Recorder | None) -> RedisCache:
    cache = object.__new__(RedisCache)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L676-L682
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "namespace", None)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", Logger())
    if recorder is None:
        breaker.record_failure()
    else:
        object.__setattr__(cache, "_run_redis_mget_operation", recorder.mget)
        object.__setattr__(cache, "_async_run_redis_mget_operation", recorder.async_mget)
    return cache


def memory_for(
    call: Fields, *, fail_reads: bool = False, fail_writes: bool = False
) -> InMemoryCache:
    clock = partial(float, call["clock"])
    if fail_reads:
        return ReadFailureMemory(clock=clock)
    return WriteFailureMemory(clock=clock) if fail_writes else InMemoryCache(clock=clock)


def dual(call: Fields, memory: InMemoryCache, redis: RedisCache | None) -> TypedCache:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L87
        TypedCache,
        DualCache(memory, redis, call["default_in_memory_ttl"]),
    )


def seed(cache: TypedCache, entries: list[Entry]) -> TypedCache:
    for entry in entries:
        cache.set_cache(entry["key"], entry["value"], local_only=True)
    return cache


def reserved(reservation: tuple[list[JsonValue], dict[str, JsonValue]]) -> dict[str, JsonValue]:
    due, previous = reservation
    return {"due": due, "previous": previous}


async def shared_read(caches: list[TypedCache], fields: list[CacheFields]) -> list[JsonValue]:
    reads = [(cache, entry["keys"]) for cache, entry in zip(caches, fields, strict=True)]
    return await caches[0].async_batch_get_cache_shared(reads)


async def reserve(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = fakes.cache(call, "shared")
    rejecting = fakes.cache(call, "rejecting")
    absent = fakes.cache(call, "none")
    return {
        "first": reserved(cache.reserve_redis_batch_reads(call["keys"])),
        "second": reserved(cache.reserve_redis_batch_reads(call["keys"])),
        "read": cache.batch_get_cache(call["read_keys"]),
        "after_read": reserved(cache.reserve_redis_batch_reads(call["keys"] + call["read_keys"])),
        "failed": rejecting.batch_get_cache(call["due_keys"]),
        "after_failure": reserved(rejecting.reserve_redis_batch_reads(call["due_keys"])),
        "no_redis": reserved(absent.reserve_redis_batch_reads(call["keys"])),
        "mgets": fakes.shared.keys,
    }


async def sync_backfill(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = fakes.cache(call, "shared")
    absent = fakes.cache(call, "none")
    read = cache.batch_get_cache(call["read_keys"])
    memory = [cache.get_cache(key, local_only=True) for key in call["read_keys"]]
    ttls = [await cache.async_get_ttl(key) for key in call["ttl_keys"]]
    all_hit = cache.batch_get_cache(call["keys"])
    local_only = cache.batch_get_cache(call["read_keys"], local_only=True)
    no_redis = absent.batch_get_cache(call["read_keys"])
    return {
        "read": read,
        "memory": memory,
        "ttls": ttls,
        "all_hit": all_hit,
        "local_only": local_only,
        "no_redis": no_redis,
        "mgets": fakes.shared.keys,
    }


async def sync_errors(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    rejecting = fakes.cache(call, "rejecting")
    facts: dict[str, JsonValue] = {}
    facts["breaker_read"] = rejecting.batch_get_cache(call["read_keys"])
    facts["breaker_due"] = reserved(rejecting.reserve_redis_batch_reads(call["due_keys"]))
    facts["memory_failure"] = dual(
        call, memory_for(call, fail_reads=True), fakes.redis("shared")
    ).batch_get_cache(call["read_keys"])
    facts["write_failure"] = dual(
        call, memory_for(call, fail_writes=True), fakes.redis("shared")
    ).batch_get_cache(call["write_keys"])
    facts["mgets"] = fakes.shared.keys
    return facts


async def async_switch(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = fakes.cache(call, "shared")
    first = await cache.async_batch_get_cache(call["keys"])
    second = await cache.async_batch_get_cache(call["keys"])
    third = await cache.async_batch_get_cache(call["keys"], throttle_redis=False)
    duplicates = await cache.async_batch_get_cache(call["duplicate_keys"], throttle_redis=False)
    merge = await cache.async_batch_get_cache(call["merge_keys"], throttle_redis=False)
    observed = call["keys"] + call["merge_keys"] + call["duplicate_keys"]
    memory = [cache.get_cache(key, local_only=True) for key in observed]
    return {
        "first": first,
        "second": second,
        "third": third,
        "duplicates": duplicates,
        "merge": merge,
        "memory": memory,
        "mgets": fakes.shared.keys,
    }


async def shared_merge(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    caches = [fakes.pair(call, row) for row in call["caches"]]
    return {
        # memory and ttls read after the shared read, so they show its backfill
        "reads": await shared_read(caches, call["caches"]),
        "memory": [
            [cache.get_cache(key, local_only=True) for key in fields["keys"]]
            for cache, fields in zip(caches, call["caches"], strict=True)
        ],
        "ttls": [
            [await cache.async_get_ttl(key) for key in fields["keys"]]
            for cache, fields in zip(caches, call["caches"], strict=True)
        ],
        "shared_mgets": fakes.shared.keys,
        "own_mgets": fakes.separate.keys,
    }


async def shared_failures(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    breakers = [fakes.pair(call, row) for row in call["breaker_caches"]]
    breaker = await shared_read(breakers, call["breaker_caches"])
    prepares = [fakes.pair(call, row) for row in call["prepare_caches"]]
    applies = [fakes.pair(call, row) for row in call["apply_caches"]]
    return {
        "breaker": breaker,
        # the read failed first, so breaker_due shows the reservations it rolled back
        "breaker_due": [
            reserved(cache.reserve_redis_batch_reads(fields["keys"]))
            for cache, fields in zip(breakers, call["breaker_caches"], strict=True)
        ],
        "prepare": await shared_read(prepares, call["prepare_caches"]),
        "apply": await shared_read(applies, call["apply_caches"]),
        "mgets": fakes.shared.keys,
    }


SCENARIOS: dict[str, Callable[[Fields, Fakes], Coroutine[None, None, dict[str, JsonValue]]]] = {
    "reserve_throttle_and_rollback": reserve,
    "sync_read_backfill": sync_backfill,
    "sync_read_errors": sync_errors,
    "async_read_throttle_switch": async_switch,
    "shared_read_merge": shared_merge,
    "shared_read_failures": shared_failures,
}


def run(input_value: dict[str, Fields]) -> dict[str, JsonValue]:
    ((name, call),) = input_value.items()
    shared, own = Recorder(call["replies"]), Recorder(call["own_replies"])
    fakes = Fakes(fake_redis(shared), fake_redis(own), fake_redis(None), shared, own)
    return asyncio.run(SCENARIOS[name](call, fakes))
