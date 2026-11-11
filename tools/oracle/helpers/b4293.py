import asyncio
import json
from collections.abc import Callable, Coroutine, Sequence
from contextlib import AbstractAsyncContextManager
from datetime import timedelta
from typing import TypedDict, cast
from unittest.mock import AsyncMock, MagicMock

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_batch import active_request_redis_batch, request_redis_batch_scope
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from redis.exceptions import ConnectionError as RedisConnectionError

# pyright: reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false

DEFAULT_TTL: int = 60
DEADLINE_SECONDS: float = 1.0

# what the fake Redis answers once a command has no scripted reply left
FALLBACKS: dict[str, object] = {
    "GET": None,
    "INCRBY": 1,
    "INCRBYFLOAT": 1.0,
    "TTL": DEFAULT_TTL,
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
    environment: dict[str, str]


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
        return [{"key": str(args[0]), "ex": expiry_text(args[-1])} for args in sets]

    def deletions(self) -> list[int]:
        return [len(args) for name, args in self.seen if name == "DEL"]


def expiry_text(expiry: object) -> str:
    if isinstance(expiry, timedelta):
        return str(int(expiry.total_seconds()))
    return str(expiry)


class FakePipeline(AbstractAsyncContextManager["FakePipeline"]):
    def __init__(self, commands: FakeCommands) -> None:
        self.commands = commands
        self.queued: list[tuple[str, tuple[object, ...]]] = []

    async def __aexit__(self, *_exception: object) -> None:
        return None

    def __getattr__(self, command: str) -> Callable[..., None]:
        name = command.upper()
        return lambda *args, **kwargs: self.queued.append((name, (*args, *kwargs.values())))

    async def execute(self, **_options: bool) -> list[object]:
        return [
            [None] * len(cast(Sequence[str], args[0]))
            if name == "MGET"
            else self.commands.record(name, *args)
            for name, args in self.queued
        ]


def fake_sync_client(commands: FakeCommands) -> MagicMock:
    return MagicMock(
        get=MagicMock(side_effect=lambda name: commands.record("GET", name)),
        set=MagicMock(side_effect=lambda name, *_, ex=None, **__: commands.record("SET", name, ex)),
        incr=MagicMock(side_effect=lambda name, amount=1: commands.record("INCRBY", name, amount)),
    )


def fake_async_client(commands: FakeCommands) -> MagicMock:
    return MagicMock(
        set=AsyncMock(side_effect=lambda name, *_, ex=None, **__: commands.record("SET", name, ex)),
        delete=AsyncMock(side_effect=lambda *names: commands.record("DEL", *names)),
        ttl=AsyncMock(side_effect=lambda name: commands.record("TTL", name)),
        mget=AsyncMock(side_effect=lambda keys: [None for _key in keys]),
        pipeline=MagicMock(side_effect=lambda **_options: FakePipeline(commands)),
    )


def build_cache(commands: FakeCommands) -> RedisCache:
    cache = object.__new__(RedisCache)
    async_client = fake_async_client(commands)
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
    object.__setattr__(cache, "redis_client", fake_sync_client(commands))
    object.__setattr__(cache, "init_async_client", lambda: async_client)
    return cache


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
