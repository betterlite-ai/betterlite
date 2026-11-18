import asyncio
import json
import socketserver
import threading
from collections.abc import Callable, Coroutine
from functools import partial
from typing import Literal, NamedTuple, Protocol, TypeAlias, TypedDict, cast

import litellm.constants
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from pydantic import JsonValue
from redis.backoff import NoBackoff
from redis.retry import Retry
from typing_extensions import override

from b4293 import NIL, Handler, Script, bulk

Client = Literal["shared", "own", "rejecting", "none"]
HOST = "127.0.0.1"


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


Reservation: TypeAlias = tuple[list[JsonValue], dict[str, JsonValue]]
ReadPair: TypeAlias = tuple["TypedCache", list[str]]


class TypedCache(Protocol):
    def get_cache(self, key: str, *, local_only: bool = False) -> JsonValue: ...
    def set_cache(self, key: str, value: JsonValue, *, local_only: bool = False) -> None: ...
    def batch_get_cache(self, keys: list[str], *, local_only: bool = False) -> JsonValue: ...
    def reserve_redis_batch_reads(self, keys: list[str]) -> Reservation: ...
    async def async_batch_get_cache(
        self, keys: list[str], *, local_only: bool = False, throttle_redis: bool = True
    ) -> list[JsonValue] | None: ...
    async def async_get_ttl(self, key: str) -> JsonValue: ...
    @staticmethod
    async def async_batch_get_cache_shared(reads: list[ReadPair]) -> list[JsonValue]: ...


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
    shared: Script
    separate: Script

    def redis(self, client: Client) -> RedisCache | None:
        return {"own": self.own, "rejecting": self.rejecting, "shared": self.recording}.get(client)

    def cache(self, call: Fields, client: Client) -> TypedCache:
        return seed(dual(call, memory_for(call), self.redis(client)), call["memory"])

    def pair(self, call: Fields, row: CacheFields) -> TypedCache:
        memory = memory_for(call, reads=row["fail_reads"], writes=row["fail_writes"])
        return seed(dual(call, memory, self.redis(row["client"])), row["memory"])


def mget_reply(values: list[JsonValue]) -> bytes:
    parts = [NIL if value is None else bulk(json.dumps(value)) for value in values]
    return b"*%d\r\n%s" % (len(values), b"".join(parts))


# a scripted fake RESP server, or with no script one that closes every connection it accepts
def start_redis(script: Script | None) -> tuple[socketserver.ThreadingTCPServer, RedisCache]:
    factory = socketserver.StreamRequestHandler if script is None else partial(Handler, script)
    server = socketserver.ThreadingTCPServer((HOST, 0), factory)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    cache = RedisCache(host=HOST, port=port, socket_timeout=2.0, retry=Retry(NoBackoff(), 0))
    return server, cache


def sent(script: Script) -> list[JsonValue]:
    mgets = [args for args in script.received if args[0].upper() == "MGET"]
    keys: list[JsonValue] = [list(args[1:]) for args in mgets]
    return keys


def memory_for(call: Fields, *, reads: bool = False, writes: bool = False) -> InMemoryCache:
    clock = partial(float, call["clock"])
    if reads:
        return ReadFailureMemory(clock=clock)
    return WriteFailureMemory(clock=clock) if writes else InMemoryCache(clock=clock)


def dual(call: Fields, memory: InMemoryCache, redis: RedisCache | None) -> TypedCache:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L87
        TypedCache,
        DualCache(memory, redis, call["default_in_memory_ttl"]),
    )


def seed(cache: TypedCache, entries: list[Entry]) -> TypedCache:
    for entry in entries:
        cache.set_cache(entry["key"], entry["value"], local_only=True)
    return cache


def reserved(reservation: Reservation) -> dict[str, JsonValue]:
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
        "mgets": sent(fakes.shared),
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
        "mgets": sent(fakes.shared),
    }


async def sync_errors(call: Fields, fakes: Fakes) -> dict[str, JsonValue]:
    rejecting = fakes.cache(call, "rejecting")
    reads = memory_for(call, reads=True)
    writes = memory_for(call, writes=True)
    shared = fakes.redis("shared")
    facts: dict[str, JsonValue] = {}
    facts["breaker_read"] = rejecting.batch_get_cache(call["read_keys"])
    facts["breaker_due"] = reserved(rejecting.reserve_redis_batch_reads(call["due_keys"]))
    facts["memory_failure"] = dual(call, reads, shared).batch_get_cache(call["read_keys"])
    facts["write_failure"] = dual(call, writes, shared).batch_get_cache(call["write_keys"])
    facts["mgets"] = sent(fakes.shared)
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
        "mgets": sent(fakes.shared),
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
        "shared_mgets": sent(fakes.shared),
        "own_mgets": sent(fakes.separate),
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
        "mgets": sent(fakes.shared),
    }


# a broken Redis opens the real breaker after this many failed public reads
def open_breaker(call: Fields, redis: RedisCache) -> None:
    for index in range(litellm.constants.REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD):
        dual(call, InMemoryCache(), redis).batch_get_cache([f"breaker-open-{index}"])


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
    shared, own = Script(), Script()
    shared.replies["MGET"].extend(mget_reply(reply) for reply in call["replies"])
    own.replies["MGET"].extend(mget_reply(reply) for reply in call["own_replies"])
    started = [start_redis(script) for script in (shared, own, None)]
    try:
        fakes = Fakes(started[0][1], started[1][1], started[2][1], shared, own)
        open_breaker(call, fakes.rejecting)
        return asyncio.run(SCENARIOS[name](call, fakes))
    finally:
        for server, _redis in started:
            server.shutdown()
            server.server_close()
