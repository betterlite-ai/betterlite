import asyncio
from collections.abc import Mapping, Sequence
from datetime import timedelta
from types import SimpleNamespace
from typing import Self, TypeAlias, cast

from litellm.caching.redis_batch import (
    BatchResult,
    RedisBatch,
    RequestRedisBatches,
    active_post_call_redis_batch,
    active_request_redis_batch,
    active_request_redis_batches,
    drain_post_call_redis_batches,
    flush_post_call_redis_batches,
    request_redis_batch_scope,
)
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Declared: TypeAlias = BatchResult[Mapping[str, object]] | BatchResult[float]

DEFAULT_TTL: int = 60


async def noop_hook(**_: object) -> None:
    return None


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def number_value(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def reply_value(spec: Json) -> object:
    if spec == "nil":
        return None
    entry = object_value(spec, "reply")
    if "bulk" in entry:
        return string_value(entry["bulk"], "bulk")
    if "array" in entry:
        return [reply_value(item) for item in list_value(entry["array"], "array")]
    raise ValueError("unknown reply spec")


class FakePipeline:
    def __init__(self, client: "FakeClient") -> None:
        self._client = client
        self._queued: list[str] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)

    def mget(self, _keys: Sequence[str]) -> None:
        self._queued.append("MGET")

    def incrbyfloat(self, _name: str, _amount: float) -> None:
        self._queued.append("INCRBYFLOAT")

    def expire(self, _name: str, _time: timedelta) -> None:
        self._queued.append("EXPIRE")

    async def execute(self, *, raise_on_error: bool = True) -> list[object]:
        _ = raise_on_error
        return [self._client.reply(name) for name in self._queued]


class FakeClient:
    def __init__(self, replies: dict[str, list[Json]]) -> None:
        self.replies = replies

    def init_async_client(self) -> "FakeClient":
        return self

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        _ = transaction
        return FakePipeline(self)

    def reply(self, name: str) -> object:
        queue = self.replies.get(name)
        spec: Json = queue.pop(0) if queue else {"bulk": "OK"}
        return reply_value(spec)


def build_cache(spec: dict[str, Json], client: FakeClient) -> RedisCache:
    cache = object.__new__(RedisCache)
    logger = SimpleNamespace(
        async_service_success_hook=noop_hook, async_service_failure_hook=noop_hook
    )
    kwargs = {"host": string_value(spec.get("setting"), "setting"), "port": 6379}
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "namespace", spec.get("namespace"))
    object.__setattr__(cache, "default_ttl", DEFAULT_TTL)
    object.__setattr__(cache, "redis_kwargs", kwargs)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", logger)
    object.__setattr__(cache, "init_async_client", client.init_async_client)
    return cache


class BatchEntry:
    def __init__(self, batch: RedisBatch, kind: str, cache: int) -> None:
        self.batch = batch
        self.kind = kind
        self.caches: list[int] = [cache]


class Scenario:
    def __init__(self, call: dict[str, Json]) -> None:
        self.call = call
        self.deadline = integer_value(call.get("deadline_millis"), "deadline_millis") / 1000
        in_scope = call.get("in_scope")
        if not isinstance(in_scope, bool):
            raise ValueError("in_scope must be a boolean")
        self.in_scope = in_scope
        self.clients: dict[str, FakeClient] = {}
        self.caches: list[RedisCache] = []
        self.requests: list[RequestRedisBatches] = []
        self.batches: list[BatchEntry] = []
        self.handles: list[Declared | None] = []
        self.snapshots: list[Json] = []
        self.awaited: list[Json] = []
        self.nested_batches: list[Json] = []
        self.prefetch_steps: list[Json] = []

    async def drive(self) -> Json:
        self.connect_caches()
        if self.in_scope:
            with request_redis_batch_scope(post_call_deadline=self.deadline) as outer:
                self.record_request(outer)
                await self.walk()
        else:
            await self.walk()
        return self.report()

    def connect_caches(self) -> None:
        replies = object_value(self.call.get("replies") or {}, "replies")
        for spec_value in list_value(self.call.get("caches"), "caches"):
            spec = object_value(spec_value, "cache")
            setting = string_value(spec.get("setting"), "setting")
            client = self.clients.get(setting)
            if client is None:
                scripts = object_value(replies.get(setting) or {}, "replies")
                client = FakeClient(
                    {name: list_value(rows, name) for name, rows in scripts.items()}
                )
                self.clients[setting] = client
            self.caches.append(build_cache(spec, client))

    async def walk(self) -> None:
        for step in list_value(self.call.get("steps"), "steps"):
            await self.step(object_value(step, "step"))

    async def step(self, step: dict[str, Json]) -> None:
        kind = string_value(step.get("kind"), "kind")
        if self.declare_step(kind, step):
            return
        if not await self.control_step(kind, step):
            raise ValueError(f"unknown step {kind}")

    def declare_step(self, kind: str, step: dict[str, Json]) -> bool:
        if kind == "batch":
            self.declare_batch(step)
        elif kind == "post_call":
            self.declare_post_call(None, step)
        elif kind == "post_call_on":
            self.declare_post_call(integer_value(step.get("request"), "request"), step)
        else:
            return False
        return True

    async def control_step(self, kind: str, step: dict[str, Json]) -> bool:
        if kind == "await":
            await self.settle(integer_value(step.get("handle"), "handle"))
        elif kind == "flush_post_call":
            await flush_post_call_redis_batches()
        elif kind == "sleep":
            await asyncio.sleep(integer_value(step.get("millis"), "millis") / 1000)
        elif kind == "drain":
            await drain_post_call_redis_batches()
        elif kind == "nested":
            await self.nested(integer_value(step.get("cache"), "cache"))
        elif kind == "create_request":
            _ = self.record_request(RequestRedisBatches(post_call_deadline=self.deadline))
        elif kind in ("prefetch_arm", "prefetch_take"):
            self.prefetch(step)
        elif kind == "snapshot":
            self.snapshots.append(self.state())
        else:
            return False
        return True

    def declare_batch(self, step: dict[str, Json]) -> None:
        cache = integer_value(step.get("cache"), "cache")
        batch = active_request_redis_batch(self.caches[cache])
        if batch is None:
            raise ValueError("a batch step needs a request scope")
        _ = self.record_request(self.active_request())
        _ = self.record_batch(batch, "request", cache)
        keys = [string_value(key, "key") for key in list_value(step.get("keys"), "keys")]
        self.handles.append(batch.mget(keys))

    def declare_post_call(self, request: int | None, step: dict[str, Json]) -> None:
        spec = object_value(step.get("call"), "call")
        cache = integer_value(spec.get("cache"), "cache")
        if request is None:
            batch = active_post_call_redis_batch(self.caches[cache])
            if batch is None:
                raise ValueError("a post-call step needs a request scope")
            _ = self.record_request(self.active_request())
        else:
            batch = self.requests[request].post_call(self.caches[cache])
        _ = self.record_batch(batch, "post_call", cache)
        ttl = spec.get("ttl")
        handle = batch.increment(
            string_value(spec.get("key"), "key"),
            number_value(spec.get("value"), "value"),
            None if ttl is None else integer_value(ttl, "ttl"),
        )
        self.handles.append(handle)

    async def settle(self, index: int) -> None:
        handle = self.handles[index]
        if handle is None:
            raise ValueError(f"no unsettled handle {index}")
        self.handles[index] = None
        try:
            value = await handle
        except Exception:  # noqa: BLE001 - the wire contract carries every failure
            self.awaited.append({"handle": index, "ok": False})
            return
        if isinstance(value, Mapping):
            entries = cast("Mapping[object, object]", value)
            projected: Json = {
                string_value(cast(Json, key), "key"): item is not None
                for key, item in entries.items()
            }
        elif isinstance(value, float):
            projected = value
        else:
            raise ValueError(f"unexpected batch value {type(value).__name__}")
        self.awaited.append({"handle": index, "ok": True, "value": projected})

    async def nested(self, cache: int) -> None:
        cached = self.caches[cache]
        before = self.active_request()
        _ = self.record_request(before)
        before_batch = active_request_redis_batch(cached)
        if before_batch is None:
            raise ValueError("a nested step needs a request scope")
        _ = self.record_batch(before_batch, "request", cache)
        with request_redis_batch_scope(post_call_deadline=self.deadline):
            inside = self.active_request()
            inside_batch = active_request_redis_batch(cached)
            if inside_batch is None:
                raise ValueError("the nested scope has no batch")
        after = self.active_request()
        after_batch = active_request_redis_batch(cached)
        if after_batch is None:
            raise ValueError("the outer scope lost its batch")
        _ = self.record_request(inside)
        _ = self.record_request(after)
        self.nested_batches.append(
            {
                "cache": cache,
                "request_shared": inside is before and after is before,
                "batch_shared": inside_batch is before_batch
                and after_batch is before_batch,
            }
        )

    def prefetch(self, step: dict[str, Json]) -> None:
        request = self.active_request()
        slot = string_value(step.get("slot"), "slot")
        value = step.get("value")
        taken: Json = None
        action = "take"
        if value is not None:
            action = "arm"
            request.prefetched[slot] = string_value(value, "value")
        else:
            taken = cast(Json, request.prefetched.pop(slot, None))
        self.prefetch_steps.append(
            {"action": action, "slot": slot, "taken": taken, "armed": slot in request.prefetched}
        )

    def active_request(self) -> RequestRedisBatches:
        request = active_request_redis_batches()
        if request is None:
            raise ValueError("the step needs an active request_redis_batch_scope")
        return request

    def record_request(self, request: RequestRedisBatches) -> int:
        for index, held in enumerate(self.requests):
            if held is request:
                return index
        self.requests.append(request)
        return len(self.requests) - 1

    def record_batch(self, batch: RedisBatch, kind: str, cache: int) -> int:
        for index, entry in enumerate(self.batches):
            if entry.batch is batch:
                if cache not in entry.caches:
                    entry.caches.append(cache)
                return index
        self.batches.append(BatchEntry(batch, kind, cache))
        return len(self.batches) - 1

    def state(self) -> Json:
        return {
            "batches": [
                {
                    "kind": entry.kind,
                    "caches": list(entry.caches),
                    "pending": entry.batch.pending,
                    "flushes": entry.batch.flushes,
                }
                for entry in self.batches
            ],
            "requests": len(self.requests),
        }

    def report(self) -> Json:
        return {
            "state": self.state(),
            "snapshots": self.snapshots,
            "awaited": self.awaited,
            "nested": self.nested_batches,
            "prefetch": self.prefetch_steps,
        }


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(Scenario(input_value).drive())
