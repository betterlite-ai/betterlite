import asyncio
import json
from collections.abc import Callable, Coroutine, Sequence
from datetime import timedelta
from types import TracebackType
from typing import Self, TypedDict, cast
from unittest.mock import AsyncMock, MagicMock

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_batch import active_request_redis_batch, request_redis_batch_scope
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from redis.exceptions import ConnectionError as RedisConnectionError

# pyright: reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false

DEFAULT_TTL: int = 60
DEADLINE_SECONDS: float = 1.0

# what the fake Redis answers once a command has no scripted reply left
FALLBACKS: dict[str, object] = {
    "GET": None,
    "INCRBY": 1,
    "INCRBYFLOAT": 1.0,
    "TTL": DEFAULT_TTL,
    "SET": True,
    "DEL": 1,
    "EXPIRE": 1,
}


class Entry(TypedDict):
    key: str
    value: object


class Fields(TypedDict, total=False):
    clock: float
    key: str
    value: object
    redis_get: object
    memory: list[Entry]
    default_in_memory_ttl: float | None
    entries: list[Entry]
    key_count: int


class FakeCommands:
    def __init__(self, replies: dict[str, list[object]]) -> None:
        self.replies: dict[str, list[object]] = replies
        self.seen: list[tuple[str, tuple[object, ...]]] = []

    def record(self, name: str, *args: object) -> object:
        self.seen.append((name, args))
        queue = self.replies.get(name)
        reply = queue.pop(0) if queue else FALLBACKS.get(name, True)
        if isinstance(reply, Exception):
            raise reply
        return reply

    def expiries(self) -> list[dict[str, str]]:
        sets = [args for name, args in self.seen if name == "SET"]
        return [{"key": str(key), "ex": expiry_text(expiry)} for key, expiry in sets]

    def deletions(self) -> list[int]:
        return [len(args) for name, args in self.seen if name == "DEL"]


def expiry_text(expiry: object) -> str:
    if isinstance(expiry, timedelta):
        return str(int(expiry.total_seconds()))
    return str(expiry)


class FakeSyncRedis:
    def __init__(self, commands: FakeCommands) -> None:
        self.commands = commands

    def get(self, name: str) -> object:
        return self.commands.record("GET", name)

    def set(self, name: str, value: object, ex: object = None, *, nx: bool = False) -> object:
        _ = value, nx
        return self.commands.record("SET", name, ex)

    def incr(self, name: str, amount: int = 1) -> object:
        return self.commands.record("INCRBY", name, amount)


class FakePipeline:
    def __init__(self, commands: FakeCommands) -> None:
        self.commands = commands
        self.queued: list[tuple[str, tuple[object, ...]]] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = kind, value, traceback

    def mget(self, keys: Sequence[str]) -> None:
        self.queued.append(("MGET", tuple(keys)))

    def incrbyfloat(self, name: str, amount: float) -> None:
        self.queued.append(("INCRBYFLOAT", (name, amount)))

    def expire(self, name: str, time: object) -> None:
        self.queued.append(("EXPIRE", (name, time)))

    def set(self, name: str, value: object, ex: object = None) -> None:
        _ = value
        self.queued.append(("SET", (name, ex)))

    def delete(self, *names: str) -> None:
        self.queued.append(("DEL", names))

    async def execute(self, *, raise_on_error: bool = True) -> list[object]:
        _ = raise_on_error
        return [
            [None] * len(args) if name == "MGET" else self.commands.record(name, *args)
            for name, args in self.queued
        ]


class FakeAsyncRedis:
    def __init__(self, commands: FakeCommands) -> None:
        self.commands = commands

    async def set(self, name: str, value: object, ex: object = None, *, nx: bool = False) -> object:
        _ = value, nx
        return self.commands.record("SET", name, ex)

    async def delete(self, *names: str) -> object:
        return self.commands.record("DEL", *names)

    async def ttl(self, name: str) -> object:
        return self.commands.record("TTL", name)

    async def mget(self, keys: Sequence[str]) -> list[object]:
        return [None for _key in keys]

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        _ = transaction
        return FakePipeline(self.commands)


def build_cache(commands: FakeCommands) -> RedisCache:
    cache = object.__new__(RedisCache)
    async_client = FakeAsyncRedis(commands)
    object.__setattr__(cache, "namespace", None)
    object.__setattr__(cache, "default_ttl", DEFAULT_TTL)
    object.__setattr__(cache, "redis_kwargs", {})
    object.__setattr__(cache, "redis_version", "Unknown")
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    logger = MagicMock()
    logger.async_service_success_hook = AsyncMock()
    logger.async_service_failure_hook = AsyncMock()
    object.__setattr__(cache, "service_logger_obj", logger)
    object.__setattr__(cache, "redis_client", FakeSyncRedis(commands))
    object.__setattr__(cache, "init_async_client", lambda: async_client)
    return cache


# every scenario scripts the call's redis_get as the next GET reply, as the Rust helper does
def start(call: Fields, failure: str | None = None) -> tuple[DualCache, FakeCommands, RedisCache]:
    replies: dict[str, list[object]] = {"GET": [json.dumps(call.get("redis_get"))]}
    if failure is not None:
        replies[failure] = [RedisConnectionError("injected failure")]
    commands = FakeCommands(replies)
    cache = build_cache(commands)
    clock = call.get("clock", 0.0)
    dual = DualCache(
        in_memory_cache=InMemoryCache(clock=lambda: clock),
        redis_cache=cache,
        default_in_memory_ttl=call.get("default_in_memory_ttl"),
        default_redis_ttl=None,
        default_redis_batch_cache_expiry=None,
    )
    return dual, commands, cache


# the ttl and a second read show what the first read left in memory
async def observe(dual: DualCache, key: str, read: object) -> dict[str, object]:
    ttl = await dual.async_get_ttl(key)
    return {"read": read, "ttl": ttl, "second": dual.get_cache(key)}


async def read(call: Fields) -> dict[str, object]:
    dual, _commands, _cache = start(call)
    for entry in call.get("memory", []):
        dual.set_cache(entry["key"], entry["value"], local_only=True)
    key = call.get("key", "")
    return await observe(dual, key, dual.get_cache(key))


async def batch_read(call: Fields) -> dict[str, object]:
    dual, _commands, cache = start(call)
    key = call.get("key", "")
    found: object = None
    with request_redis_batch_scope(post_call_deadline=DEADLINE_SECONDS):
        batch = active_request_redis_batch(cache)
        if batch is not None:
            await batch.mget([key])
            found = await dual.async_get_cache(key)
    return await observe(dual, key, found)


async def set_default_ttl(call: Fields) -> dict[str, object]:
    dual, commands, _cache = start(call)
    key = call.get("key", "")
    dual.set_cache(key, call.get("value"))
    return {"memory_ttl": await dual.async_get_ttl(key), "sets": commands.expiries()}


async def increment_error(call: Fields) -> dict[str, object]:
    dual, _commands, _cache = start(call, "INCRBY")
    key = call.get("key", "")
    try:
        dual.increment_cache(key, cast(int, call.get("value")))
        failed = False
    except RedisConnectionError:
        failed = True
    return {"error": failed, "memory": dual.get_cache(key, local_only=True)}


async def increment_fallback(call: Fields) -> dict[str, object]:
    dual, _commands, _cache = start(call, "INCRBYFLOAT")
    amount = cast(float, call.get("value"))
    return {"result": await dual.async_increment_cache(call.get("key", ""), amount)}


async def pre_call(call: Fields) -> dict[str, object]:
    dual, commands, _cache = start(call)
    key, value = call.get("key", ""), call.get("value")
    outside = await dual.async_set_cache_pre_call(key, value, None)
    in_scope = False
    with request_redis_batch_scope(post_call_deadline=DEADLINE_SECONDS):
        queued = await dual.async_set_cache_pre_call(key, value, None)
        if queued is not None:
            in_scope = True
            await queued
    return {
        "outside": outside is not None,
        "in_scope": in_scope,
        "memory": dual.get_cache(key, local_only=True),
        "sets": commands.expiries(),
    }


async def pipeline(call: Fields) -> dict[str, object]:
    dual, commands, _cache = start(call)
    pairs = [(entry["key"], entry["value"]) for entry in call.get("entries", [])]
    await dual.async_set_cache_pipeline(pairs)
    ttls = [await dual.async_get_ttl(key) for key, _value in pairs]
    return {"ttls": ttls, "sets": commands.expiries()}


async def delete_keys(call: Fields) -> dict[str, object]:
    dual, commands, _cache = start(call)
    keys = [f"key-{index}" for index in range(call.get("key_count", 0))]
    for key in keys:
        dual.set_cache(key, "value", local_only=True)
    await dual.async_delete_cache_keys(keys)
    remaining = sum(1 for key in keys if dual.get_cache(key, local_only=True) is not None)
    return {"chunks": commands.deletions(), "remaining": remaining}


SCENARIOS: dict[str, Callable[[Fields], Coroutine[None, None, dict[str, object]]]] = {
    "get_hits_memory_first": read,
    "get_backfills_memory_from_redis": read,
    "async_get_checks_request_batch_missing": batch_read,
    "set_cache_applies_default_ttl": set_default_ttl,
    "increment_propagates_redis_error": increment_error,
    "async_increment_fallback_on_redis_error": increment_fallback,
    "set_pre_call_returns_batch_or_none": pre_call,
    "set_pipeline_applies_single_ttl": pipeline,
    "delete_cache_keys_chunks_redis": delete_keys,
}


def run(input_value: dict[str, Fields]) -> dict[str, object]:
    ((name, call),) = input_value.items()
    return asyncio.run(SCENARIOS[name](call))
