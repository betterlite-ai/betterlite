import asyncio
import json
from collections.abc import Sequence
from types import SimpleNamespace
from typing import TYPE_CHECKING, Final, Literal, Self, cast
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_batch import (
    RequestRedisBatches,
    _active_request_batches,  # pyright: ignore[reportPrivateUsage]
)
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from litellm.router_strategy.lowest_tpm_rpm_v2 import LowestTPMLoggingHandler_v2
from litellm.router_utils.cooldown_cache import CooldownCache
from litellm.router_utils.routing_read_batch import (
    RoutingPrefetch,
)
from litellm.router_utils.routing_read_batch import (
    RoutingReadBatch as LiteLLMRoutingReadBatch,
)
from pydantic import Field, JsonValue
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

if TYPE_CHECKING:
    from litellm.router import Router

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

DEFAULT_READ_EXPIRY_MILLIS: Final = 10_000
COOLDOWN_KEY_PREFIX: Final = "deployment:"
COOLDOWN_KEY_SUFFIX: Final = ":cooldown"
BREAKER_FAILURE_THRESHOLD: Final = 100
BREAKER_RECOVERY_SECONDS: Final = 60
REQUESTS: Final = 2


class TierInput(Strict):
    keys: list[str]
    local: dict[str, JsonValue] = Field(default_factory=dict)


class UsageInput(Strict):
    tpm: list[str]
    rpm: list[str]
    local: dict[str, JsonValue] = Field(default_factory=dict)


class Step(Strict):
    kind: Literal["arm", "read", "down"]
    now_millis: int
    request: int = 0
    usage_v2: bool = False
    down: bool = False


class UsageOutput(Strict):
    covers: bool
    values: list[JsonValue | None] | None = None


class StepOutput(Strict):
    kind: Literal["arm", "read", "down"]
    armed: bool | None = None
    active: bool | None = None
    cooldown: list[JsonValue | None] | None = None
    usage: list[JsonValue | None] | None = None
    prefetched_usage: UsageOutput | None = None
    cooldown_local: list[JsonValue | None]
    usage_local: list[JsonValue | None]


class Output(Strict):
    steps: list[StepOutput]
    reads: list[list[str]]


class FakeStore:
    def __init__(self, values: dict[str, str]) -> None:
        self.values = values
        self.down = False
        self.reads: list[list[str]] = []


class FakePipeline:
    def __init__(self, store: FakeStore) -> None:
        self.store = store
        self.keys: list[str] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, _kind: object, _value: object, _traceback: object) -> None:
        return None

    def mget(self, keys: Sequence[str]) -> None:
        self.keys = list(keys)

    async def execute(self, *, raise_on_error: bool = True) -> list[list[str | None]]:
        _ = raise_on_error
        self.store.reads.append(list(self.keys))
        if self.store.down:
            raise RedisConnectionError("connection down")
        return [[self.store.values.get(key) for key in self.keys]]


class FakeClient:
    def __init__(self, store: FakeStore) -> None:
        self.store = store

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        _ = transaction
        return FakePipeline(self.store)

    async def mget(self, keys: Sequence[str]) -> list[str | None]:
        self.store.reads.append(list(keys))
        if self.store.down:
            raise RedisConnectionError("connection down")
        return [self.store.values.get(key) for key in keys]


async def noop_hook(**_kwargs: object) -> None:
    return None


def build_redis(store: FakeStore) -> RedisCache:
    cache = object.__new__(RedisCache)
    client = FakeClient(store)
    breaker = RedisCircuitBreaker(
        failure_threshold=BREAKER_FAILURE_THRESHOLD,
        recovery_timeout=BREAKER_RECOVERY_SECONDS,
        enabled=True,
    )
    logger = SimpleNamespace(
        async_service_success_hook=noop_hook,
        async_service_failure_hook=noop_hook,
    )
    object.__setattr__(cache, "namespace", None)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "redis_async_client", client)
    object.__setattr__(cache, "service_logger_obj", logger)
    object.__setattr__(cache, "init_async_client", lambda: client)
    object.__setattr__(cache, "default_ttl", 60)
    return cache


def build_dual(
    redis: RedisCache, expiry_seconds: float, backfill_ttl: float | None
) -> DualCache:
    return DualCache(
        in_memory_cache=InMemoryCache(),
        redis_cache=redis,
        default_in_memory_ttl=backfill_ttl,
        default_redis_batch_cache_expiry=expiry_seconds,
    )


def seed(cache: DualCache, local: dict[str, JsonValue]) -> None:
    for key, value in local.items():
        cache.set_cache(key, value, local_only=True)  # pyright: ignore[reportUnknownMemberType]


def memory_values(cache: DualCache, keys: Sequence[str]) -> list[JsonValue | None]:
    values = cast(
        "list[object]",
        cache.in_memory_cache.batch_get_cache(list(keys)),  # pyright: ignore[reportUnknownMemberType]
    )
    return [cast("JsonValue | None", value) for value in values]


def millis_to_seconds(millis: int) -> float:
    return millis / 1000


StepResult = tuple[
    bool | None,
    bool | None,
    list[JsonValue | None] | None,
    list[JsonValue | None] | None,
    UsageOutput | None,
]


class CooldownView:
    def __init__(self) -> None:
        self.cooldown_store: DualCache | None = None
        self.last_results: list[JsonValue | None] | None = None

    def active_cooldowns_from_results(
        self, model_ids: Sequence[str], results: Sequence[object] | None
    ) -> list[tuple[str, object]]:
        _ = model_ids
        self.last_results = (
            None if results is None else [cast("JsonValue", value) for value in results]
        )
        return []


def fake_selector(cache: DualCache) -> LowestTPMLoggingHandler_v2:
    selector = object.__new__(LowestTPMLoggingHandler_v2)
    object.__setattr__(selector, "router_cache", cache)
    return selector


def model_ids_from(keys: Sequence[str]) -> list[str]:
    identifiers: list[str] = []
    for key in keys:
        if not key.startswith(COOLDOWN_KEY_PREFIX) or not key.endswith(COOLDOWN_KEY_SUFFIX):
            raise ProbeError("invalid_request", f"{key!r} is not a cooldown cache key")
        identifiers.append(key[len(COOLDOWN_KEY_PREFIX) : len(key) - len(COOLDOWN_KEY_SUFFIX)])
    return identifiers


def validated_model_ids(keys: Sequence[str]) -> list[str]:
    identifiers = model_ids_from(keys)
    derived = [CooldownCache.get_cooldown_cache_key(model_id) for model_id in identifiers]
    if derived != list(keys):
        raise ProbeError("invalid_request", "the cooldown keys do not match their model ids")
    return identifiers


class Scenario:
    def __init__(self, spec: "RoutingReadBatch") -> None:
        self.spec = spec
        expiry = millis_to_seconds(
            spec.read_expiry_millis
            if spec.read_expiry_millis is not None
            else DEFAULT_READ_EXPIRY_MILLIS
        )
        backfill = (
            None
            if spec.backfill_ttl_millis is None
            else millis_to_seconds(spec.backfill_ttl_millis)
        )
        self.store = FakeStore(
            {key: json.dumps(value) for key, value in spec.remote.items()}
        )
        self.redis = build_redis(self.store)
        self.cooldown = build_dual(self.redis, expiry, backfill)
        self.usage = build_dual(self.redis, expiry, backfill)
        with patch("time.time", return_value=0.0):
            seed(self.cooldown, spec.cooldown.local)
            if spec.usage is not None:
                seed(self.usage, spec.usage.local)
        self.cooldown_keys = list(spec.cooldown.keys)
        self.model_ids = validated_model_ids(self.cooldown_keys)
        self.usage_keys = usage_key_lists(spec.usage)
        self.selector = fake_selector(self.usage)
        self.cooldown_view = CooldownView()
        self.cooldown_view.cooldown_store = self.cooldown
        self.router = cast(
            "Router",
            cast("object", SimpleNamespace(
                cache=SimpleNamespace(redis_cache=self.redis),
                cooldown_cache=self.cooldown_view,
                get_model_ids=lambda: self.model_ids,
            )),
        )
        self.requests = [RequestRedisBatches() for _index in range(REQUESTS)]

    async def run(self) -> Output:
        steps = [await self.run_step(step) for step in self.spec.steps]
        return Output(steps=steps, reads=self.store.reads)

    async def run_step(self, step: Step) -> StepOutput:
        if step.request >= len(self.requests):
            raise ProbeError("invalid_request", f"unknown request {step.request}")
        request = self.requests[step.request]
        with patch("time.time", return_value=millis_to_seconds(step.now_millis)):
            token = _active_request_batches.set(request)
            try:
                outcome = await self.apply_step(step)
            finally:
                _active_request_batches.reset(token)
            return StepOutput(
                kind=step.kind,
                armed=outcome[0],
                active=outcome[1],
                cooldown=outcome[2],
                usage=outcome[3],
                prefetched_usage=outcome[4],
                cooldown_local=memory_values(self.cooldown, self.cooldown_keys),
                usage_local=memory_values(
                    self.usage, self.usage_keys[0] + self.usage_keys[1]
                ),
            )

    async def apply_step(self, step: Step) -> StepResult:
        if step.kind == "arm":
            return (self.arm(step), None, None, None, None)
        if step.kind == "read":
            active, cooldown, usage, prefetched = await self.read(step)
            return (None, active, cooldown, usage, prefetched)
        self.store.down = step.down
        return (None, None, None, None, None)

    def arm(self, step: Step) -> bool:
        selector = self.selector if step.usage_v2 else None
        with patch.object(
            LowestTPMLoggingHandler_v2,
            "usage_counter_keys",
            return_value=self.usage_keys,
        ):
            RoutingPrefetch.arm(self.router, selector, [{}])  # pyright: ignore[reportUnknownMemberType]
        return RoutingPrefetch.armed()

    async def read(
        self, step: Step
    ) -> tuple[
        bool,
        list[JsonValue | None] | None,
        list[JsonValue | None] | None,
        UsageOutput | None,
    ]:
        selector = self.selector if step.usage_v2 else None
        strategy = "usage-based-routing-v2" if step.usage_v2 else "simple-shuffle"
        with patch.object(
            LowestTPMLoggingHandler_v2,
            "usage_counter_keys",
            return_value=self.usage_keys,
        ):
            batch = LiteLLMRoutingReadBatch.for_strategy(strategy, selector)
            if batch is None:
                return (False, None, None, None)
            with LiteLLMRoutingReadBatch.scoped(batch):
                _ = await batch.async_get_cooldown_deployments(self.router, [], None)  # pyright: ignore[reportUnknownMemberType]

        prefetched = batch.prefetched_usage
        if prefetched is None:
            return (True, self.cooldown_view.last_results, None, None)
        combined = self.usage_keys[0] + self.usage_keys[1]
        output = UsageOutput(
            covers=prefetched.covers(combined),
            values=cast("list[JsonValue | None] | None", prefetched.values_for(combined)),
        )
        return (True, self.cooldown_view.last_results, output.values, output)


def usage_key_lists(usage: UsageInput | None) -> tuple[list[str], list[str]]:
    if usage is None:
        return ([], [])
    return (list(usage.tpm), list(usage.rpm))


class RoutingReadBatch(Operation):
    cooldown: TierInput
    usage: UsageInput | None = None
    remote: dict[str, JsonValue] = Field(default_factory=dict)
    read_expiry_millis: int | None = None
    backfill_ttl_millis: int | None = None
    steps: list[Step]

    @override
    def run(self) -> Output:
        scenario = Scenario(self)
        return asyncio.run(scenario.run())
