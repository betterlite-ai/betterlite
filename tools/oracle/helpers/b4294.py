import asyncio
import json
from collections.abc import Awaitable, Callable
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
    async def async_batch_get_cache_shared(
        self, reads: list[tuple["TypedCache", list[str]]]
    ) -> list[JsonValue]: ...


class Logger:
    def service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    def service_failure_hook(self, **_kwargs: JsonValue) -> None: ...
    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class Recorder:
    def __init__(self, replies: list[list[JsonValue]]) -> None:
        self.replies = replies
        self.keys: list[JsonValue] = []

    def mget(self, keys: list[str]) -> list[bytes | None]:
        recorded: list[JsonValue] = [key for key in keys]
        self.keys.append(recorded)
        scripted = self.replies.pop(0)
        return [None if reply is None else json.dumps(reply).encode() for reply in scripted]

    async def async_mget(self, keys: list[str]) -> list[bytes | None]:
        return self.mget(keys)


class ReadFailureMemory(InMemoryCache):
    @override
    def batch_get_cache(self, _keys: list[str], **_kwargs: JsonValue) -> JsonValue:
        raise RuntimeError("the memory read failed")

    @override
    async def async_batch_get_cache(self, _keys: list[str], **_kwargs: JsonValue) -> JsonValue:
        raise RuntimeError("the memory read failed")


class WriteFailureMemory(InMemoryCache):
    @override
    def set_cache(self, _key: str, _value: JsonValue, **_kwargs: JsonValue) -> None:
        raise RuntimeError("the memory write failed")

    @override
    async def async_set_cache(self, _key: str, _value: JsonValue, **_kwargs: JsonValue) -> None:
        raise RuntimeError("the memory write failed")


class Fakes(NamedTuple):
    recording: RedisCache
    own: RedisCache
    rejecting: RedisCache
    shared: Recorder
    separate: Recorder


def fake_redis(breaker: RedisCircuitBreaker) -> RedisCache:
    cache = object.__new__(RedisCache)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L676-L682
    object.__setattr__(cache, "namespace", None)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", Logger())
    return cache


def recording_redis(recorder: Recorder) -> RedisCache:
    cache = fake_redis(RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60))
    object.__setattr__(cache, "_run_redis_mget_operation", recorder.mget)
    object.__setattr__(cache, "_async_run_redis_mget_operation", recorder.async_mget)
    return cache


def rejecting_redis() -> RedisCache:
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    breaker.record_failure()
    return fake_redis(breaker)


def memory_for(call: Fields, *, fail_reads: bool, fail_writes: bool) -> InMemoryCache:
    clock = partial(float, call["clock"])
    if fail_reads:
        return ReadFailureMemory(clock=clock)
    return WriteFailureMemory(clock=clock) if fail_writes else InMemoryCache(clock=clock)


def dual(call: Fields, memory: InMemoryCache, redis: RedisCache | None) -> TypedCache:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L87
        TypedCache,
        DualCache(
            in_memory_cache=memory,
            redis_cache=redis,
            default_in_memory_ttl=call["default_in_memory_ttl"],
        ),
    )


def single(
    call: Fields, redis: RedisCache | None, *, fail_reads: bool = False, fail_writes: bool = False
) -> TypedCache:
    return dual(call, memory_for(call, fail_reads=fail_reads, fail_writes=fail_writes), redis)


def cache_pair(call: Fields, fields: CacheFields, redis: RedisCache | None) -> TypedCache:
    memory = memory_for(call, fail_reads=fields["fail_reads"], fail_writes=fields["fail_writes"])
    return seed(dual(call, memory, redis), fields["memory"])


def seed(cache: TypedCache, entries: list[Entry]) -> TypedCache:
    for entry in entries:
        cache.set_cache(entry["key"], entry["value"], local_only=True)
    return cache


def pick(fakes: Fakes, client: Client) -> RedisCache | None:
    caches = {"own": fakes.own, "rejecting": fakes.rejecting, "shared": fakes.recording}
    return None if client == "none" else caches[client]


def reserved(reservation: tuple[list[JsonValue], dict[str, JsonValue]]) -> dict[str, JsonValue]:
    due, previous = reservation
    return {"due": due, "previous": previous}


async def shared_read(caches: list[TypedCache], fields: list[CacheFields]) -> list[JsonValue]:
    reads = [(cache, entry["keys"]) for cache, entry in zip(caches, fields, strict=True)]
    return await caches[0].async_batch_get_cache_shared(reads)


async def reserve(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = seed(single(call, fakes.recording), call["memory"])
    rejecting = seed(single(call, fakes.rejecting), call["memory"])
    absent = seed(single(call, None), call["memory"])
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
    cache = seed(single(call, fakes.recording), call["memory"])
    absent = seed(single(call, None), call["memory"])
    return {
        "read": cache.batch_get_cache(call["read_keys"]),
        "memory": [cache.get_cache(key, local_only=True) for key in call["read_keys"]],
        "ttls": [await cache.async_get_ttl(key) for key in call["ttl_keys"]],
        "all_hit": cache.batch_get_cache(call["keys"]),
        "local_only": cache.batch_get_cache(call["read_keys"], local_only=True),
        "no_redis": absent.batch_get_cache(call["read_keys"]),
        "mgets": fakes.shared.keys,
    }


async def sync_errors(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    rejecting = seed(single(call, fakes.rejecting), call["memory"])
    return {
        "breaker_read": rejecting.batch_get_cache(call["read_keys"]),
        "breaker_due": reserved(rejecting.reserve_redis_batch_reads(call["due_keys"])),
        "memory_failure": single(call, fakes.recording, fail_reads=True).batch_get_cache(
            call["read_keys"]
        ),
        "write_failure": single(call, fakes.recording, fail_writes=True).batch_get_cache(
            call["write_keys"]
        ),
        "mgets": fakes.shared.keys,
    }


async def async_switch(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = seed(single(call, fakes.recording), call["memory"])
    observed = call["keys"] + call["merge_keys"] + call["duplicate_keys"]
    return {
        "first": await cache.async_batch_get_cache(call["keys"]),
        "second": await cache.async_batch_get_cache(call["keys"]),
        "third": await cache.async_batch_get_cache(call["keys"], throttle_redis=False),
        "duplicates": await cache.async_batch_get_cache(
            call["duplicate_keys"], throttle_redis=False
        ),
        "merge": await cache.async_batch_get_cache(call["merge_keys"], throttle_redis=False),
        "memory": [cache.get_cache(key, local_only=True) for key in observed],
        "mgets": fakes.shared.keys,
    }


async def shared_merge(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    caches = [cache_pair(call, row, pick(fakes, row["client"])) for row in call["caches"]]
    memory: list[JsonValue] = [
        [cache.get_cache(key, local_only=True) for key in fields["keys"]]
        for cache, fields in zip(caches, call["caches"], strict=True)
    ]
    ttls: list[JsonValue] = [
        [await cache.async_get_ttl(key) for key in fields["keys"]]
        for cache, fields in zip(caches, call["caches"], strict=True)
    ]
    return {
        "reads": await shared_read(caches, call["caches"]),
        "memory": memory,
        "ttls": ttls,
        "shared_mgets": fakes.shared.keys,
        "own_mgets": fakes.separate.keys,
    }


async def shared_failures(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    breaker_caches = [cache_pair(call, row, fakes.rejecting) for row in call["breaker_caches"]]
    breaker_due: list[JsonValue] = [
        reserved(cache.reserve_redis_batch_reads(fields["keys"]))
        for cache, fields in zip(breaker_caches, call["breaker_caches"], strict=True)
    ]
    return {
        "breaker": await shared_read(breaker_caches, call["breaker_caches"]),
        "breaker_due": breaker_due,
        "prepare": await shared_read(
            [cache_pair(call, row, fakes.recording) for row in call["prepare_caches"]],
            call["prepare_caches"],
        ),
        "apply": await shared_read(
            [cache_pair(call, row, fakes.recording) for row in call["apply_caches"]],
            call["apply_caches"],
        ),
        "mgets": fakes.shared.keys,
    }


SCENARIOS: dict[str, Callable[[Fields, Fakes], Awaitable[dict[str, JsonValue]]]] = {
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
    fakes = Fakes(recording_redis(shared), recording_redis(own), rejecting_redis(), shared, own)
    return asyncio.run(SCENARIOS[name](call, fakes))
