import asyncio
import os
from typing import ClassVar, Literal, Protocol

import litellm
from litellm.exceptions import BadRequestError, ServiceUnavailableError
from litellm.router import Router
from litellm.types.router import RouterRateLimitError
from pydantic import ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

API_KEY = "sk-probe"
Strategy = Literal["simple-shuffle", "lar1"]


class Entry(Strict):
    model_name: str
    model: str
    id: str
    use_in_pass_through: bool | None = None
    weight: int | None = Field(default=None, ge=0, le=2**32 - 1)
    rpm: int | None = Field(default=None, ge=0, le=2**32 - 1)
    tpm: int | None = Field(default=None, ge=0, le=2**32 - 1)
    order: int | None = None
    blocked: bool | None = None
    supported_environments: list[str] | None = None


class Lookup(Strict):
    model: str
    specific_deployment: bool = False
    messages: bool = False
    response_format: bool = False
    routing_strategy: Strategy | None = None


class Input(Strict):
    strategy: Strategy
    enable_pre_call_checks: bool = False
    drop_params: bool = False
    models: list[Entry]
    lookups: list[Lookup]


class Selected(Strict):
    model_name: str
    model: str
    id: str


class Found(Strict):
    outcome: Literal["selected", "bad_request", "rate_limited", "service_unavailable"]
    selected: Selected | None


class Output(Strict):
    lookups: list[Found]


class Returned(Strict):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


class RouterView(Protocol):
    async def async_get_available_deployment_for_pass_through(
        self,
        model: str,
        request_kwargs: dict[str, object],
        messages: list[dict[str, str]] | None = None,
        responses_input: str | list[JsonValue] | None = None,
        /,
        *,
        specific_deployment: bool = False,
    ) -> dict[str, JsonValue]: ...


def model_list_entry(entry: Entry) -> dict[str, object]:
    info: dict[str, object] = {"id": entry.id}
    if entry.blocked is not None:
        info["blocked"] = entry.blocked
    if entry.supported_environments is not None:
        info["supported_environments"] = entry.supported_environments
    params: dict[str, object] = {"model": entry.model, "api_key": API_KEY}
    if entry.use_in_pass_through is not None:
        params["use_in_pass_through"] = entry.use_in_pass_through
    for key, value in (
        ("weight", entry.weight),
        ("rpm", entry.rpm),
        ("tpm", entry.tpm),
        ("order", entry.order),
    ):
        if value is not None:
            params[key] = value
    return {"model_name": entry.model_name, "litellm_params": params, "model_info": info}


def request_kwargs(lookup: Lookup) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    if lookup.response_format:
        kwargs["response_format"] = {"type": "json_object"}
    if lookup.routing_strategy is not None:
        kwargs["routing_strategy"] = lookup.routing_strategy
    return kwargs


def messages(lookup: Lookup) -> list[dict[str, str]] | None:
    if not lookup.messages:
        return None
    return [{"role": "user", "content": "hi"}]


def selected(deployment: dict[str, JsonValue]) -> Selected:
    returned = TypeAdapter(Returned).validate_python(deployment)
    model = returned.litellm_params.get("model")
    if not isinstance(model, str):
        raise TypeError("the selected deployment has no model")
    identifier = returned.model_info.get("id")
    if not isinstance(identifier, str):
        raise TypeError("the selected deployment has no string id")
    return Selected(model_name=returned.model_name, model=model, id=identifier)


class PassthroughSelection(Operation):
    strategy: Strategy
    enable_pre_call_checks: bool = False
    drop_params: bool = False
    models: list[Entry]
    lookups: list[Lookup]

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        os.environ["LITELLM_ENVIRONMENT"] = "production"
        litellm.drop_params = self.drop_params
        router = Router(
            model_list=[model_list_entry(entry) for entry in self.models],
            enable_pre_call_checks=self.enable_pre_call_checks,
            routing_strategy=self.strategy,
        )
        return Output(lookups=[await self.answer(router, lookup) for lookup in self.lookups])

    async def answer(self, router: RouterView, lookup: Lookup) -> Found:
        try:
            deployment = await router.async_get_available_deployment_for_pass_through(
                lookup.model,
                request_kwargs(lookup),
                messages(lookup),
                None,
                specific_deployment=lookup.specific_deployment,
            )
        except ServiceUnavailableError:
            return Found(outcome="service_unavailable", selected=None)
        except RouterRateLimitError:
            return Found(outcome="rate_limited", selected=None)
        except BadRequestError:
            return Found(outcome="bad_request", selected=None)
        return Found(outcome="selected", selected=selected(deployment))
