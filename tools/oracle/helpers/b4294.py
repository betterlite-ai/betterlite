import asyncio
import json
from collections.abc import Awaitable, Callable, Sequence
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
    def batch_get_cache(
        self, keys: list[str], *, local_only: bool = False
    ) -> list[JsonValue] | None: ...
    def reserve_redis_batch_reads(
        self, keys: Sequence[str]
    ) -> tuple[list[str], dict[str, JsonValue]]: ...
    async def async_batch_get_cache(
        self, keys: list[str], *, local_only: bool = False, throttle_redis: bool = True
    ) -> list[JsonValue] | None: ...
    async def async_get_ttl(self, key: str) -> float | None: ...
    @staticmethod
    async def async_batch_get_cache_shared(
        reads: Sequence[tuple["TypedCache", list[str]]],
    ) -> list[list[JsonValue] | None]: ...


class ServiceLogger(Protocol):
    def service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    def service_failure_hook(self, **_kwargs: JsonValue) -> None: ...
    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class Logger(ServiceLogger):
    @override
    def service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    @override
    def service_failure_hook(self, **_kwargs: JsonValue) -> None: ...
    @override
    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...
    @override
    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class Mget(Protocol):
    def mget(self, keys: list[str]) -> list[bytes | None]: ...
    async def async_mget(self, keys: list[str]) -> list[bytes | None]: ...


# one scripted MGET answer per round trip, decoded by the RedisCache exactly as a live reply is
class Recorder(Mget):
    def __init__(self, replies: list[list[JsonValue]]) -> None:
        self.replies = replies
        self.keys: list[list[str]] = []

    @override
    def mget(self, keys: list[str]) -> list[bytes | None]:
        self.keys.append(keys)
        scripted = self.replies.pop(0)
        return [None if reply is None else json.dumps(reply).encode() for reply in scripted]

    @override
    async def async_mget(self, keys: list[str]) -> list[bytes | None]:
        return self.mget(keys)


class FailableMemory(InMemoryCache):
    def __init__(
        self, *, clock: Callable[[], float], fail_reads: bool, fail_writes: bool
    ) -> None:
        super().__init__(clock=clock)
        self.fail_reads = fail_reads
        self.fail_writes = fail_writes

    @override
    def batch_get_cache(self, keys: list[str], **kwargs: JsonValue) -> list[JsonValue]:
        if self.fail_reads:
            raise RuntimeError("the memory read failed")
        return super().batch_get_cache(keys, **kwargs)

    @override
    async def async_batch_get_cache(
        self, keys: list[str], **kwargs: JsonValue
    ) -> list[JsonValue]:
        if self.fail_reads:
            raise RuntimeError("the memory read failed")
        return await super().async_batch_get_cache(keys, **kwargs)

    @override
    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        if self.fail_writes:
            raise RuntimeError("the memory write failed")
        super().set_cache(key, value, **kwargs)

    @override
    async def async_set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        if self.fail_writes:
            raise RuntimeError("the memory write failed")
        await super().async_set_cache(key, value, **kwargs)


class Fakes(NamedTuple):
    recording: RedisCache
    own: RedisCache
    rejecting: RedisCache
    shared: Recorder
    separate: Recorder


def empty_redis(breaker: RedisCircuitBreaker) -> RedisCache:
    cache = object.__new__(RedisCache)
    object.__setattr__(cache, "namespace", None)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", Logger())
    return cache


def recording_redis(recorder: Recorder) -> RedisCache:
    cache = empty_redis(RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60))
    object.__setattr__(cache, "_run_redis_mget_operation", recorder.mget)
    object.__setattr__(cache, "_async_run_redis_mget_operation", recorder.async_mget)
    return cache


def rejecting_redis() -> RedisCache:
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    breaker.record_failure()
    return empty_redis(breaker)


def memory_for(call: Fields, *, fail_reads: bool, fail_writes: bool) -> InMemoryCache:
    clock = partial(float, call["clock"])
    if fail_reads or fail_writes:
        return FailableMemory(clock=clock, fail_reads=fail_reads, fail_writes=fail_writes)
    return InMemoryCache(clock=clock)


def dual(call: Fields, memory: InMemoryCache, redis: RedisCache | None) -> TypedCache:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L87
        TypedCache,
        DualCache(
            in_memory_cache=memory,
            redis_cache=redis,
            default_in_memory_ttl=call["default_in_memory_ttl"],
            default_redis_ttl=None,
            default_redis_batch_cache_expiry=None,
        ),
    )


def single(
    call: Fields, redis: RedisCache | None, *, fail_reads: bool, fail_writes: bool
) -> TypedCache:
    memory = memory_for(call, fail_reads=fail_reads, fail_writes=fail_writes)
    return dual(call, memory, redis)


def cache_pair(call: Fields, fields: CacheFields, redis: RedisCache | None) -> TypedCache:
    memory = memory_for(call, fail_reads=fields["fail_reads"], fail_writes=fields["fail_writes"])
    cache = dual(call, memory, redis)
    seed(cache, fields["memory"])
    return cache


def seed(cache: TypedCache, entries: list[Entry]) -> None:
    for entry in entries:
        cache.set_cache(entry["key"], entry["value"], local_only=True)


def pick(fakes: Fakes, client: Client) -> RedisCache | None:
    match client:
        case "own":
            return fakes.own
        case "rejecting":
            return fakes.rejecting
        case "none":
            return None
        case "shared":
            return fakes.recording


def reservation(reserved: tuple[list[str], dict[str, JsonValue]]) -> dict[str, JsonValue]:
    due, previous = reserved
    return {"due": due, "previous": previous}


async def ttl_values(cache: TypedCache, keys: list[str]) -> list[JsonValue]:
    values: list[JsonValue] = []
    for key in keys:
        values.append(await cache.async_get_ttl(key))
    return values


async def shared_read(
    caches: list[TypedCache], fields: list[CacheFields]
) -> list[list[JsonValue] | None]:
    reads = [(cache, entry["keys"]) for cache, entry in zip(caches, fields, strict=True)]
    return await caches[0].async_batch_get_cache_shared(reads)


async def reserve(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = single(call, fakes.recording, fail_reads=False, fail_writes=False)
    seed(cache, call["memory"])
    first = cache.reserve_redis_batch_reads(call["keys"])
    second = cache.reserve_redis_batch_reads(call["keys"])
    read = cache.batch_get_cache(call["read_keys"])
    after_read = cache.reserve_redis_batch_reads(call["keys"] + call["read_keys"])
    rejecting = single(call, fakes.rejecting, fail_reads=False, fail_writes=False)
    seed(rejecting, call["memory"])
    failed = rejecting.batch_get_cache(call["due_keys"])
    after_failure = rejecting.reserve_redis_batch_reads(call["due_keys"])
    absent = single(call, None, fail_reads=False, fail_writes=False)
    seed(absent, call["memory"])
    return {
        "first": reservation(first),
        "second": reservation(second),
        "read": read,
        "after_read": reservation(after_read),
        "failed": failed,
        "after_failure": reservation(after_failure),
        "no_redis": reservation(absent.reserve_redis_batch_reads(call["keys"])),
        "mgets": fakes.shared.keys,
    }


async def sync_backfill(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = single(call, fakes.recording, fail_reads=False, fail_writes=False)
    seed(cache, call["memory"])
    read = cache.batch_get_cache(call["read_keys"])
    stored = [cache.get_cache(key, local_only=True) for key in call["read_keys"]]
    ttls = await ttl_values(cache, call["ttl_keys"])
    all_hit = cache.batch_get_cache(call["keys"])
    local_only = cache.batch_get_cache(call["read_keys"], local_only=True)
    absent = single(call, None, fail_reads=False, fail_writes=False)
    seed(absent, call["memory"])
    return {
        "read": read,
        "memory": stored,
        "ttls": ttls,
        "all_hit": all_hit,
        "local_only": local_only,
        "no_redis": absent.batch_get_cache(call["read_keys"]),
        "mgets": fakes.shared.keys,
    }


async def sync_errors(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    rejecting = single(call, fakes.rejecting, fail_reads=False, fail_writes=False)
    seed(rejecting, call["memory"])
    breaker_read = rejecting.batch_get_cache(call["read_keys"])
    breaker_due = rejecting.reserve_redis_batch_reads(call["due_keys"])
    failing_read = single(call, fakes.recording, fail_reads=True, fail_writes=False)
    failing_write = single(call, fakes.recording, fail_reads=False, fail_writes=True)
    return {
        "breaker_read": breaker_read,
        "breaker_due": reservation(breaker_due),
        "memory_failure": failing_read.batch_get_cache(call["read_keys"]),
        "write_failure": failing_write.batch_get_cache(call["write_keys"]),
        "mgets": fakes.shared.keys,
    }


async def async_switch(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    cache = single(call, fakes.recording, fail_reads=False, fail_writes=False)
    seed(cache, call["memory"])
    first = await cache.async_batch_get_cache(call["keys"])
    second = await cache.async_batch_get_cache(call["keys"])
    third = await cache.async_batch_get_cache(call["keys"], throttle_redis=False)
    duplicates = await cache.async_batch_get_cache(call["duplicate_keys"], throttle_redis=False)
    merge = await cache.async_batch_get_cache(call["merge_keys"], throttle_redis=False)
    observed = call["keys"] + call["merge_keys"] + call["duplicate_keys"]
    return {
        "first": first,
        "second": second,
        "third": third,
        "duplicates": duplicates,
        "merge": merge,
        "memory": [cache.get_cache(key, local_only=True) for key in observed],
        "mgets": fakes.shared.keys,
    }


async def shared_merge(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    caches = [cache_pair(call, fields, pick(fakes, fields["client"])) for fields in call["caches"]]
    return {
        "reads": await shared_read(caches, call["caches"]),
        "memory": [
            [cache.get_cache(key, local_only=True) for key in fields["keys"]]
            for cache, fields in zip(caches, call["caches"], strict=True)
        ],
        "ttls": [
            await ttl_values(cache, fields["keys"])
            for cache, fields in zip(caches, call["caches"], strict=True)
        ],
        "shared_mgets": fakes.shared.keys,
        "own_mgets": fakes.separate.keys,
    }


async def shared_failures(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    breaker_caches = [
        cache_pair(call, fields, fakes.rejecting) for fields in call["breaker_caches"]
    ]
    breaker = await shared_read(breaker_caches, call["breaker_caches"])
    breaker_due = [
        reservation(cache.reserve_redis_batch_reads(fields["keys"]))
        for cache, fields in zip(breaker_caches, call["breaker_caches"], strict=True)
    ]
    prepare_caches = [
        cache_pair(call, fields, fakes.recording) for fields in call["prepare_caches"]
    ]
    prepare = await shared_read(prepare_caches, call["prepare_caches"])
    apply_caches = [cache_pair(call, fields, fakes.recording) for fields in call["apply_caches"]]
    apply = await shared_read(apply_caches, call["apply_caches"])
    return {
        "breaker": breaker,
        "breaker_due": breaker_due,
        "prepare": prepare,
        "apply": apply,
        "mgets": fakes.shared.keys,
    }


Scenario = Callable[[Fields, Fakes], Awaitable[dict[str, JsonValue]]]

SCENARIOS: dict[str, Scenario] = {
    "reserve_throttle_and_rollback": reserve,
    "sync_read_backfill": sync_backfill,
    "sync_read_errors": sync_errors,
    "async_read_throttle_switch": async_switch,
    "shared_read_merge": shared_merge,
    "shared_read_failures": shared_failures,
}


async def execute(name: str, call: Fields) -> dict[str, JsonValue]:
    recorder = Recorder(call["replies"])
    own = Recorder(call["own_replies"])
    fakes = Fakes(
        recording=recording_redis(recorder),
        own=recording_redis(own),
        rejecting=rejecting_redis(),
        shared=recorder,
        separate=own,
    )
    return await SCENARIOS[name](call, fakes)


def run(input_value: dict[str, Fields]) -> dict[str, JsonValue]:
    ((name, call),) = input_value.items()
    return asyncio.run(execute(name, call))
