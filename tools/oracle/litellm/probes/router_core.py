import asyncio
import os
from typing import ClassVar, Literal, Protocol

from litellm.exceptions import BadRequestError, ServiceUnavailableError
from litellm.router import Router
from litellm.router_utils.add_retry_fallback_headers import (
    apply_response_model_id,
    ensure_response_additional_headers,
    get_hidden_params_dict,
    prepare_response_for_header_attachment,
    response_has_hidden_params,
)
from litellm.types.router import RouterRateLimitError
from pydantic import ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

API_KEY = "sk-probe"
ENVIRONMENT = "production"
Carrier = Literal["dict", "attributed", "stream"]
Outcome = Literal["selected", "bad_request", "rate_limited", "service_unavailable"]
_HIDDEN = TypeAdapter(dict[str, JsonValue])


class Entry(Strict):
    model_name: str
    model: str
    id: str
    weight: int | None = None


class Route(Strict):
    model: str
    excluded_deployment_ids: list[str] = Field(default_factory=list)


class Scenario(Strict):
    carrier: Carrier
    hidden: dict[str, JsonValue] | None = None
    metadata: dict[str, JsonValue] | None = None


class Input(Strict):
    models: list[Entry]
    routes: list[Route] = Field(default_factory=list)
    responses: list[Scenario] = Field(default_factory=list)


class Selected(Strict):
    model_name: str
    model: str
    id: str


class Observed(Strict):
    error_type: str
    model: str
    cooldown_time: str


class Found(Strict):
    outcome: Outcome
    selected: Selected | None
    error: Observed | None


class ResponseFound(Strict):
    prepared: bool
    has_hidden: bool
    hidden: dict[str, JsonValue]


class Output(Strict):
    routes: list[Found]
    responses: list[ResponseFound]


class Returned(Strict):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


class RouterView(Protocol):
    async def async_get_available_deployment(
        self, model: str, request_kwargs: dict[str, object], messages: None, responses_input: None, /
    ) -> dict[str, JsonValue]: ...


class HiddenResponse:
    def __init__(self, hidden: dict[str, JsonValue]) -> None:
        self._hidden_params: dict[str, JsonValue] = hidden


class BareStream:
    def __anext__(self) -> JsonValue:
        raise StopAsyncIteration


def model_list_entry(entry: Entry) -> dict[str, object]:
    params: dict[str, object] = {"model": entry.model, "api_key": API_KEY}
    if entry.weight is not None:
        params["weight"] = entry.weight
    return {"model_name": entry.model_name, "litellm_params": params, "model_info": {"id": entry.id}}


def selected(deployment: dict[str, JsonValue]) -> Selected:
    returned = TypeAdapter(Returned).validate_python(deployment)
    model = returned.litellm_params.get("model")
    if not isinstance(model, str):
        raise TypeError("the selected deployment has no model")
    identifier = returned.model_info.get("id")
    if not isinstance(identifier, str):
        raise TypeError("the selected deployment has no string id")
    return Selected(model_name=returned.model_name, model=model, id=identifier)


def observed_error(error: RouterRateLimitError) -> Observed:
    return Observed(
        error_type=error.type,
        model=error.model,
        cooldown_time=str(float(error.cooldown_time)),
    )


def carrier_response(scenario: Scenario) -> object:
    match scenario.carrier:
        case "dict":
            body: dict[str, JsonValue] = {}
            if scenario.hidden is not None:
                body["_hidden_params"] = scenario.hidden
            return body
        case "attributed":
            return HiddenResponse(dict(scenario.hidden or {}))
        case "stream":
            return BareStream()


def observe(scenario: Scenario) -> ResponseFound:
    prepared = prepare_response_for_header_attachment(carrier_response(scenario))
    if prepared is None:
        return ResponseFound(prepared=False, has_hidden=False, hidden={})
    _ = ensure_response_additional_headers(prepared)
    apply_response_model_id(prepared, scenario.metadata)
    has_hidden = response_has_hidden_params(prepared)
    hidden: dict[str, JsonValue] = _HIDDEN.validate_python(get_hidden_params_dict(prepared)) if has_hidden else {}
    return ResponseFound(prepared=True, has_hidden=has_hidden, hidden=hidden)


class RouterCore(Operation):
    models: list[Entry]
    routes: list[Route] = Field(default_factory=list)
    responses: list[Scenario] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        os.environ["LITELLM_ENVIRONMENT"] = ENVIRONMENT
        router = Router(
            model_list=[model_list_entry(entry) for entry in self.models],
            routing_strategy="simple-shuffle",
        )
        routes = [await self.answer(router, route) for route in self.routes]
        return Output(routes=routes, responses=[observe(scenario) for scenario in self.responses])

    async def answer(self, router: RouterView, route: Route) -> Found:
        request_kwargs: dict[str, object] = {}
        if route.excluded_deployment_ids:
            request_kwargs["_excluded_deployment_ids"] = route.excluded_deployment_ids
        try:
            deployment = await router.async_get_available_deployment(route.model, request_kwargs, None, None)
        except ServiceUnavailableError:
            return Found(outcome="service_unavailable", selected=None, error=None)
        except RouterRateLimitError as error:
            return Found(outcome="rate_limited", selected=None, error=observed_error(error))
        except BadRequestError:
            return Found(outcome="bad_request", selected=None, error=None)
        return Found(outcome="selected", selected=selected(deployment), error=None)
