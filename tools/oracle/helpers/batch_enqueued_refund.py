import asyncio
import json
import logging
from collections.abc import Sequence
from typing import Final, Literal, TypedDict, cast

from litellm.proxy.hooks.batch_enqueued_tokens import (
    BATCH_ENQUEUED_REFUND_STATUSES,
    BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope,
    BatchEnqueuedTokenStore,
    canonical_provider_batch_id,
)
from litellm.proxy.utils import InternalUsageCache
from opentelemetry.trace import Span
from pydantic import BaseModel, Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

LOGGER_NAME: Final = "LiteLLM Proxy"
SCRIPT_KINDS: Final = ("reserve", "refund", "save", "pop")
JSON_VALUE: Final = TypeAdapter(JsonValue)


class Scope(BaseModel):
    key: Literal["api_key", "team"]
    value: str
    limit: int


class Record(BaseModel):
    tokens: int
    scopes: list[Scope]
    backend: Literal["redis", "memory"] = "redis"
    owner: str = ""
    reserved_at_monotonic: float


class Plan(BaseModel):
    op: Literal["post_call", "refund", "save", "pop"]
    batch_id: str = ""
    status: str = ""
    pending: Record | None = None
    reservation: Record | None = None


class ScriptSpec(BaseModel):
    refund: list[JsonValue] = Field(default_factory=list)
    save: list[JsonValue] = Field(default_factory=list)
    pop: list[JsonValue] = Field(default_factory=list)


class MemorySpec(BaseModel):
    counters: dict[str, int] = Field(default_factory=dict)
    records: dict[str, str] = Field(default_factory=dict)
    read: list[str] = Field(default_factory=list)


class Step(BaseModel):
    kind: Literal["done", "record", "tombstone", "absent", "failure"]
    record: JsonValue = None
    message: str = ""


class Input(TypedDict):
    plan: JsonValue
    redis: JsonValue
    memory: JsonValue
    now: float


class _FixedClock:
    def __init__(self, now: float) -> None:
        self._now = now

    def __call__(self) -> float:
        return self._now


def stored_value(value: JsonValue) -> JsonValue:
    match value:
        case str():
            try:
                return JSON_VALUE.validate_json(value)
            except ValidationError:
                return value
        case None | bool() | int() | float() | list() | dict():
            return value


def step_reply(step: Step) -> JsonValue:
    match step.kind:
        case "record":
            return json.dumps(step.record)
        case "tombstone":
            return ""
        case "absent" | "done":
            return None
        case "failure":
            raise RuntimeError(step.message)


class _ScriptRunner:
    def __init__(self, events: list[JsonValue], kind: str, steps: list[JsonValue]) -> None:
        self._events = events
        self._kind = kind
        self._steps = steps

    async def __call__(
        self, keys: Sequence[str], args: Sequence[str | int | float]
    ) -> JsonValue:
        key = keys[0]
        step = Step.model_validate(self._steps.pop(0))
        values = [stored_value(argument) for argument in args]
        if self._kind == "save":
            self._events.append(
                {"event": "save", "key": key, "record": values[0], "args": [values[1]]}
            )
        else:
            self._events.append({"event": self._kind, "key": key, "args": values})
        return step_reply(step)


class _FakeRedis:
    def __init__(self, events: list[JsonValue], spec: ScriptSpec) -> None:
        self._events = events
        self._queues: list[list[JsonValue]] = [[], spec.refund, spec.save, spec.pop]
        self._registered = 0

    def async_register_script(self, source: str) -> _ScriptRunner:
        del source
        kind = SCRIPT_KINDS[self._registered]
        steps = self._queues[self._registered]
        self._registered += 1
        return _ScriptRunner(self._events, kind, steps)


class _FakeInMemoryCache:
    def __init__(self, events: list[JsonValue], values: dict[str, int | str]) -> None:
        self._events = events
        self._values = values

    def delete_cache(self, key: str) -> None:
        self._events.append({"event": "local_delete", "key": key})
        self._values.pop(key, None)


class _FakeDualCache:
    def __init__(self, redis: _FakeRedis | None, memory: _FakeInMemoryCache) -> None:
        self.redis_cache = redis
        self.in_memory_cache = memory


class _FakeUsageCache:
    def __init__(
        self, events: list[JsonValue], redis: _FakeRedis | None, memory: MemorySpec
    ) -> None:
        self._events = events
        self.values: dict[str, int | str] = {**memory.counters, **memory.records}
        self.dual_cache = _FakeDualCache(redis, _FakeInMemoryCache(events, self.values))

    async def async_get_cache(
        self,
        *,
        key: str,
        litellm_parent_otel_span: Span | None,
        local_only: bool = False,
    ) -> int | str | None:
        if not local_only:
            raise RuntimeError("the enqueued-token store reads local values only")
        if litellm_parent_otel_span is not None:
            raise RuntimeError("the enqueued-token read needs no span")
        return self.values.get(key)

    async def async_set_cache(
        self,
        *,
        key: str,
        value: int | str,
        litellm_parent_otel_span: Span | None,
        local_only: bool = False,
        ttl: float | None = None,
    ) -> None:
        if not local_only:
            raise RuntimeError("the enqueued-token store writes local values only")
        if litellm_parent_otel_span is not None:
            raise RuntimeError("the enqueued-token write needs no span")
        if ttl is None:
            raise RuntimeError("the enqueued-token write needs a ttl")
        self._events.append(
            {"event": "local_set", "key": key, "value": stored_value(value), "args": [ttl]}
        )
        self.values[key] = value


class _WarningCollector(logging.Handler):
    def __init__(self, events: list[JsonValue]) -> None:
        super().__init__(level=logging.WARNING)
        self._events = events

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self._events.append({"event": "warning", "level": record.levelname.lower()})


def reservation_from(record: Record) -> BatchEnqueuedTokenReservation:
    return BatchEnqueuedTokenReservation(
        tokens=record.tokens,
        scopes=tuple(
            BatchEnqueuedTokenScope(key=scope.key, value=scope.value, limit=scope.limit)
            for scope in record.scopes
        ),
        backend=record.backend,
        owner=record.owner,
        reserved_at_monotonic=record.reserved_at_monotonic,
    )


def record_projection(reservation: BatchEnqueuedTokenReservation) -> dict[str, JsonValue]:
    return {
        "tokens": reservation.tokens,
        "scopes": [
            {"key": scope.key, "value": scope.value, "limit": scope.limit}
            for scope in reservation.scopes
        ],
        "backend": reservation.backend,
        "owner": reservation.owner,
        "reserved_at_monotonic": reservation.reserved_at_monotonic,
    }


def required_reservation(plan: Plan) -> Record:
    reservation = plan.reservation
    if reservation is None:
        raise ValueError("this plan carries no reservation")
    return reservation


async def dispatch(plan: Plan, store: BatchEnqueuedTokenStore) -> JsonValue:
    match plan.op:
        case "post_call":
            batch_id = canonical_provider_batch_id(plan.batch_id)
            pending = plan.pending
            if pending is not None:
                await store.save_reservation(
                    batch_id=batch_id, reservation=reservation_from(pending)
                )
            refunded = False
            if plan.status.lower() in BATCH_ENQUEUED_REFUND_STATUSES:
                popped = await store.pop_reservation(batch_id=batch_id)
                if popped is not None:
                    await store.refund(reservation=popped)
                    refunded = True
            return {
                "kind": "post_call",
                "batch_id": batch_id,
                "saved": pending is not None,
                "refunded": refunded,
            }
        case "refund":
            await store.refund(reservation=reservation_from(required_reservation(plan)))
            return {"kind": "refunded"}
        case "save":
            await store.save_reservation(
                batch_id=plan.batch_id, reservation=reservation_from(required_reservation(plan))
            )
            return {"kind": "saved"}
        case "pop":
            popped = await store.pop_reservation(batch_id=plan.batch_id)
            return {
                "kind": "popped",
                "reservation": None if popped is None else record_projection(popped),
            }


def run(input_value: Input) -> JsonValue:
    events: list[JsonValue] = []
    redis_raw = input_value["redis"]
    redis = (
        None if redis_raw is None else _FakeRedis(events, ScriptSpec.model_validate(redis_raw))
    )
    memory = MemorySpec.model_validate(input_value["memory"])
    fake = _FakeUsageCache(events, redis, memory)
    cache = cast(InternalUsageCache, fake)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L378
    store = BatchEnqueuedTokenStore(
        internal_usage_cache=cache, monotonic=_FixedClock(input_value["now"])
    )
    logger = logging.getLogger(LOGGER_NAME)
    collector = _WarningCollector(events)
    logger.addHandler(collector)
    try:
        outcome = asyncio.run(dispatch(Plan.model_validate(input_value["plan"]), store))
    finally:
        logger.removeHandler(collector)
    return {
        "outcome": outcome,
        "events": events,
        "memory": {key: stored_value(fake.values.get(key)) for key in memory.read},
    }
