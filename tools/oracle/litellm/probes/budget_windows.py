import asyncio
import warnings
from collections.abc import Mapping
from decimal import Decimal
from typing import Annotated, Final, Literal, Protocol

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.router import Router
from litellm.types.utils import BudgetConfig
from pydantic import Field, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MONEY: Final = TypeAdapter(float | None)
DEFAULT_MODEL: Final = "openai/gpt-4o-mini"
MODEL_GROUP: Final = "gpt"


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
    kind: Literal["build", "init", "start", "reset"]
    model_id: str | None = None
    start_time_key: str | None = None
    spend_key: str | None = None
    current_time: str | None = None
    response_cost: str | None = None
    ttl_seconds: int | None = None


class StepOutput(Strict):
    kind: str
    should_init: bool | None
    limiter: bool | None
    tags: list[str] | None
    window_started: bool | None
    spend: str | None


class Output(Strict):
    steps: list[StepOutput]


class Cache(Protocol):
    async def async_get_cache(self, key: str) -> object: ...

    async def async_set_cache(
        self, key: str, value: object, *, local_only: bool = False, **kwargs: object
    ) -> None: ...


class Limiter(Protocol):
    @property
    def tag_budget_config(self) -> Mapping[str, object] | None: ...

    async def async_log_success_event(
        self, kwargs: dict[str, object], response_obj: object, start_time: object, end_time: object
    ) -> None: ...


def deployment_entry(deployment: Deployment) -> dict[str, object]:
    params: dict[str, object] = {"model": DEFAULT_MODEL}
    if deployment.max_budget is not None:
        params["max_budget"] = float(deployment.max_budget)
    if deployment.budget_duration is not None:
        params["budget_duration"] = deployment.budget_duration
    return {"model_name": MODEL_GROUP, "litellm_params": params, "model_info": {"id": deployment.model_id}}


def provider_budgets(providers: dict[str, Provider] | None) -> dict[str, BudgetConfig] | None:
    if providers is None:
        return None
    budgets: dict[str, BudgetConfig] = {}
    for name, provider in providers.items():
        limit = None if provider.budget_limit is None else float(provider.budget_limit)
        budgets[name] = BudgetConfig(budget_limit=limit, time_period=provider.time_period)
    return budgets


def tag_names(tags: dict[str, Tag] | None, model_list: list[dict[str, object]]) -> list[str] | None:
    if tags is None:
        litellm.tag_budget_config = None
        return None
    configured: dict[str, object] = {}
    for name, tag in tags.items():
        budget: dict[str, object] = {"budget_duration": tag.budget_duration}
        if tag.max_budget is not None:
            budget["max_budget"] = float(tag.max_budget)
        configured[name] = budget
    litellm.tag_budget_config = configured
    try:
        _router = Router(model_list=model_list)
    except ValueError:
        return None
    finally:
        litellm.tag_budget_config = None
    return sorted(tags)


async def success_event(limiter: Limiter, step: Step, cost: float) -> None:
    if step.model_id is None:
        raise ProbeError("invalid_request", "the step needs a model_id")
    payload: dict[str, object] = {
        "standard_logging_object": {"response_cost": cost, "model_id": step.model_id, "custom_llm_provider": None},
    }
    await limiter.async_log_success_event(payload, None, None, None)


async def observe(step: Step, cache: Cache, memory: InMemoryCache) -> StepOutput:
    if step.start_time_key is None:
        raise ProbeError("invalid_request", "the step needs a start_time_key")
    started = await memory.async_get_ttl(step.start_time_key) is not None
    spend = None
    if step.spend_key is not None:
        value = MONEY.validate_python(await cache.async_get_cache(step.spend_key))
        spend = None if value is None else format(Decimal(str(value)).normalize(), "f")
    return StepOutput(kind=step.kind, should_init=None, limiter=None, tags=None, window_started=started, spend=spend)


async def reset_step(step: Step, limiter: Limiter, cache: Cache, memory: InMemoryCache) -> StepOutput:
    if step.start_time_key is None or step.response_cost is None or step.ttl_seconds is None:
        raise ProbeError("invalid_request", "the reset step needs start_time_key, response_cost and ttl_seconds")
    await success_event(limiter, step, float(step.response_cost))
    await cache.async_set_cache(step.start_time_key, 0.0, ttl=step.ttl_seconds)
    await success_event(limiter, step, float(step.response_cost))
    return await observe(step, cache, memory)


async def run_steps(
    deployments: list[Deployment],
    providers: dict[str, Provider] | None,
    tag_budget_config: dict[str, Tag] | None,
    steps: list[Step],
) -> Output:
    model_list: list[dict[str, object]] = [deployment_entry(deployment) for deployment in deployments]
    litellm.tag_budget_config = None
    router = Router(model_list=model_list, provider_budget_config=provider_budgets(providers))
    created = router.router_budget_logger
    if created is None:
        raise ProbeError("invalid_request", "the router did not create a budget limiter")
    # the limiter schedules _init_provider_budget_in_cache as a task, so one yield lets it write the keys
    await asyncio.sleep(0)
    limiter: Limiter = created
    cache: Cache = router.cache
    memory: InMemoryCache = router.cache.in_memory_cache
    outputs: list[StepOutput] = []
    for step in steps:
        match step.kind:
            case "build":
                tags = tag_names(tag_budget_config, model_list)
                outputs.append(
                    StepOutput(
                        kind=step.kind, should_init=True, limiter=True, tags=tags, window_started=None, spend=None
                    )
                )
            case "init":
                outputs.append(await observe(step, cache, memory))
            case "start":
                await success_event(limiter, step, 0.0)
                outputs.append(await observe(step, cache, memory))
            case "reset":
                outputs.append(await reset_step(step, limiter, cache, memory))
    return Output(steps=outputs)


class BudgetWindows(Operation):
    deployments: list[Deployment]
    provider_budget_config: dict[str, Provider] | None = None
    tag_budget_config: dict[str, Tag] | None = None
    steps: Annotated[list[Step], Field(min_length=1)]

    @override
    def run(self) -> Output:
        # pydantic emits ReadOnly-qualifier warnings while LiteLLM builds its logging models; neither side compares them
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(
                run_steps(self.deployments, self.provider_budget_config, self.tag_budget_config, self.steps)
            )
