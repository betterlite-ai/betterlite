import asyncio
from typing import Final, Protocol, TypedDict

from litellm.router import Router
from litellm.router_strategy.quality_router.quality_router import QualityRouter as LiteLLMQualityRouter
from litellm.router_utils.add_retry_fallback_headers import (
    apply_quality_router_decision_headers,
    ensure_response_additional_headers,
)
from litellm.types.router import PreRoutingHookResponse
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_HEADERS: Final = TypeAdapter(dict[str, str])


class ResponseFields(TypedDict):
    model: str
    routing_decision: JsonValue | None


_RESPONSE: Final = TypeAdapter(ResponseFields)


# the hook as the router calls it: the concrete method's bare dict and list are partially unknown
class PreRoutingHook(Protocol):
    async def async_pre_routing_hook(
        self,
        model: str,
        request_kwargs: dict[str, JsonValue],
        messages: list[dict[str, JsonValue]] | None,
    ) -> PreRoutingHookResponse | None: ...


def pre_route(
    hook: PreRoutingHook,
    model: str,
    request_kwargs: dict[str, JsonValue],
    messages: list[dict[str, JsonValue]] | None,
) -> PreRoutingHookResponse | None:
    return asyncio.run(
        hook.async_pre_routing_hook(model=model, request_kwargs=request_kwargs, messages=messages)
    )


class ParamsInput(Strict):
    model: str
    quality_router_config: dict[str, JsonValue] | None = None


class InfoInput(Strict):
    litellm_routing_preferences: dict[str, JsonValue] | None = None
    # a price may be written as a JSON string, which LiteLLM's model_info coerces to a float
    input_cost_per_token: float | str | None = None


class DeploymentInput(Strict):
    model_name: str
    litellm_params: ParamsInput
    model_info: InfoInput = Field(default_factory=InfoInput)


class Document(Strict):
    model_list: list[DeploymentInput]


class Output(Strict):
    model: str | None = None
    decision: JsonValue | None = None
    metadata: JsonValue | None = None
    headers: dict[str, str] = Field(default_factory=dict)
    warnings: list[str] = Field(default_factory=list)


def router_entry(entries: list[DeploymentInput]) -> DeploymentInput:
    for entry in entries:
        if entry.litellm_params.quality_router_config is not None:
            return entry
    raise ProbeError("invalid_request", "no deployment declares quality_router_config")


# the Router validates its own deployments, so each entry carries only the keys the quality router reads
def deployment_entry(entry: DeploymentInput) -> dict[str, JsonValue]:
    info: dict[str, JsonValue] = {}
    if entry.model_info.litellm_routing_preferences is not None:
        info["litellm_routing_preferences"] = entry.model_info.litellm_routing_preferences
    if entry.model_info.input_cost_per_token is not None:
        info["input_cost_per_token"] = entry.model_info.input_cost_per_token
    return {
        "model_name": entry.model_name,
        "litellm_params": {"model": entry.litellm_params.model},
        "model_info": info,
    }


def model_list(entries: list[DeploymentInput], chosen: DeploymentInput) -> list[dict[str, JsonValue]]:
    return [deployment_entry(entry) for entry in entries if entry is not chosen]


class QualityRouter(Operation):
    config: Document | None = None
    # the document's JSON text, so a number literal reaches each runtime spelled as the case writes it
    config_text: str | None = None
    messages: list[dict[str, JsonValue]] | None = None
    default_model: str | None = None

    def document(self) -> Document:
        if self.config_text is not None:
            return Document.model_validate_json(self.config_text)
        if self.config is None:
            raise ProbeError("invalid_request", "no config document declared")
        return self.config

    @override
    def run(self) -> Output:
        entries = self.document().model_list
        chosen = router_entry(entries)
        router = Router(model_list=model_list(entries, chosen))
        configured = chosen.litellm_params.quality_router_config
        if configured is None:
            raise ProbeError("invalid_request", "no deployment declares quality_router_config")
        quality = LiteLLMQualityRouter(
            model_name=chosen.model_name,
            litellm_router_instance=router,
            default_model=self.default_model,
            quality_router_config=configured,
        )
        request_kwargs: dict[str, JsonValue] = {}
        try:
            response = pre_route(quality, chosen.model_name, request_kwargs, self.messages)
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        metadata = request_kwargs.get("metadata")
        carrier: dict[str, JsonValue] = {}
        additional = ensure_response_additional_headers(carrier)
        apply_quality_router_decision_headers(additional, {"metadata": metadata})
        headers = _HEADERS.validate_python(additional)
        if response is None:
            return Output(headers=headers)
        fields = _RESPONSE.validate_json(response.model_dump_json())
        return Output(
            model=fields["model"],
            decision=fields["routing_decision"],
            metadata=metadata,
            headers=headers,
        )
