import asyncio
import warnings
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Annotated, Final, Literal

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.router_strategy.least_busy import LeastBusyLoggingHandler
from pydantic import Field, JsonValue, TypeAdapter
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false


class Deployment(Strict):
    id: str


class Step(Strict):
    kind: Literal[
        "pre_call",
        "success",
        "failure",
        "select",
        "select_sync",
        "shared",
        "down",
        "up",
    ]
    call_id: int | None = None
    deployment: str | None = None
    call_type: str | None = None
    callback: Literal["sync", "async"] | None = None


class PreCallOutput(Strict):
    kind: Literal["pre_call"] = "pre_call"


class SuccessOutput(Strict):
    kind: Literal["success"] = "success"


class FailureOutput(Strict):
    kind: Literal["failure"] = "failure"


class SelectOutput(Strict):
    kind: Literal["select"] = "select"
    selected: str | None


class SelectSyncOutput(Strict):
    kind: Literal["select_sync"] = "select_sync"
    selected: str | None


class SharedOutput(Strict):
    kind: Literal["shared"] = "shared"
    state: Literal["values", "absent", "unreadable"]
    values: list[int | None]


class DownOutput(Strict):
    kind: Literal["down"] = "down"


class UpOutput(Strict):
    kind: Literal["up"] = "up"


Observation = Annotated[
    PreCallOutput
    | SuccessOutput
    | FailureOutput
    | SelectOutput
    | SelectSyncOutput
    | SharedOutput
    | DownOutput
    | UpOutput,
    Field(discriminator="kind"),
]


class Output(Strict):
    steps: list[Observation]


class DeploymentInfo(Strict):
    id: str


class Selected(Strict):
    model_info: DeploymentInfo


SELECTED: Final = TypeAdapter(Selected)


@dataclass
class FakeStore(RedisCache):
    counts: dict[str, int]
    down: bool

    @override
    def increment_with_floor(self, key: str, value: int, ttl: int) -> int:
        _ = ttl
        if self.down:
            raise RedisConnectionError("connection down")
        count = max(self.counts.get(key, 0) + value, 0)
        self.counts[key] = count
        return count

    @override
    async def async_increment_with_floor(self, key: str, value: int, ttl: int) -> int:
        return self.increment_with_floor(key, value, ttl)

    @override
    def batch_get_counts(self, key_list: list[str]) -> tuple[int | None, ...]:
        if self.down:
            raise RedisConnectionError("connection down")
        return tuple(self.counts.get(key) for key in key_list)

    @override
    async def async_batch_get_counts(self, key_list: list[str]) -> tuple[int | None, ...]:
        return self.batch_get_counts(key_list)


@dataclass
class Context:
    handler: LeastBusyLoggingHandler
    store: FakeStore
    model_group: str
    deployments: list[dict[str, JsonValue]]
    keys: list[str]
    calls: dict[int, dict[str, object]]
    shared: bool


class LeastBusy(Operation):
    model_group: str
    shared: bool
    deployments: list[Deployment]
    steps: list[Step]

    @override
    def run(self) -> Strict:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(run_scenario(self))


def required_int(value: int | None, what: str) -> int:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


def required_text(value: str | None, what: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


def required_callback(value: Literal["sync", "async"] | None) -> Literal["sync", "async"]:
    if value is None:
        raise ProbeError("invalid_request", "the step needs a callback mode")
    return value


def attempt_kwargs(model_group: str, deployment_id: str, call_type: str | None) -> dict[str, object]:
    litellm_params: dict[str, object] = {
        "metadata": {"model_group": model_group},
        "model_info": {"id": deployment_id},
    }
    kwargs: dict[str, object] = {"litellm_params": litellm_params}
    if call_type is not None:
        kwargs["call_type"] = call_type
    return kwargs


def pre_call(step: Step, context: Context) -> Observation:
    deployment_id = required_text(step.deployment, "a deployment id")
    call_id = required_int(step.call_id, "a call id")
    kwargs = attempt_kwargs(context.model_group, deployment_id, step.call_type)
    messages: list[str] = []
    context.handler.log_pre_api_call(context.model_group, messages, kwargs)
    context.calls[call_id] = kwargs
    return PreCallOutput()


async def finish(step: Step, context: Context) -> Observation:
    call_id = required_int(step.call_id, "a call id")
    if call_id not in context.calls:
        raise ProbeError("invalid_request", f"call {call_id} was never started")
    kwargs = context.calls.pop(call_id)
    mode = required_callback(step.callback)
    if step.kind == "success":
        if mode == "sync":
            context.handler.log_success_event(kwargs, None, None, None)
        else:
            await context.handler.async_log_success_event(kwargs, None, None, None)
        return SuccessOutput()
    if mode == "sync":
        context.handler.log_failure_event(kwargs, None, None, None)
    else:
        await context.handler.async_log_failure_event(kwargs, None, None, None)
    return FailureOutput()


async def select_async(step: Step, context: Context) -> Observation:
    _ = step
    returned = await context.handler.async_get_available_deployments(context.model_group, context.deployments)
    return SelectOutput(selected=selected_id(returned))


def select_sync(step: Step, context: Context) -> Observation:
    _ = step
    returned = context.handler.get_available_deployments(context.model_group, context.deployments)
    return SelectSyncOutput(selected=selected_id(returned))


def selected_id(returned: Mapping[str, object] | None) -> str | None:
    if returned is None:
        return None
    return SELECTED.validate_python(returned).model_info.id


def shared(step: Step, context: Context) -> Observation:
    _ = step
    if not context.shared:
        return SharedOutput(state="absent", values=[])
    try:
        values = list(context.store.batch_get_counts(context.keys))
    except RedisConnectionError:
        return SharedOutput(state="unreadable", values=[])
    return SharedOutput(state="values", values=values)


def switch(step: Step, context: Context) -> Observation:
    if step.kind == "down":
        context.store.down = True
        return DownOutput()
    context.store.down = False
    return UpOutput()


SyncStep = Callable[[Step, Context], Observation]
AsyncStep = Callable[[Step, Context], Awaitable[Observation]]

SYNC_STEPS: Final[dict[str, SyncStep]] = {
    "pre_call": pre_call,
    "select_sync": select_sync,
    "shared": shared,
    "down": switch,
    "up": switch,
}
ASYNC_STEPS: Final[dict[str, AsyncStep]] = {
    "success": finish,
    "failure": finish,
    "select": select_async,
}


async def apply_step(step: Step, context: Context) -> Observation:
    async_handler = ASYNC_STEPS.get(step.kind)
    if async_handler is not None:
        return await async_handler(step, context)
    sync_handler = SYNC_STEPS.get(step.kind)
    if sync_handler is None:
        raise ProbeError("invalid_request", f"unknown step {step.kind}")
    return sync_handler(step, context)


async def run_scenario(operation: LeastBusy) -> Output:
    store = FakeStore(counts={}, down=False)
    cache = DualCache(in_memory_cache=InMemoryCache(), redis_cache=store if operation.shared else None)
    handler = LeastBusyLoggingHandler(router_cache=cache)
    deployments: list[dict[str, JsonValue]] = [
        {"model_info": {"id": deployment.id}} for deployment in operation.deployments
    ]
    keys = [f"{operation.model_group}_request_count:{deployment.id}" for deployment in operation.deployments]
    context = Context(
        handler=handler,
        store=store,
        model_group=operation.model_group,
        deployments=deployments,
        keys=keys,
        calls={},
        shared=operation.shared,
    )
    return Output(steps=[await apply_step(step, context) for step in operation.steps])
