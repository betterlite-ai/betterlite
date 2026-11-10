import asyncio
from collections.abc import Mapping, Sequence
from datetime import timedelta
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
REPORTED: frozenset[str] = frozenset(
    {"DEL", "EVALSHA", "EXPIRE", "INCRBYFLOAT", "MGET", "SCRIPT", "SET"}
)


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


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return integer_value(value, name)


def number_value(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def reply_value(spec: Json) -> object:
    if spec == "nil":
        return None
    entry = object_value(spec, "reply")
    if "bulk" in entry:
        return string_value(entry["bulk"], "bulk")
    if "simple" in entry:
        return string_value(entry["simple"], "simple")
    if "int" in entry:
        return integer_value(entry["int"], "int")
    if "array" in entry:
        return [reply_value(item) for item in list_value(entry["array"], "array")]
    raise ValueError("unknown reply spec")


class FakePipeline:
    def __init__(self, client: "FakeClient") -> None:
        self._client = client
        self._queued: list[tuple[str, list[str]]] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)

    def mget(self, keys: Sequence[str]) -> None:
        self._queued.append(("MGET", list(keys)))

    def incrbyfloat(self, name: str, amount: float) -> None:
        self._queued.append(("INCRBYFLOAT", [name, str(amount)]))

    def expire(self, name: str, time: timedelta) -> None:
        self._queued.append(("EXPIRE", [name, str(int(time.total_seconds()))]))

    async def execute(self, *, raise_on_error: bool = True) -> list[object]:
        _ = raise_on_error
        replies: list[object] = []
        for name, _args in self._queued:
            self._client.commands.append(name)
            replies.append(self._client.reply(name))
        self._queued.clear()
        return replies


class FakeClient:
    def __init__(self, replies: dict[str, list[Json]]) -> None:
        self.replies = replies
        self.commands: list[str] = []

    def init_async_client(self) -> "FakeClient":
        return self

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        _ = transaction
        return FakePipeline(self)

    def reply(self, name: str) -> object:
        queue = self.replies.get(name)
        spec: Json = queue.pop(0) if queue else {"simple": "OK"}
        return reply_value(spec)


class Logger:
    async def async_service_success_hook(self, **_: object) -> None:
        return None

    async def async_service_failure_hook(self, **_: object) -> None:
        return None

    def service_success_hook(self, **_: object) -> None:
        return None

    def service_failure_hook(self, **_: object) -> None:
        return None


def build_cache(spec: dict[str, Json], client: FakeClient) -> RedisCache:
    cache = object.__new__(RedisCache)
    fields: dict[str, object] = {
        "namespace": optional_string(spec.get("namespace"), "namespace"),
        "default_ttl": DEFAULT_TTL,
        "redis_kwargs": {
            "host": string_value(spec.get("setting"), "setting"),
            "port": 6379,
        },
        "_circuit_breaker": RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60),
        "service_logger_obj": Logger(),
        "init_async_client": client.init_async_client,
    }
    for name, value in fields.items():
        object.__setattr__(cache, name, value)
    return cache


class BatchEntry:
    def __init__(self, batch: RedisBatch, kind: str, cache: int) -> None:
        self.batch = batch
        self.kind = kind
        self.caches: list[int] = [cache]


def await_value(value: object) -> Json:
    if isinstance(value, Mapping):
        entries = cast("Mapping[object, object]", value)
        return {
            string_value(cast(Json, key), "key"): item is not None
            for key, item in entries.items()
        }
    if isinstance(value, float):
        return value
    raise ValueError(f"unexpected batch value {type(value).__name__}")


class Scenario:
    def __init__(self, call: dict[str, Json]) -> None:
        self.call = call
        self.deadline = integer_value(call.get("deadline_millis"), "deadline_millis") / 1000
        self.in_scope = boolean_value(call.get("in_scope"), "in_scope")
        self.clients: dict[str, FakeClient] = {}
        self.caches: list[RedisCache] = []
        self.requests: list[RequestRedisBatches] = []
        self.batches: list[BatchEntry] = []
        self.handles: list[Declared | None] = []
        self.steps: list[Json] = []

    def run(self) -> Json:
        return asyncio.run(self.drive())

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
        if await self.runtime_step(kind, step):
            return
        if await self.request_step(kind, step):
            return
        raise ValueError(f"unknown step {kind}")

    def declare_step(self, kind: str, step: dict[str, Json]) -> bool:
        if kind == "batch":
            self.declare_batch(step)
            return True
        if kind == "post_call":
            self.declare_post_call(step)
            return True
        if kind == "post_call_on":
            self.declare_post_call_on(step)
            return True
        return False

    async def runtime_step(self, kind: str, step: dict[str, Json]) -> bool:
        if kind == "await":
            await self.settle(integer_value(step.get("handle"), "handle"))
            return True
        if kind == "flush_post_call":
            await flush_post_call_redis_batches()
            self.steps.append({"kind": "flush_post_call"})
            return True
        if kind == "flush_all":
            request = self.active_request()
            await request.flush_all()
            self.steps.append({"kind": "flush_all"})
            return True
        if kind == "sleep":
            await asyncio.sleep(integer_value(step.get("millis"), "millis") / 1000)
            self.steps.append({"kind": "sleep"})
            return True
        if kind == "drain":
            await drain_post_call_redis_batches()
            self.steps.append({"kind": "drain"})
            return True
        return False

    async def request_step(self, kind: str, step: dict[str, Json]) -> bool:
        if kind == "nested":
            await self.nested(integer_value(step.get("cache"), "cache"))
            return True
        if kind == "create_request":
            request = RequestRedisBatches(post_call_deadline=self.deadline)
            self.steps.append(
                {"kind": "create_request", "request": self.record_request(request)}
            )
            return True
        if kind in ("prefetch_arm", "prefetch_take", "prefetch_armed"):
            self.prefetch_step(kind, step)
            return True
        if kind == "snapshot":
            self.steps.append(self.snapshot())
            return True
        return False

    def prefetch_step(self, kind: str, step: dict[str, Json]) -> None:
        if kind == "prefetch_arm":
            self.prefetch_arm(step)
        elif kind == "prefetch_take":
            self.prefetch_take(step)
        else:
            self.prefetch_armed(step)

    def declare_batch(self, step: dict[str, Json]) -> None:
        cached = self.caches[integer_value(step.get("cache"), "cache")]
        self.touch_active_request()
        batch = active_request_redis_batch(cached)
        if batch is None:
            raise ValueError("a batch step needs a request scope")
        index = self.record_batch(batch, "request", integer_value(step.get("cache"), "cache"))
        keys = [string_value(key, "key") for key in list_value(step.get("keys"), "keys")]
        self.handles.append(batch.mget(keys))
        self.steps.append(
            {
                "kind": "batch",
                "cache": integer_value(step.get("cache"), "cache"),
                "batch": index,
            }
        )

    def declare_post_call(self, step: dict[str, Json]) -> None:
        spec = object_value(step.get("call"), "call")
        cached = self.caches[integer_value(spec.get("cache"), "cache")]
        self.touch_active_request()
        batch = active_post_call_redis_batch(cached)
        if batch is None:
            raise ValueError("a post-call step needs a request scope")
        self.push_post_call(batch, spec)

    def declare_post_call_on(self, step: dict[str, Json]) -> None:
        spec = object_value(step.get("call"), "call")
        request = self.requests[integer_value(step.get("request"), "request")]
        cached = self.caches[integer_value(spec.get("cache"), "cache")]
        self.push_post_call(request.post_call(cached), spec)

    def push_post_call(self, batch: RedisBatch, spec: dict[str, Json]) -> None:
        cache = integer_value(spec.get("cache"), "cache")
        index = self.record_batch(batch, "post_call", cache)
        handle = batch.increment(
            string_value(spec.get("key"), "key"),
            number_value(spec.get("value"), "value"),
            optional_integer(spec.get("ttl"), "ttl"),
        )
        self.handles.append(handle)
        self.steps.append({"kind": "post_call", "cache": cache, "batch": index})

    async def settle(self, index: int) -> None:
        handle = self.handles[index]
        if handle is None:
            raise ValueError(f"no unsettled handle {index}")
        self.handles[index] = None
        try:
            value = await handle
        except Exception:  # noqa: BLE001 - the wire contract carries every failure
            self.steps.append({"kind": "await", "handle": index, "ok": False})
            return
        self.steps.append(
            {
                "kind": "await",
                "handle": index,
                "ok": True,
                "value": await_value(value),
            }
        )

    async def nested(self, cache: int) -> None:
        cached = self.caches[cache]
        outer = self.active_request()
        request_before = self.record_request(outer)
        outer_batch = active_request_redis_batch(cached)
        if outer_batch is None:
            raise ValueError("a nested step needs a request scope")
        batch_before = self.record_batch(outer_batch, "request", cache)
        with request_redis_batch_scope(post_call_deadline=self.deadline):
            inside_request = active_request_redis_batches()
            if inside_request is None:
                raise ValueError("the nested scope has no request")
            inside_batch = active_request_redis_batch(cached)
            if inside_batch is None:
                raise ValueError("the nested scope has no batch")
            request_inside = self.record_request(inside_request)
            batch_inside = self.record_batch(inside_batch, "request", cache)
        after = self.active_request()
        request_after = self.record_request(after)
        after_batch = active_request_redis_batch(cached)
        if after_batch is None:
            raise ValueError("the outer scope lost its batch")
        batch_after = self.record_batch(after_batch, "request", cache)
        self.steps.append(
            {
                "kind": "nested",
                "cache": cache,
                "request_before": request_before,
                "batch_before": batch_before,
                "request_inside": request_inside,
                "batch_inside": batch_inside,
                "request_after": request_after,
                "batch_after": batch_after,
            }
        )

    def prefetch_arm(self, step: dict[str, Json]) -> None:
        request = self.active_request()
        slot = string_value(step.get("slot"), "slot")
        request.prefetched[slot] = string_value(step.get("value"), "value")
        self.steps.append(
            {"kind": "prefetch_arm", "slot": slot, "armed": slot in request.prefetched}
        )

    def prefetch_take(self, step: dict[str, Json]) -> None:
        request = self.active_request()
        slot = string_value(step.get("slot"), "slot")
        taken = cast(Json, request.prefetched.pop(slot, None))
        self.steps.append(
            {
                "kind": "prefetch_take",
                "slot": slot,
                "taken": taken,
                "armed": slot in request.prefetched,
            }
        )

    def prefetch_armed(self, step: dict[str, Json]) -> None:
        request = self.active_request()
        slot = string_value(step.get("slot"), "slot")
        self.steps.append(
            {"kind": "prefetch_armed", "slot": slot, "armed": slot in request.prefetched}
        )

    def touch_active_request(self) -> None:
        self.record_request(self.active_request())

    def active_request(self) -> RequestRedisBatches:
        request = active_request_redis_batches()
        if request is None:
            raise ValueError("the step needs a request scope")
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

    def batches_json(self) -> list[Json]:
        return [
            {
                "kind": entry.kind,
                "caches": list(entry.caches),
                "pending": entry.batch.pending,
                "flushes": entry.batch.flushes,
            }
            for entry in self.batches
        ]

    def commands_json(self) -> dict[str, Json]:
        return {
            setting: [name for name in client.commands if name in REPORTED]
            for setting, client in self.clients.items()
        }

    def snapshot(self) -> Json:
        return {
            "batches": self.batches_json(),
            "requests": len(self.requests),
            "commands": self.commands_json(),
        }

    def report(self) -> Json:
        return {
            "steps": self.steps,
            "batches": self.batches_json(),
            "requests": len(self.requests),
            "commands": self.commands_json(),
        }


def run(input_value: dict[str, Json]) -> Json:
    return Scenario(input_value).run()
