import asyncio
from typing import Final, Literal, Protocol
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.router_utils.pre_call_checks.deployment_affinity_check import (
    DeploymentAffinityCheck,
)
from litellm.types.llms.openai import AllMessageValues
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnknownMemberType=false


class Deployment(Strict):
    id: str
    model: str


class Step(Strict):
    kind: Literal["filter", "hook"]
    now_millis: int
    model: str | None = None
    pool: list[str] | None = None
    metadata: dict[str, JsonValue] | None = None
    litellm_metadata: dict[str, JsonValue] | None = None
    previous_response_id: str | None = None
    target_order: int | None = None
    model_info: dict[str, JsonValue] | None = None


class Observation(Strict):
    action: Literal["filter", "hook"]
    selected: list[str]


class Output(Strict):
    steps: list[Observation]


class FilterCheck(Protocol):
    async def async_filter_deployments(
        self,
        model: str,
        healthy_deployments: list[dict[str, JsonValue]],
        messages: list[AllMessageValues] | None,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> list[dict[str, JsonValue]]: ...


def deployment_dict(deployment: Deployment, model_group: str) -> dict[str, JsonValue]:
    return {
        "model_name": model_group,
        "litellm_params": {"model": deployment.model},
        "model_info": {"id": deployment.id},
    }


def wall(now_millis: int) -> float:
    return now_millis / 1000


class DeploymentAffinity(Operation):
    model: str
    deployments: list[Deployment]
    steps: list[Step]
    ttl_seconds: int
    defaults: list[Literal["user_key", "responses", "session"]] = Field(default_factory=list)
    model_group_affinity_config: dict[str, list[str]] = Field(default_factory=dict)
    priority_groups: list[str] = Field(default_factory=list)

    @override
    def run(self) -> Strict:
        return asyncio.run(self.filtered())

    async def filtered(self) -> Output:
        cache: Final = DualCache()
        priority: Final[set[str]] = set(self.priority_groups)
        check: Final[DeploymentAffinityCheck] = DeploymentAffinityCheck(
            cache=cache,
            ttl_seconds=self.ttl_seconds,
            enable_user_key_affinity="user_key" in self.defaults,
            enable_responses_api_affinity="responses" in self.defaults,
            enable_session_id_affinity="session" in self.defaults,
            model_group_affinity_config=self.model_group_affinity_config,
            is_priority_group=lambda name: name in priority,
        )
        observations: Final[list[Observation]] = []
        for step in self.steps:
            with patch("time.time", return_value=wall(step.now_millis)):
                observations.append(await self.apply_step(check, step))
        return Output(steps=observations)

    async def apply_step(self, check: DeploymentAffinityCheck, step: Step) -> Observation:
        kwargs: Final[dict[str, JsonValue]] = self.request_kwargs(step)
        if step.kind == "hook":
            if step.model_info is not None:
                kwargs["model_info"] = step.model_info
            await check.async_pre_call_deployment_hook(kwargs, None)
            return Observation(action="hook", selected=[])
        pool_deployments: Final[list[Deployment]] = [
            deployment
            for deployment in self.deployments
            if step.pool is None or deployment.id in step.pool
        ]
        pool: Final[list[dict[str, JsonValue]]] = [
            deployment_dict(deployment, self.model) for deployment in pool_deployments
        ]
        model: Final = self.model if step.model is None else step.model
        chosen: Final[list[dict[str, JsonValue]]] = await self.filtered_pool(
            check, model, pool, kwargs
        )
        return Observation(
            action="filter",
            selected=[
                deployment.id
                for position, deployment in enumerate(pool_deployments)
                if any(pool[position] is kept for kept in chosen)
            ],
        )

    # LiteLLM types the method as list[dict], so the call goes through the protocol parameter
    async def filtered_pool(
        self,
        check: FilterCheck,
        model: str,
        pool: list[dict[str, JsonValue]],
        kwargs: dict[str, JsonValue],
    ) -> list[dict[str, JsonValue]]:
        return await check.async_filter_deployments(
            model=model,
            healthy_deployments=pool,
            messages=None,
            request_kwargs=kwargs,
        )

    def request_kwargs(self, step: Step) -> dict[str, JsonValue]:
        kwargs: dict[str, JsonValue] = {}
        if step.metadata is not None:
            kwargs["metadata"] = step.metadata
        if step.litellm_metadata is not None:
            kwargs["litellm_metadata"] = step.litellm_metadata
        if step.previous_response_id is not None:
            kwargs["previous_response_id"] = step.previous_response_id
        if step.target_order is not None:
            kwargs["_target_order"] = step.target_order
        return kwargs
