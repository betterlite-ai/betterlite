import asyncio
import os
from typing import Literal, Protocol

import litellm
from litellm.exceptions import (
    BadRequestError,
    ContextWindowExceededError,
    ServiceUnavailableError,
)
from litellm.router import Router
from litellm.types.router import RouterRateLimitError
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

API_KEY = "sk-probe"


class Entry(Strict):
    model_name: str
    model: str
    id: str
    base_model: str | None = None
    max_input_tokens: int | None = None
    blocked: bool | None = None
    supports_web_search: bool | None = None
    supported_environments: list[str] | None = None
    order: int | None = None


class Lookup(Strict):
    model: str
    specific_deployment: bool = False
    messages: list[dict[str, str]] | None = None
    tools: list[dict[str, JsonValue]] = Field(default_factory=list)
    response_format: bool = False
    target_order: int | None = None


class Found(Strict):
    outcome: Literal[
        "selected", "bad_request", "rate_limited", "context_exceeded", "service_unavailable"
    ]
    selected: list[str]


class Output(Strict):
    lookups: list[Found]


class Returned(Strict):
    model_info: dict[str, JsonValue]


class RouterView(Protocol):
    async def async_get_healthy_deployments(
        self,
        model: str,
        request_kwargs: dict[str, object],
        messages: list[dict[str, str]] | None = None,
        *,
        specific_deployment: bool = False,
    ) -> list[dict[str, JsonValue]] | dict[str, JsonValue]: ...


def model_list_entry(entry: Entry) -> dict[str, object]:
    info: dict[str, object] = {"id": entry.id}
    info.update(
        {
            key: value
            for key, value in (
                ("base_model", entry.base_model),
                ("max_input_tokens", entry.max_input_tokens),
                ("blocked", entry.blocked),
                ("supports_web_search", entry.supports_web_search),
                ("supported_environments", entry.supported_environments),
            )
            if value is not None
        }
    )
    params: dict[str, object] = {"model": entry.model, "api_key": API_KEY}
    if entry.order is not None:
        params["order"] = entry.order
    return {"model_name": entry.model_name, "litellm_params": params, "model_info": info}


def request_kwargs(lookup: Lookup) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    if lookup.response_format:
        kwargs["response_format"] = {"type": "json_object"}
    if lookup.tools:
        kwargs["tools"] = lookup.tools
    if lookup.target_order is not None:
        kwargs["_target_order"] = lookup.target_order
    return kwargs


def selected_ids(deployments: list[dict[str, JsonValue]] | dict[str, JsonValue]) -> list[str]:
    if isinstance(deployments, dict):
        return [_id(deployments)]
    return [_id(deployment) for deployment in deployments]


def _id(deployment: dict[str, JsonValue]) -> str:
    identifier = TypeAdapter(Returned).validate_python(deployment).model_info["id"]
    if not isinstance(identifier, str):
        raise TypeError("the selected deployment has no string id")
    return identifier


class StaticEligibility(Operation):
    environment: str
    enable_pre_call_checks: bool
    drop_params: bool = False
    models: list[Entry]
    lookups: list[Lookup]

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        os.environ["LITELLM_ENVIRONMENT"] = self.environment
        litellm.drop_params = self.drop_params
        router = Router(
            model_list=[model_list_entry(entry) for entry in self.models],
            enable_pre_call_checks=self.enable_pre_call_checks,
        )
        return Output(lookups=[await self.answer(router, lookup) for lookup in self.lookups])

    async def answer(self, router: RouterView, lookup: Lookup) -> Found:
        try:
            deployments = await router.async_get_healthy_deployments(
                model=lookup.model,
                request_kwargs=request_kwargs(lookup),
                messages=lookup.messages,
                specific_deployment=lookup.specific_deployment,
            )
        except ContextWindowExceededError:
            return Found(outcome="context_exceeded", selected=[])
        except ServiceUnavailableError:
            return Found(outcome="service_unavailable", selected=[])
        except RouterRateLimitError:
            return Found(outcome="rate_limited", selected=[])
        except BadRequestError:
            return Found(outcome="bad_request", selected=[])
        return Found(outcome="selected", selected=selected_ids(deployments))
