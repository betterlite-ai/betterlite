import asyncio
import json
from collections.abc import Sequence
from datetime import timedelta
from types import TracebackType
from typing import Self, TypeAlias, cast

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_batch import active_request_redis_batch, request_redis_batch_scope
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from redis.exceptions import ConnectionError as RedisConnectionError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_TTL: int = 60
DEADLINE_SECONDS: float = 1.0

FALLBACKS: dict[str, object] = {
    "GET": None,
    "INCRBY": 1,
    "INCRBYFLOAT": 1.0,
    "TTL": DEFAULT_TTL,
    "SET": True,
    "FLUSHALL": True,
    "DEL": 1,
    "EXPIRE": 1,
}


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def number_value(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def expiry_text(expiry: object) -> str:
    if isinstance(expiry, timedelta):
        return str(int(expiry.total_seconds()))
    return str(expiry)


def fallback_reply(name: str) -> object:
    return FALLBACKS.get(name, True)


class FakeCommands:
    def __init__(self, replies: dict[str, list[object]]) -> None:
        self.replies: dict[str, list[object]] = replies
        self.seen: list[tuple[str, tuple[object, ...]]] = []

    def record(self, name: str, *args: object) -> object:
        self.seen.append((name, args))
        queue = self.replies.get(name)
        reply = queue.pop(0) if queue else fallback_reply(name)
        if isinstance(reply, Exception):
            raise reply
        return reply

    def expiries(self) -> list[Json]:
        records: list[Json] = []
        for name, args in self.seen:
            if name == "SET":
                key, *rest = args
                expiry = rest[0] if rest else None
                records.append(cast(Json, {"key": key, "ex": expiry_text(expiry)}))
        return records

    def deletions(self) -> list[int]:
        return [len(args) for name, args in self.seen if name == "DEL"]


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

    def ttl(self, name: str) -> object:
        return self.commands.record("TTL", name)

    def expire(self, name: str, time: object) -> object:
        return self.commands.record("EXPIRE", name, time)

    def delete(self, *names: str) -> object:
        return self.commands.record("DEL", *names)

    def flushall(self) -> object:
        return self.commands.record("FLUSHALL")


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
        replies: list[object] = []
        for name, args in self.queued:
            if name == "MGET":
                replies.append([None] * len(args))
            else:
                replies.append(self.commands.record(name, *args))
        return replies


class FakeAsyncRedis:
    def __init__(self, commands: FakeCommands) -> None:
        self.commands = commands

    async def get(self, name: str) -> object:
        return self.commands.record("GET", name)

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


class NoopServiceLogger:
    def service_success_hook(self, **_: object) -> None:
        return None

    def service_failure_hook(self, **_: object) -> None:
        return None

    async def async_service_success_hook(self, **_: object) -> None:
        return None

    async def async_service_failure_hook(self, **_: object) -> None:
        return None


def build_cache(commands: FakeCommands) -> RedisCache:
    cache = object.__new__(RedisCache)
    async_client = FakeAsyncRedis(commands)
    object.__setattr__(cache, "namespace", None)
    object.__setattr__(cache, "default_ttl", DEFAULT_TTL)
    object.__setattr__(cache, "redis_kwargs", {})
    object.__setattr__(cache, "redis_version", "Unknown")
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", NoopServiceLogger())
    object.__setattr__(cache, "redis_client", FakeSyncRedis(commands))
    object.__setattr__(cache, "init_async_client", lambda: async_client)
    return cache


def dual_cache(clock: float, memory_ttl: float | None, cache: RedisCache) -> DualCache:
    memory = InMemoryCache(clock=lambda: clock)
    return DualCache(
        in_memory_cache=memory,
        redis_cache=cache,
        default_in_memory_ttl=memory_ttl,
        default_redis_ttl=None,
        default_redis_batch_cache_expiry=None,
    )


async def get_hits_memory_first(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    key = string_value(call.get("key"), "key")
    commands = FakeCommands({"GET": [json.dumps(call.get("redis_get"))]})
    dual = dual_cache(clock, None, build_cache(commands))
    for entry in list_value(call.get("memory"), "memory"):
        record = object_value(entry, "memory entry")
        dual.set_cache(  # pyright: ignore[reportUnknownMemberType]  # untyped LiteLLM cache API
            string_value(record.get("key"), "key"), record.get("value"), local_only=True
        )
    read = cast(Json, dual.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"read": read})


async def get_backfills_memory_from_redis(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    memory_ttl = number_value(call.get("default_in_memory_ttl"), "default_in_memory_ttl")
    key = string_value(call.get("key"), "key")
    commands = FakeCommands({"GET": [json.dumps(call.get("redis_get"))]})
    dual = dual_cache(clock, memory_ttl, build_cache(commands))
    first = cast(Json, dual.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
    ttl = cast(Json, await dual.async_get_ttl(key))  # pyright: ignore[reportUnknownMemberType]
    second = cast(Json, dual.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"first": first, "ttl": ttl, "second": second})


async def async_get_checks_request_batch_missing(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    key = string_value(call.get("key"), "key")
    commands = FakeCommands({"GET": [json.dumps(call.get("redis_get"))]})
    cache = build_cache(commands)
    dual = dual_cache(clock, None, cache)
    read: Json = None
    with request_redis_batch_scope(post_call_deadline=DEADLINE_SECONDS):
        batch = active_request_redis_batch(cache)
        if batch is not None:
            await batch.mget([key])
            read = cast(Json, await dual.async_get_cache(key))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"read": read})


async def set_cache_applies_default_ttl(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    memory_ttl = number_value(call.get("default_in_memory_ttl"), "default_in_memory_ttl")
    key = string_value(call.get("key"), "key")
    commands = FakeCommands({})
    dual = dual_cache(clock, memory_ttl, build_cache(commands))
    dual.set_cache(key, call.get("value"))  # pyright: ignore[reportUnknownMemberType]
    memory_ttl_read = cast(Json, await dual.async_get_ttl(key))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"memory_ttl": memory_ttl_read, "sets": commands.expiries()})


async def increment_propagates_redis_error(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    key = string_value(call.get("key"), "key")
    amount = integer_value(call.get("value"), "value")
    commands = FakeCommands({"INCRBY": [RedisConnectionError("injected failure")]})
    dual = dual_cache(clock, None, build_cache(commands))
    try:
        _ = dual.increment_cache(key, amount)  # pyright: ignore[reportUnknownMemberType]
        failed = False
    except RedisConnectionError:
        failed = True
    memory = cast(Json, dual.get_cache(key, local_only=True))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"error": failed, "memory": memory})


async def async_increment_fallback_on_redis_error(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    key = string_value(call.get("key"), "key")
    amount = number_value(call.get("value"), "value")
    commands = FakeCommands({"INCRBYFLOAT": [RedisConnectionError("injected failure")]})
    dual = dual_cache(clock, None, build_cache(commands))
    result = cast(Json, await dual.async_increment_cache(key, amount))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"result": result})


async def set_pre_call_returns_batch_or_none(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    memory_ttl = number_value(call.get("default_in_memory_ttl"), "default_in_memory_ttl")
    key = string_value(call.get("key"), "key")
    commands = FakeCommands({})
    dual = dual_cache(clock, memory_ttl, build_cache(commands))
    outside = await dual.async_set_cache_pre_call(key, call.get("value"), None)  # pyright: ignore[reportUnknownMemberType]
    in_scope = False
    with request_redis_batch_scope(post_call_deadline=DEADLINE_SECONDS):
        queued = await dual.async_set_cache_pre_call(key, call.get("value"), None)  # pyright: ignore[reportUnknownMemberType]
        if queued is not None:
            in_scope = True
            await queued
    memory = cast(Json, dual.get_cache(key, local_only=True))  # pyright: ignore[reportUnknownMemberType]
    return cast(
        Json,
        {
            "outside": outside is not None,
            "in_scope": in_scope,
            "memory": memory,
            "sets": commands.expiries(),
        },
    )


async def set_pipeline_applies_single_ttl(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    memory_ttl = number_value(call.get("default_in_memory_ttl"), "default_in_memory_ttl")
    commands = FakeCommands({})
    dual = dual_cache(clock, memory_ttl, build_cache(commands))
    pairs: list[tuple[str, Json]] = []
    for entry in list_value(call.get("entries"), "entries"):
        record = object_value(entry, "entry")
        pairs.append((string_value(record.get("key"), "key"), record.get("value")))
    await dual.async_set_cache_pipeline(pairs)  # pyright: ignore[reportUnknownMemberType]
    ttls: list[Json] = []
    for key, _value in pairs:
        ttls.append(cast(Json, await dual.async_get_ttl(key)))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"ttls": ttls, "sets": commands.expiries()})


async def delete_cache_keys_chunks_redis(call: dict[str, Json]) -> Json:
    clock = number_value(call.get("clock"), "clock")
    key_count = integer_value(call.get("key_count"), "key_count")
    commands = FakeCommands({})
    dual = dual_cache(clock, None, build_cache(commands))
    keys = [f"key-{index}" for index in range(key_count)]
    for key in keys:
        dual.set_cache(key, "value", local_only=True)  # pyright: ignore[reportUnknownMemberType]
    await dual.async_delete_cache_keys(keys)  # pyright: ignore[reportUnknownMemberType]
    remaining = sum(
        1
        for key in keys
        if dual.get_cache(key, local_only=True) is not None  # pyright: ignore[reportUnknownMemberType]
    )
    return cast(Json, {"chunks": commands.deletions(), "remaining": remaining})


SCENARIOS = {
    "get_hits_memory_first": get_hits_memory_first,
    "get_backfills_memory_from_redis": get_backfills_memory_from_redis,
    "async_get_checks_request_batch_missing": async_get_checks_request_batch_missing,
    "set_cache_applies_default_ttl": set_cache_applies_default_ttl,
    "increment_propagates_redis_error": increment_propagates_redis_error,
    "async_increment_fallback_on_redis_error": async_increment_fallback_on_redis_error,
    "set_pre_call_returns_batch_or_none": set_pre_call_returns_batch_or_none,
    "set_pipeline_applies_single_ttl": set_pipeline_applies_single_ttl,
    "delete_cache_keys_chunks_redis": delete_cache_keys_chunks_redis,
}


async def scenario(input_value: dict[str, Json]) -> Json:
    if len(input_value) != 1:
        raise ValueError("the call names exactly one scenario")
    name = next(iter(input_value))
    runner = SCENARIOS.get(name)
    if runner is None:
        raise ValueError(f"unknown scenario {name}")
    return await runner(object_value(input_value[name], name))


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(scenario(input_value))
