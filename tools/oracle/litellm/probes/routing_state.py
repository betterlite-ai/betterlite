import asyncio
from collections.abc import Awaitable, Callable, Mapping
from contextlib import suppress
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
from typing import Annotated, Final, Literal, NoReturn
from unittest.mock import AsyncMock, patch

from litellm.caching.caching import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.router import Router
from litellm.router_strategy.lowest_tpm_rpm_v2 import LowestTPMLoggingHandler_v2
from litellm.router_utils.router_callbacks.track_deployment_metrics import (
    find_deployment_metadata,
    get_counted_usage_tokens,
    get_deployment_failures_for_current_minute,
    get_deployment_successes_for_current_minute,
    increment_deployment_failures_for_current_minute,
    increment_deployment_successes_for_current_minute,
)
from litellm.types.caching import RedisPipelineIncrementOperation
from pydantic import Field, JsonValue, TypeAdapter
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

from probes.protocol import Millis, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

CHANNELS: Final[TypeAdapter[dict[str, object]]] = TypeAdapter(dict[str, object])


class Step(Strict):
    kind: Literal[
        "pre_call",
        "success",
        "failure",
        "outcomes",
        "tracked",
        "sync",
        "retire",
        "down",
        "up",
        "resolve",
        "metadata",
        "redis",
    ]
    now_millis: Millis = 0
    deployment_id: str | None = None
    model: str | None = None
    repeat: int | None = None
    sync_interval_millis: Millis | None = None
    key: str | None = None
    channels: str | None = None


class ObservedRecorded(Strict):
    kind: Literal["recorded"] = "recorded"
    step: int


class ObservedKey(Strict):
    kind: Literal["key"] = "key"
    step: int
    key: str


class ObservedKeys(Strict):
    kind: Literal["keys"] = "keys"
    step: int
    keys: list[str]


class ObservedOutcomes(Strict):
    kind: Literal["outcomes"] = "outcomes"
    step: int
    successes: int
    failures: int


class ObservedMillis(Strict):
    kind: Literal["millis"] = "millis"
    step: int
    millis: int


class ObservedChannel(Strict):
    kind: Literal["channel"] = "channel"
    step: int
    channel: str | None
    tokens: int | None


class ObservedRedis(Strict):
    kind: Literal["redis"] = "redis"
    step: int
    value: str


Observation = Annotated[
    ObservedRecorded
    | ObservedKey
    | ObservedKeys
    | ObservedOutcomes
    | ObservedMillis
    | ObservedChannel
    | ObservedRedis,
    Field(discriminator="kind"),
]


class ScenarioOutput(Strict):
    steps: list[Observation]


class StepClock:
    def __init__(self) -> None:
        self.millis: int = 0

    def wall(self) -> float:
        seconds = self.millis / 1000
        return seconds

    def utc(self) -> datetime:
        moment = datetime.fromtimestamp(self.millis / 1000, tz=timezone.utc)
        return moment


class FakeStore(RedisCache):
    def __init__(self) -> None:
        self.values: dict[str, float] = {}
        self.down: bool = False

    @override
    async def async_increment_pipeline(
        self,
        increment_list: list[RedisPipelineIncrementOperation],
        **_kwargs: object,
    ) -> list[float] | None:
        if self.down:
            raise RedisConnectionError("connection down")
        results: list[float] = []
        for increment in increment_list:
            total = self.values.get(increment["key"], 0.0) + increment["increment_value"]
            self.values[increment["key"]] = total
            results.append(total)
        return results


class Recorder:
    def __init__(self) -> None:
        self.delays: list[float] = []

    def __call__(self, delay: float) -> NoReturn:
        self.delays.append(delay)
        raise asyncio.CancelledError


@dataclass
class Context:
    strategy: LowestTPMLoggingHandler_v2
    router: Router
    store: FakeStore


class RoutingState(Operation):
    env: dict[str, str] = Field(default_factory=dict)
    steps: list[Step]

    @override
    def run(self) -> Strict:
        return asyncio.run(run_scenario(self))


def required(value: str | None, what: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


async def pre_call(index: int, step: Step, context: Context) -> Observation:
    deployment_id = required(step.deployment_id, "a deployment id")
    model = required(step.model, "a model")
    deployment: dict[str, JsonValue] = {
        "model_info": {"id": deployment_id},
        "litellm_params": {"model": model},
    }
    repeat = 1 if step.repeat is None else step.repeat
    for _run in range(repeat):
        _checked = await context.strategy.async_pre_call_check(deployment, None)
    return ObservedRecorded(step=index)


def success(index: int, step: Step, context: Context) -> Observation:
    deployment_id = required(step.deployment_id, "a deployment id")
    key = increment_deployment_successes_for_current_minute(context.router, deployment_id)
    return ObservedKey(step=index, key=key)


def failure(index: int, step: Step, context: Context) -> Observation:
    deployment_id = required(step.deployment_id, "a deployment id")
    _counted = increment_deployment_failures_for_current_minute(context.router, deployment_id)
    return ObservedRecorded(step=index)


def outcomes(index: int, step: Step, context: Context) -> Observation:
    deployment_id = required(step.deployment_id, "a deployment id")
    successes = get_deployment_successes_for_current_minute(context.router, deployment_id)
    failures = get_deployment_failures_for_current_minute(context.router, deployment_id)
    return ObservedOutcomes(step=index, successes=successes, failures=failures)


def tracked(index: int, _step: Step, context: Context) -> Observation:
    keys = context.strategy.get_and_reset_in_memory_keys_to_update()
    return ObservedKeys(step=index, keys=sorted(keys))


async def sync(index: int, _step: Step, context: Context) -> Observation:
    _delay = await sleep_once(context.strategy, None)
    return ObservedRecorded(step=index)


async def sleep_once(strategy: LowestTPMLoggingHandler_v2, interval: float | None) -> float:
    recorder = Recorder()
    with patch("asyncio.sleep", AsyncMock(side_effect=recorder)), suppress(asyncio.CancelledError):
        await strategy.periodic_sync_in_memory_spend_with_redis(default_sync_interval=interval)
    return recorder.delays[0]


async def retire(index: int, _step: Step, context: Context) -> Observation:
    context.strategy.retire()
    await asyncio.sleep(0)
    return ObservedRecorded(step=index)


def switch(index: int, step: Step, context: Context) -> Observation:
    down = step.kind == "down"
    context.store.down = down
    return ObservedRecorded(step=index)


async def resolve(index: int, step: Step, context: Context) -> Observation:
    interval = None if step.sync_interval_millis is None else step.sync_interval_millis / 1000
    delay = await sleep_once(context.strategy, interval)
    return ObservedMillis(step=index, millis=round(delay * 1000))


def metadata(index: int, step: Step, _context: Context) -> Observation:
    channels = CHANNELS.validate_json(required(step.channels, "a channels object"))
    channel = channel_name(channels)
    tokens = get_counted_usage_tokens(channels)
    return ObservedChannel(step=index, channel=channel, tokens=tokens)


def channel_name(channels: Mapping[str, object]) -> str | None:
    found = find_deployment_metadata(channels)
    if found is None:
        return None
    if found is channels.get("litellm_metadata"):
        return "litellm_metadata"
    return "metadata"


async def redis(index: int, step: Step, context: Context) -> Observation:
    key = required(step.key, "a key")
    value = context.store.values.get(key, 0.0)
    return ObservedRedis(step=index, value=format(Decimal(value).normalize(), "f"))


StepHandler = Callable[[int, Step, Context], Observation]
AsyncStepHandler = Callable[[int, Step, Context], Awaitable[Observation]]
ASYNC_STEPS: Final[dict[str, AsyncStepHandler]] = {
    "pre_call": pre_call,
    "sync": sync,
    "retire": retire,
    "resolve": resolve,
    "redis": redis,
}
SYNC_STEPS: Final[dict[str, StepHandler]] = {
    "success": success,
    "failure": failure,
    "outcomes": outcomes,
    "tracked": tracked,
    "down": switch,
    "up": switch,
    "metadata": metadata,
}


async def apply_step(index: int, step: Step, context: Context) -> Observation:
    async_handler = ASYNC_STEPS.get(step.kind)
    if async_handler is not None:
        return await async_handler(index, step, context)
    return SYNC_STEPS[step.kind](index, step, context)


async def run_scenario(operation: RoutingState) -> ScenarioOutput:
    clock = StepClock()
    store = FakeStore()
    cache = DualCache(in_memory_cache=InMemoryCache(clock=clock.wall), redis_cache=store)
    strategy = LowestTPMLoggingHandler_v2(router_cache=cache, routing_args=dict[str, object]())
    strategy.cancel_sync_task()
    router = object.__new__(Router)
    object.__setattr__(router, "cache", cache)
    context = Context(strategy=strategy, router=router, store=store)
    observations: list[Observation] = []
    default = sync_default(operation.env)
    with (
        patch("litellm.router_strategy.base_routing_strategy.DEFAULT_REDIS_SYNC_INTERVAL", default),
        patch("litellm.router_strategy.lowest_tpm_rpm_v2.get_utc_datetime", clock.utc),
    ):
        for index, step in enumerate(operation.steps):
            clock.millis = step.now_millis
            observations.append(await apply_step(index, step, context))
    return ScenarioOutput(steps=observations)


def sync_default(env: dict[str, str]) -> int:
    text = env.get("DEFAULT_REDIS_SYNC_INTERVAL")
    if text is None or text == "":
        return 1
    try:
        return int(text)
    except ValueError as error:
        raise ProbeError("invalid_request", str(error)) from error
