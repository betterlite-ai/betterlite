import asyncio
import warnings
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import ClassVar, Final, Literal
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.exceptions import RateLimitError
from litellm.router_strategy import lowest_tpm_rpm_v2
from litellm.router_strategy.lowest_tpm_rpm_v2 import LowestTPMLoggingHandler_v2, PrefetchedUsage
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Millis, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true
# pyright: reportMissingTypeStubs=false, reportUnknownMemberType=false, reportUnusedCallResult=false

MILLIS_PER_SECOND: Final = 1000


class DeploymentInput(Strict):
    id: str
    model: str
    params_tpm: JsonValue | None = None
    params_rpm: JsonValue | None = None
    info_tpm: JsonValue | None = None
    info_rpm: JsonValue | None = None
    tpm: JsonValue | None = None
    rpm: JsonValue | None = None


class CounterInput(Strict):
    tpm: int | None = None
    rpm: int | None = None


class Step(Strict):
    kind: Literal["arm", "select_async", "select_sync", "down", "up"]
    now_millis: Millis = 0
    input_tokens: int = 0
    request: int = 0


class ErrorShape(Strict):
    status_code: int | None = None
    code: str | None = None
    error_type: str | None = None
    retry_after: str | None = None
    message: str


class StepOutput(Strict):
    kind: Literal["arm", "select_async", "select_sync", "down", "up"]
    armed: bool | None = None
    selected: str | None = None
    error: ErrorShape | None = None


class Output(Strict):
    steps: list[StepOutput]
    reads: list[list[str]]


class Info(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    id: str


class Returned(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_info: Info


RETURNED: Final = TypeAdapter(Returned)


class UsageRoutingV2(Operation):
    model_group: str
    deployments: list[DeploymentInput]
    counters: dict[str, CounterInput] = Field(default_factory=dict)
    local: dict[str, int] = Field(default_factory=dict)
    steps: list[Step]

    @override
    def run(self) -> Strict:
        # LiteLLM builds its response models with ReadOnly qualifiers; nothing here compares those warnings
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(run_scenario(self))


class StepClock:
    def __init__(self) -> None:
        self.millis: int = 0

    def wall(self) -> float:
        return self.millis / MILLIS_PER_SECOND

    def utc(self) -> datetime:
        return datetime.fromtimestamp(self.wall(), tz=UTC)


@dataclass
class FixedTokens:
    tokens: int = 0

    def __call__(self, **_kwargs: object) -> int:
        return self.tokens


# RedisCache's own batch read swallows a failed MGET and answers the keys it did not read, which is what a down
# server looks like to DualCache; the values here are already decoded, one counter per deployment window.
@dataclass
class FakeStore(RedisCache):
    values: dict[str, int]
    down: bool
    reads: list[list[str]]

    def find(self, key: str) -> int | None:
        prefix, _separator, _minute = key.rpartition(":")
        return self.values.get(prefix)

    def found(self, keys: list[str]) -> dict[str, object]:
        if self.down:
            return {}
        return {key: value for key in keys if (value := self.find(key)) is not None}

    @override
    def batch_get_cache(
        self,
        key_list: list[str] | list[str | None],
        parent_otel_span: object | None = None,
    ) -> dict[str, object]:
        _ = parent_otel_span
        keys = [key for key in key_list if key is not None]
        self.reads.append(keys)
        return self.found(keys)

    @override
    async def async_batch_get_cache(
        self,
        key_list: list[str] | list[str | None],
        parent_otel_span: object | None = None,
    ) -> dict[str, object]:
        _ = parent_otel_span
        return self.batch_get_cache(key_list)


@dataclass
class Context:
    group: str
    healthy: list[dict[str, JsonValue]]
    cache: DualCache
    store: FakeStore
    handler: LowestTPMLoggingHandler_v2
    tokens: FixedTokens
    prefetches: dict[int, PrefetchedUsage] = field(default_factory=dict)


def deployment_dict(deployment: DeploymentInput) -> dict[str, JsonValue]:
    param_fields: dict[str, JsonValue] = {
        "model": deployment.model,
        "tpm": deployment.params_tpm,
        "rpm": deployment.params_rpm,
    }
    info_fields: dict[str, JsonValue] = {
        "id": deployment.id,
        "tpm": deployment.info_tpm,
        "rpm": deployment.info_rpm,
    }
    entry_fields: dict[str, JsonValue] = {
        "model_info": {name: value for name, value in info_fields.items() if value is not None},
        "litellm_params": {name: value for name, value in param_fields.items() if value is not None},
        "tpm": deployment.tpm,
        "rpm": deployment.rpm,
    }
    return {name: value for name, value in entry_fields.items() if value is not None}


def store_values(operation: UsageRoutingV2) -> dict[str, int]:
    values: dict[str, int] = {}
    for deployment in operation.deployments:
        counter = operation.counters.get(deployment.id)
        if counter is None:
            continue
        for name, value in (("tpm", counter.tpm), ("rpm", counter.rpm)):
            if value is not None:
                values[f"{deployment.id}:{deployment.model}:{name}"] = value
    return values


def selected_id(picked: dict[str, JsonValue] | None) -> str | None:
    if picked is None:
        return None
    return RETURNED.validate_python(picked).model_info.id


def error_shape(error: RateLimitError) -> ErrorShape:
    try:
        retry_after: str | None = error.response.headers["retry-after"]
    except KeyError:
        retry_after = None
    return ErrorShape(
        status_code=error.status_code,
        code=error.code or "",
        error_type=error.type or "",
        retry_after=retry_after,
        message=error.message,
    )


async def arm(step: Step, context: Context) -> StepOutput:
    tpm_keys, rpm_keys = context.handler.usage_counter_keys(context.healthy)
    combined = tpm_keys + rpm_keys
    before = len(context.store.reads)
    raw: list[object | None] | None = await context.cache.async_batch_get_cache(keys=combined)
    values: Mapping[str, object] | None = (
        None
        if raw is None
        else {key: value for key, value in zip(combined, raw, strict=True) if value is not None}
    )
    context.prefetches[step.request] = PrefetchedUsage(keys=frozenset(combined), values=values)
    return StepOutput(kind="arm", armed=len(context.store.reads) > before)


async def select_async(step: Step, context: Context) -> StepOutput:
    context.tokens.tokens = step.input_tokens
    prefetched = context.prefetches.pop(step.request, None)
    try:
        with PrefetchedUsage.scoped(prefetched):
            picked: dict[str, JsonValue] | None = await context.handler.async_get_available_deployments(
                context.group, context.healthy, None, None
            )
    except RateLimitError as error:
        return StepOutput(kind="select_async", error=error_shape(error))
    return StepOutput(kind="select_async", selected=selected_id(picked))


def select_sync(step: Step, context: Context) -> StepOutput:
    context.tokens.tokens = step.input_tokens
    try:
        picked: dict[str, JsonValue] | None = context.handler.get_available_deployments(
            context.group, context.healthy, None, None
        )
    except ValueError as error:
        return StepOutput(kind="select_sync", error=ErrorShape(message=str(error)))
    if picked is None:
        raise ProbeError("invalid_request", "the sync facade returned no deployment without raising")
    return StepOutput(kind="select_sync", selected=selected_id(picked))


async def run_step(step: Step, context: Context) -> StepOutput:
    if step.kind == "arm":
        return await arm(step, context)
    if step.kind == "select_async":
        return await select_async(step, context)
    if step.kind == "select_sync":
        return select_sync(step, context)
    context.store.down = step.kind == "down"
    return StepOutput(kind=step.kind)


async def run_scenario(operation: UsageRoutingV2) -> Output:
    clock = StepClock()
    if operation.steps:
        clock.millis = operation.steps[0].now_millis
    store = FakeStore(values=store_values(operation), down=False, reads=[])
    memory = InMemoryCache(clock=clock.wall)
    cache = DualCache(in_memory_cache=memory, redis_cache=store)
    with patch("time.time", return_value=clock.wall()):
        for key, value in operation.local.items():
            memory.set_cache(key, value)
    routing_args: dict[str, JsonValue] = {}
    handler = LowestTPMLoggingHandler_v2(router_cache=cache, routing_args=routing_args)
    handler.cancel_sync_task()
    tokens = FixedTokens()
    context = Context(
        group=operation.model_group,
        healthy=[deployment_dict(deployment) for deployment in operation.deployments],
        cache=cache,
        store=store,
        handler=handler,
        tokens=tokens,
    )
    original = lowest_tpm_rpm_v2.get_utc_datetime
    lowest_tpm_rpm_v2.get_utc_datetime = clock.utc
    try:
        steps: list[StepOutput] = []
        with patch.object(lowest_tpm_rpm_v2, "token_counter", tokens):
            for step in operation.steps:
                clock.millis = step.now_millis
                with patch("time.time", return_value=clock.wall()):
                    steps.append(await run_step(step, context))
        return Output(steps=steps, reads=store.reads)
    finally:
        lowest_tpm_rpm_v2.get_utc_datetime = original
