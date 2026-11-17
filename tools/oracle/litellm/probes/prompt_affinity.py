import asyncio
from typing import Final, Protocol

import litellm
from litellm.caching.dual_cache import DualCache
from litellm.router_utils.pre_call_checks.prompt_caching_deployment_check import (
    PromptCachingDeploymentCheck,
)
from litellm.router_utils.prompt_caching_cache import PromptCachingCache
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class Deployment(Strict):
    id: str
    model: str


class Write(Strict):
    model_id: str
    call_type: str = "completion"
    messages: list[dict[str, JsonValue]]


class Facts(Strict):
    providers: dict[str, str] = {}
    supports: dict[str, bool] = {}


class Output(Strict):
    selected: list[int]
    cache_key: str | None
    pin: str | None


class Check(Protocol):
    async def async_filter_deployments(
        self,
        model: str,
        healthy_deployments: list[dict[str, JsonValue]],
        messages: list[AllMessageValues] | None,
        request_kwargs: dict[str, JsonValue] | None = None,
    ) -> list[dict[str, JsonValue]]: ...

    async def async_log_success_event(
        self,
        kwargs: dict[str, object],
        response_obj: object,
        start_time: object,
        end_time: object,
    ) -> None: ...


class PromptAffinity(Operation):
    model: str
    deployments: list[Deployment]
    write: Write | None = None
    messages: list[AllMessageValues] | None = None
    enable_prompt_caching: bool | None = None
    target_order: int | None = None
    global_enabled: bool = False
    facts: Facts = Facts()

    @override
    def run(self) -> Strict:
        return asyncio.run(self.filtered())

    async def filtered(self) -> Output:
        cache: Final = DualCache()
        check: Final[Check] = PromptCachingDeploymentCheck(cache=cache)
        deployments: Final[list[dict[str, JsonValue]]] = [
            {
                "model_name": self.model,
                "litellm_params": {"model": deployment.model},
                "model_info": {"id": deployment.id},
            }
            for deployment in self.deployments
        ]
        if self.write is not None:
            event: Final[dict[str, JsonValue]] = {
                "call_type": self.write.call_type,
                "model": self.model,
                "messages": self.write.messages,
                "model_id": self.write.model_id,
            }
            kwargs: Final[dict[str, object]] = {"standard_logging_object": event}
            await check.async_log_success_event(kwargs, None, None, None)
        request_kwargs: Final[dict[str, JsonValue]] = {}
        if self.enable_prompt_caching is not None:
            request_kwargs["enable_prompt_caching"] = self.enable_prompt_caching
        if self.target_order is not None:
            request_kwargs["_target_order"] = self.target_order
        litellm.enable_anthropic_prompt_caching = self.global_enabled
        try:
            chosen = await check.async_filter_deployments(
                model=self.model,
                healthy_deployments=deployments,
                messages=self.messages,
                request_kwargs=request_kwargs,
            )
        finally:
            litellm.enable_anthropic_prompt_caching = False
        pinned = await PromptCachingCache(cache).async_get_model_id(
            messages=self.messages, tools=None
        )
        return Output(
            selected=[
                position
                for position, candidate in enumerate(deployments)
                if any(candidate is kept for kept in chosen)
            ],
            cache_key=PromptCachingCache.get_prompt_caching_cache_key(self.messages, None),
            pin=None if pinned is None else pinned["model_id"],
        )
