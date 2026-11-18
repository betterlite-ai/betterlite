import asyncio
import warnings
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from decimal import Decimal
from typing import Annotated, Final, Literal, TypeVar, overload

from litellm.caching.caching import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.exceptions import RateLimitError
from litellm.router_strategy import lowest_tpm_rpm_v2
from litellm.router_strategy.lowest_tpm_rpm_v2 import LowestTPMLoggingHandler_v2
from litellm.types.caching import RedisPipelineIncrementOperation
from pydantic import Field, JsonValue
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

from probes.protocol import Millis, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true
# pyright: reportMissingTypeStubs=false, reportUnknownMemberType=false, reportUnusedCallResult=false

MILLIS_PER_SECOND: Final = 1000


class DeploymentInput(Strict):
    model_id: str | None = None
    model: str | None = None
    rpm: int | None = None
    params_rpm: int | None = None
    model_info_rpm: int | None = None
    num_retries: int | None = None


class StandardLoggingInput(Strict):
    model_group: str | None = None
    model: str | None = None
    model_id: str | None = None
    total_tokens: int


class Step(Strict):
    kind: Literal["pre_call", "success", "sync", "seed_store", "store", "down", "up"]
    now_millis: Millis = 0
    sync: bool = False
    deployment: DeploymentInput | None = None
    call_type: str | None = None
    standard_logging_object: StandardLoggingInput | None = None
    key: str | None = None
    value: int | None = None


class ObservedAllowed(Strict):
    kind: Literal["allowed"] = "allowed"
    step: int


class ObservedRejected(Strict):
    kind: Literal["rejected"] = "rejected"
    step: int
    status_code: int
    code: str
    error_type: str
    retry_after: str | None
    num_retries: int | None


class ObservedRecorded(Strict):
    kind: Literal["recorded"] = "recorded"
    step: int


class ObservedStore(Strict):
    kind: Literal["store"] = "store"
    step: int
    value: str


Observation = Annotated[
    ObservedAllowed | ObservedRejected | ObservedRecorded | ObservedStore,
    Field(discriminator="kind"),
]


class ScenarioOutput(Strict):
    steps: list[Observation]


class StepClock:
    def __init__(self) -> None:
        self.millis: int = 0

    def wall(self) -> float:
        return self.millis / MILLIS_PER_SECOND

    def utc(self) -> datetime:
        return datetime.fromtimestamp(self.millis / MILLIS_PER_SECOND, tz=UTC)


SleepResult = TypeVar("SleepResult")


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


# RedisCache's constructor needs a host or url and connects; the dataclass sets only the state the pipelines read
@dataclass
class FakeStore(RedisCache):
    values: dict[str, float]
    down: bool

    @override
    async def async_increment_pipeline(
        self,
        increment_list: list[RedisPipelineIncrementOperation],
        **_kwargs: JsonValue,
    ) -> list[float] | None:
        if self.down:
            raise RedisConnectionError("connection down")
        results: list[float] = []
        for increment in increment_list:
            total = self.values.get(increment["key"], 0.0) + increment["increment_value"]
            self.values[increment["key"]] = total
            results.append(total)
        return results

    @override
    def increment_cache(
        self, key: str, value: int, ttl: float | None = None, **_kwargs: JsonValue
    ) -> int:
        _ = ttl
        if self.down:
            raise RedisConnectionError("connection down")
        total = self.values.get(key, 0.0) + value
        self.values[key] = total
        return int(total)


@dataclass
class Context:
    strategy: LowestTPMLoggingHandler_v2
    store: FakeStore


class UsageV2Accounting(Operation):
    ttl_seconds: int | None = None
    steps: list[Step]

    @override
    def run(self) -> Strict:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(run_scenario(self))


def required(value: str | None, what: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


# the deployment dict lowest_tpm_rpm_v2 reads: rpm lives at the top level, then litellm_params, then model_info
def deployment_dict(deployment: DeploymentInput) -> dict[str, JsonValue]:
    model_id = required(deployment.model_id, "a deployment id")
    model = required(deployment.model, "a model")
    litellm_params: dict[str, JsonValue] = {"model": model}
    if deployment.params_rpm is not None:
        litellm_params["rpm"] = deployment.params_rpm
    model_info: dict[str, JsonValue] = {"id": model_id}
    if deployment.model_info_rpm is not None:
        model_info["rpm"] = deployment.model_info_rpm
    payload: dict[str, JsonValue] = {
        "litellm_params": litellm_params,
        "model_info": model_info,
    }
    if deployment.rpm is not None:
        payload["rpm"] = deployment.rpm
    if deployment.num_retries is not None:
        payload["num_retries"] = deployment.num_retries
    return payload


def success_kwargs(step: Step) -> dict[str, JsonValue]:
    kwargs: dict[str, JsonValue] = {"call_type": step.call_type}
    logged = step.standard_logging_object
    if logged is not None:
        hidden_params: dict[str, JsonValue] = {"litellm_model_name": logged.model}
        payload: dict[str, JsonValue] = {
            "model_group": logged.model_group,
            "model_id": logged.model_id,
            "total_tokens": logged.total_tokens,
            "hidden_params": hidden_params,
        }
        kwargs["standard_logging_object"] = payload
    return kwargs


def rejected(index: int, error: RateLimitError) -> ObservedRejected:
    return ObservedRejected(
        step=index,
        status_code=error.status_code,
        code=error.code or "",
        error_type=error.type or "",
        retry_after=error.response.headers.get("retry-after"),
        num_retries=error.num_retries,
    )


async def pre_call(index: int, step: Step, context: Context) -> Observation:
    deployment = step.deployment
    if deployment is None:
        raise ProbeError("invalid_request", "the step needs a deployment")
    payload = deployment_dict(deployment)
    try:
        if step.sync:
            _ = context.strategy.pre_call_check(payload)
        else:
            _ = await context.strategy.async_pre_call_check(payload, None)
    except RateLimitError as error:
        return rejected(index, error)
    return ObservedAllowed(step=index)


async def success(index: int, step: Step, context: Context) -> Observation:
    kwargs = success_kwargs(step)
    if step.sync:
        context.strategy.log_success_event(kwargs, None, None, None)
    else:
        await context.strategy.async_log_success_event(kwargs, None, None, None)
    return ObservedRecorded(step=index)


async def sleep_once(
    strategy: LowestTPMLoggingHandler_v2, interval: float | None
) -> float:
    original = asyncio.sleep
    asyncio.sleep = recorded_sleep
    try:
        await strategy.periodic_sync_in_memory_spend_with_redis(
            default_sync_interval=interval
        )
    except SleepInterruptedError as error:
        return error.delay
    finally:
        asyncio.sleep = original
    raise ProbeError("invalid_request", "the periodic sync returned without sleeping")


async def sync(index: int, _step: Step, context: Context) -> Observation:
    _delay = await sleep_once(context.strategy, None)
    return ObservedRecorded(step=index)


def seed_store(index: int, step: Step, context: Context) -> Observation:
    key = required(step.key, "a key")
    value = step.value
    if value is None:
        raise ProbeError("invalid_request", "the step needs a value")
    context.store.values[key] = float(value)
    return ObservedRecorded(step=index)


def store_value(index: int, step: Step, context: Context) -> Observation:
    key = required(step.key, "a key")
    value = context.store.values.get(key, 0.0)
    return ObservedStore(step=index, value=format(Decimal(value).normalize(), "f"))


def switch(index: int, step: Step, context: Context) -> Observation:
    context.store.down = step.kind == "down"
    return ObservedRecorded(step=index)


StepHandler = Callable[[int, Step, Context], Observation]
AsyncStepHandler = Callable[[int, Step, Context], Awaitable[Observation]]
ASYNC_STEPS: Final[dict[str, AsyncStepHandler]] = {
    "pre_call": pre_call,
    "success": success,
    "sync": sync,
}
SYNC_STEPS: Final[dict[str, StepHandler]] = {
    "seed_store": seed_store,
    "store": store_value,
    "down": switch,
    "up": switch,
}


async def apply_step(index: int, step: Step, context: Context) -> Observation:
    async_handler = ASYNC_STEPS.get(step.kind)
    if async_handler is not None:
        return await async_handler(index, step, context)
    return SYNC_STEPS[step.kind](index, step, context)


async def run_scenario(operation: UsageV2Accounting) -> ScenarioOutput:
    clock = StepClock()
    store = FakeStore(values={}, down=False)
    routing_args: dict[str, JsonValue] = {}
    if operation.ttl_seconds is not None:
        routing_args["ttl"] = operation.ttl_seconds
    cache = DualCache(
        in_memory_cache=InMemoryCache(clock=clock.wall), redis_cache=store
    )
    strategy = LowestTPMLoggingHandler_v2(
        router_cache=cache, routing_args=routing_args
    )
    strategy.cancel_sync_task()
    original = lowest_tpm_rpm_v2.get_utc_datetime
    lowest_tpm_rpm_v2.get_utc_datetime = clock.utc
    try:
        context = Context(strategy=strategy, store=store)
        observations: list[Observation] = []
        for index, step in enumerate(operation.steps):
            clock.millis = step.now_millis
            observations.append(await apply_step(index, step, context))
        return ScenarioOutput(steps=observations)
    finally:
        lowest_tpm_rpm_v2.get_utc_datetime = original
