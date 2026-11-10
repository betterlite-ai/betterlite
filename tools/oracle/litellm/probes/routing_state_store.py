import asyncio
from decimal import Decimal
from types import SimpleNamespace
from typing import Annotated, Final, Literal, Protocol, Self, cast
from unittest.mock import patch

from litellm.caching.redis_cache import (
    RedisCache,
    RedisCircuitBreaker,
    RedisCircuitBreakerOpenError,
)
from pydantic import Field, JsonValue
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

DEFAULT_TTL: Final = 60
WORKERS: Final = 2


class Settings(Strict):
    namespace: str | None
    password: str | None
    failure_threshold: int
    recovery_timeout_millis: int


class CounterIncrement(Strict):
    counter: int


class SpendIncrement(Strict):
    spend: str


class Step(Strict):
    kind: Literal["increment", "claim", "fail", "recover", "leak"]
    now_millis: int
    worker: int = 0
    key: str | None = None
    by: CounterIncrement | SpendIncrement | None = None
    value: JsonValue | None = None
    ttl_seconds: int | None = None


class StepOutput(Strict):
    kind: str
    value: JsonValue | None
    error: str | None


class Output(Strict):
    steps: list[StepOutput]


class Cache(Protocol):
    async def async_increment(self, key: str | None, value: float, ttl: int | None = None) -> float: ...

    async def async_set_cache(
        self, key: str | None, value: JsonValue, *, ttl: int | None, nx: bool
    ) -> bool | str | bytes | None: ...

    async def async_get_cache(self, key: str | None) -> JsonValue | None: ...


class FakeStore:
    values: dict[str, str]
    ttls: dict[str, int]
    down: bool

    def __init__(self) -> None:
        self.values = {}
        self.ttls = {}
        self.down = False


def stored_ttl(store: FakeStore, name: str) -> int:
    if name not in store.values:
        return -2
    return store.ttls.get(name, -1)


def add(store: FakeStore, name: str, amount: float) -> float:
    total = Decimal(store.values.get(name, "0")) + Decimal(str(amount))
    store.values[name] = format(total, "f")
    return float(total)


class FakePipeline:
    store: FakeStore
    results: list[float | int]

    def __init__(self, store: FakeStore) -> None:
        self.store = store
        self.results = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, _kind: object, _value: object, _traceback: object) -> None:
        return None

    def incrbyfloat(self, name: str, amount: float) -> None:
        self.results.append(add(self.store, name, amount))

    def ttl(self, name: str) -> None:
        self.results.append(stored_ttl(self.store, name))

    async def execute(self) -> list[float | int]:
        return self.results


class FakeClient:
    store: FakeStore

    def __init__(self, store: FakeStore) -> None:
        self.store = store

    def check(self) -> None:
        if self.store.down:
            raise RedisConnectionError("connection down")

    async def get(self, name: str) -> bytes | None:
        self.check()
        value = self.store.values.get(name)
        return None if value is None else value.encode()

    async def set(
        self, name: str, value: str, *, nx: bool = False, ex: int | None = None
    ) -> bool | None:
        self.check()
        if nx and name in self.store.values:
            return None
        self.store.values[name] = value
        self.store.ttls[name] = -1 if ex is None else ex
        return True

    async def expire(self, name: str, seconds: int) -> bool:
        self.check()
        if name not in self.store.values:
            return False
        self.store.ttls[name] = seconds
        return True

    def pipeline(self, **_kwargs: object) -> FakePipeline:
        self.check()
        return FakePipeline(self.store)


async def noop_hook(**_kwargs: object) -> None:
    return None


def build_cache(store: FakeStore, settings: Settings) -> Cache:
    cache = object.__new__(RedisCache)
    client = FakeClient(store)
    breaker = RedisCircuitBreaker(
        failure_threshold=settings.failure_threshold,
        recovery_timeout=settings.recovery_timeout_millis // 1000,
        enabled=True,
    )
    logger = SimpleNamespace(
        async_service_success_hook=noop_hook,
        async_service_failure_hook=noop_hook,
    )
    object.__setattr__(cache, "namespace", settings.namespace)
    object.__setattr__(cache, "default_ttl", DEFAULT_TTL)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "redis_async_client", client)
    object.__setattr__(cache, "service_logger_obj", logger)
    object.__setattr__(cache, "init_async_client", lambda: client)
    return cast("Cache", cache)


def cache_at(caches: list[Cache], worker: int) -> Cache:
    if worker < 0 or worker >= len(caches):
        raise ProbeError("invalid_request", f"unknown worker {worker}")
    return caches[worker]


def canonical(value: float) -> str:
    return str(Decimal(str(value)).normalize())


def increment_amount(step: Step) -> float:
    match step.by:
        case CounterIncrement(counter=count):
            return float(count)
        case SpendIncrement(spend=text):
            return float(text)
        case None:
            raise ProbeError("invalid_request", "the increment needs an amount")


async def increment(step: Step, caches: list[Cache]) -> StepOutput:
    cache = cache_at(caches, step.worker)
    amount = increment_amount(step)
    try:
        result = await cache.async_increment(step.key, amount, ttl=None)
    except RedisCircuitBreakerOpenError:
        return StepOutput(kind=step.kind, value=None, error="circuit_open")
    except RedisConnectionError:
        return StepOutput(kind=step.kind, value=None, error="unavailable")
    if isinstance(step.by, SpendIncrement):
        rendered = canonical(result)
        return StepOutput(kind=step.kind, value=cast("JsonValue", {"Spend": rendered}), error=None)
    return StepOutput(kind=step.kind, value=cast("JsonValue", {"Counter": int(result)}), error=None)


async def claim(step: Step, caches: list[Cache]) -> StepOutput:
    cache = cache_at(caches, step.worker)
    if step.value is None:
        raise ProbeError("invalid_request", "the claim needs a value")
    try:
        stored = await cache.async_set_cache(
            step.key, step.value, ttl=step.ttl_seconds, nx=True
        )
        current = await cache.async_get_cache(step.key)
    except RedisCircuitBreakerOpenError:
        return StepOutput(kind=step.kind, value=None, error="circuit_open")
    except RedisConnectionError:
        return StepOutput(kind=step.kind, value=None, error="unavailable")
    claimed = bool(stored)
    return StepOutput(
        kind=step.kind,
        value=cast("JsonValue", {"claimed": claimed, "current": current}),
        error=None,
    )


async def leak(step: Step, caches: list[Cache], settings: Settings) -> StepOutput:
    cache = cache_at(caches, step.worker)
    try:
        _ = await cache.async_increment(step.key, 1.0, ttl=None)
    except (RedisCircuitBreakerOpenError, RedisConnectionError) as error:
        password = settings.password
        leaked = password is not None and password in str(error)
        return StepOutput(kind=step.kind, value=leaked, error=None)
    return StepOutput(kind=step.kind, value=False, error=None)


async def apply(
    step: Step, caches: list[Cache], store: FakeStore, settings: Settings
) -> StepOutput:
    match step.kind:
        case "increment":
            return await increment(step, caches)
        case "claim":
            return await claim(step, caches)
        case "fail":
            store.down = True
            return StepOutput(kind=step.kind, value=None, error=None)
        case "recover":
            store.down = False
            return StepOutput(kind=step.kind, value=None, error=None)
        case "leak":
            return await leak(step, caches, settings)


async def run_scenario(settings: Settings, steps: list[Step]) -> Output:
    store = FakeStore()
    caches = [build_cache(store, settings) for _worker in range(WORKERS)]
    outputs: list[StepOutput] = []
    for step in steps:
        with patch("time.time", return_value=step.now_millis / 1000):
            outputs.append(await apply(step, caches, store, settings))
    return Output(steps=outputs)


class RoutingStateStore(Operation):
    settings: Settings
    steps: Annotated[list[Step], Field(min_length=1)]

    @override
    def run(self) -> Output:
        return asyncio.run(run_scenario(self.settings, self.steps))
