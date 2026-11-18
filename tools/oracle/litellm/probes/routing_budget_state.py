import asyncio
import warnings
from collections.abc import Awaitable, Callable
from contextlib import suppress
from dataclasses import dataclass
from decimal import Decimal
from typing import Annotated, Final, Literal, Protocol, TypeVar, overload

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.router import Router
from litellm.types.caching import RedisPipelineIncrementOperation
from litellm.types.utils import BudgetConfig
from opentelemetry.trace import Span
from pydantic import Field, JsonValue, TypeAdapter
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MONEY: Final = TypeAdapter(float | None)
DEFAULT_MODEL: Final = "openai/gpt-4o-mini"
MODEL_GROUP: Final = "gpt"
SleepResult = TypeVar("SleepResult")


class Deployment(Strict):
    model_id: str
    max_budget: str | None = None
    budget_duration: str | None = None


class Provider(Strict):
    budget_limit: str | None = None
    time_period: str | None = None


class Tag(Strict):
    max_budget: str | None = None
    budget_duration: str | None = None


class Step(Strict):
    kind: Literal[
        "success",
        "sync",
        "begin_sync",
        "end_sync",
        "shutdown",
        "memory",
        "redis",
        "fail_writes",
        "succeed_writes",
        "hold_writes",
        "release_writes",
    ]
    call_type: str | None = None
    provider: str | None = None
    model_id: str | None = None
    tags: list[str] | None = None
    cost: str | None = None
    key: str | None = None


class Observation(Strict):
    kind: str
    step: int
    value: str | None = None


class Output(Strict):
    steps: list[Observation]


class MemoryCache(Protocol):
    async def async_get_cache(self, key: str) -> float | None: ...


class Limiter(Protocol):
    dual_cache: DualCache
    tag_budget_config: dict[str, BudgetConfig] | None

    async def async_log_success_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: JsonValue,
        end_time: JsonValue,
    ) -> None: ...

    async def periodic_sync_in_memory_spend_with_redis(self) -> None: ...


class SleepInterruptedError(BaseException):
    def __init__(self, delay: float) -> None:
        super().__init__(delay)
        self.delay: float = delay


# asyncio.sleep's two overloads, so the fake replaces it: the second carries any result through to the caller
@overload
async def recorded_sleep(delay: float) -> None: ...


@overload
async def recorded_sleep(delay: float, result: SleepResult) -> SleepResult: ...


async def recorded_sleep(
    delay: float, result: SleepResult | None = None
) -> SleepResult | None:
    _ = result
    raise SleepInterruptedError(delay)


# RedisCache's constructor needs a host or url and connects; the dataclass sets only the state the pipeline reads
@dataclass
class FakeStore(RedisCache):
    values: dict[str, float]
    fail: bool
    hold: bool
    entered: asyncio.Event
    release: asyncio.Event
    read_done: asyncio.Event

    @override
    async def async_increment_pipeline(
        self,
        increment_list: list[RedisPipelineIncrementOperation],
        **_kwargs: JsonValue,
    ) -> list[float] | None:
        if self.hold:
            self.entered.set()
            _ = await self.release.wait()
        if self.fail:
            raise RedisConnectionError("connection down")
        totals: list[float] = []
        for increment in increment_list:
            total = self.values.get(increment["key"], 0.0) + increment["increment_value"]
            self.values[increment["key"]] = total
            totals.append(total)
        return totals

    @override
    async def async_batch_get_cache(
        self,
        key_list: list[str] | list[str | None],
        parent_otel_span: Span | None = None,
    ) -> dict[str, float | None]:
        _ = parent_otel_span
        self.read_done.set()
        return {key: self.values.get(key) for key in key_list if key is not None}

    # the start-time write-through reaches this method; no step reads it from the store, so it stops at the memory tier
    @override
    async def async_set_cache(
        self, key: str, value: JsonValue, **_kwargs: JsonValue
    ) -> None:
        _ = key, value

    @override
    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: Span | None = None,
        **_kwargs: JsonValue,
    ) -> float | None:
        _ = key, parent_otel_span
        return None


def make_store() -> FakeStore:
    return FakeStore(
        values={},
        fail=False,
        hold=False,
        entered=asyncio.Event(),
        release=asyncio.Event(),
        read_done=asyncio.Event(),
    )


@dataclass
class Context:
    limiter: Limiter
    store: FakeStore
    memory_cache: MemoryCache
    sync_task: asyncio.Task[None] | None = None


class RoutingBudgetState(Operation):
    provider_budget_config: dict[str, Provider] | None = None
    tag_budget_config: dict[str, Tag] | None = None
    deployments: list[Deployment] = Field(default_factory=list)
    steps: Annotated[list[Step], Field(min_length=1)]

    @override
    def run(self) -> Output:
        # pydantic emits ReadOnly-qualifier warnings while LiteLLM builds its logging models; neither side compares them
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            router = asyncio.run(construct_router(self))
            return asyncio.run(run_scenario(self, router))


def deployment_entry(deployment: Deployment) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {"model": DEFAULT_MODEL}
    if deployment.max_budget is not None:
        params["max_budget"] = float(deployment.max_budget)
    if deployment.budget_duration is not None:
        params["budget_duration"] = deployment.budget_duration
    return {
        "model_name": MODEL_GROUP,
        "litellm_params": params,
        "model_info": {"id": deployment.model_id},
    }


def budget(limit: str | None, duration: str | None) -> BudgetConfig:
    return BudgetConfig(
        budget_limit=None if limit is None else float(limit), time_period=duration
    )


def provider_budgets(
    providers: dict[str, Provider] | None,
) -> dict[str, BudgetConfig] | None:
    if providers is None:
        return None
    return {
        name: budget(provider.budget_limit, provider.time_period)
        for name, provider in providers.items()
    }


def tag_budgets(tags: dict[str, Tag] | None) -> dict[str, BudgetConfig] | None:
    if tags is None:
        return None
    return {
        name: budget(tag.max_budget, tag.budget_duration)
        for name, tag in tags.items()
    }


def required(value: str | None, what: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


def money(value: float) -> str:
    return format(Decimal(str(value)).normalize(), "f")


async def sleep_once(limiter: Limiter) -> None:
    original = asyncio.sleep
    asyncio.sleep = recorded_sleep
    try:
        await limiter.periodic_sync_in_memory_spend_with_redis()
    except SleepInterruptedError:
        return
    finally:
        asyncio.sleep = original
    raise ProbeError("invalid_request", "the periodic sync returned without sleeping")


async def success(index: int, step: Step, context: Context) -> Observation:
    cost = float(required(step.cost, "a cost"))
    logging_object: dict[str, JsonValue] = {
        "response_cost": cost,
        "model_id": "" if step.model_id is None else step.model_id,
        "custom_llm_provider": step.provider,
    }
    payload: dict[str, JsonValue] = {"standard_logging_object": logging_object}
    if step.call_type is not None:
        payload["call_type"] = step.call_type
    if step.tags is not None:
        tags: list[JsonValue] = []
        tags.extend(step.tags)
        metadata: dict[str, JsonValue] = {"tags": tags}
        payload["metadata"] = metadata
    await context.limiter.async_log_success_event(payload, None, None, None)
    return Observation(kind=step.kind, step=index)


async def flush_once(index: int, step: Step, context: Context) -> Observation:
    await sleep_once(context.limiter)
    return Observation(kind=step.kind, step=index)


async def begin_sync(index: int, step: Step, context: Context) -> Observation:
    if not context.store.hold:
        raise ProbeError("invalid_request", "begin_sync needs hold_writes before it")
    context.store.entered.clear()
    context.store.read_done.clear()
    context.sync_task = asyncio.create_task(
        context.limiter.periodic_sync_in_memory_spend_with_redis()
    )
    _ = await context.store.entered.wait()
    return Observation(kind=step.kind, step=index)


async def end_sync(index: int, step: Step, context: Context) -> Observation:
    task = context.sync_task
    if task is None:
        raise ProbeError("invalid_request", "end_sync needs a begin_sync before it")
    _ = await context.store.read_done.wait()
    await asyncio.sleep(0)
    _ = task.cancel()
    with suppress(asyncio.CancelledError):
        await task
    context.sync_task = None
    return Observation(kind=step.kind, step=index)


async def memory(index: int, step: Step, context: Context) -> Observation:
    key = required(step.key, "a key")
    value = MONEY.validate_python(await context.memory_cache.async_get_cache(key))
    return Observation(
        kind=step.kind, step=index, value=None if value is None else money(value)
    )


def redis(index: int, step: Step, context: Context) -> Observation:
    key = required(step.key, "a key")
    value = context.store.values.get(key)
    return Observation(
        kind=step.kind, step=index, value=None if value is None else money(value)
    )


def fail_writes(index: int, step: Step, context: Context) -> Observation:
    context.store.fail = True
    return Observation(kind=step.kind, step=index)


def succeed_writes(index: int, step: Step, context: Context) -> Observation:
    context.store.fail = False
    return Observation(kind=step.kind, step=index)


def hold_writes(index: int, step: Step, context: Context) -> Observation:
    context.store.hold = True
    return Observation(kind=step.kind, step=index)


def release_writes(index: int, step: Step, context: Context) -> Observation:
    context.store.release.set()
    return Observation(kind=step.kind, step=index)


StepHandler = Callable[[int, Step, Context], Observation]
AsyncStepHandler = Callable[[int, Step, Context], Awaitable[Observation]]
ASYNC_STEPS: Final[dict[str, AsyncStepHandler]] = {
    "success": success,
    "sync": flush_once,
    "shutdown": flush_once,
    "begin_sync": begin_sync,
    "end_sync": end_sync,
    "memory": memory,
}
SYNC_STEPS: Final[dict[str, StepHandler]] = {
    "redis": redis,
    "fail_writes": fail_writes,
    "succeed_writes": succeed_writes,
    "hold_writes": hold_writes,
    "release_writes": release_writes,
}


async def apply(index: int, step: Step, context: Context) -> Observation:
    async_handler = ASYNC_STEPS.get(step.kind)
    if async_handler is not None:
        return await async_handler(index, step, context)
    return SYNC_STEPS[step.kind](index, step, context)


# the limiter starts its periodic sync at construction; a loop that ends before any task runs cancels it unstarted
async def construct_router(operation: RoutingBudgetState) -> Router:
    await asyncio.sleep(0)
    return Router(
        model_list=[
            deployment_entry(deployment) for deployment in operation.deployments
        ],
        provider_budget_config=provider_budgets(operation.provider_budget_config),
    )


async def run_scenario(operation: RoutingBudgetState, router: Router) -> Output:
    created = router.router_budget_logger
    if created is None:
        raise ProbeError("invalid_request", "the router did not create a budget limiter")
    store = make_store()
    memory_cache = InMemoryCache()
    cache = DualCache(in_memory_cache=memory_cache, redis_cache=store)
    limiter: Limiter = created
    limiter.dual_cache = cache
    limiter.tag_budget_config = tag_budgets(operation.tag_budget_config)
    context = Context(limiter=limiter, store=store, memory_cache=memory_cache)
    observations: list[Observation] = []
    for index, step in enumerate(operation.steps):
        observations.append(await apply(index, step, context))
    return Output(steps=observations)
