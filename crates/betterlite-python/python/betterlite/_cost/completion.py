from collections.abc import Mapping, Sequence

from pydantic import JsonValue

from betterlite._cost.cost_logging import CostLogging, logging_model
from betterlite._cost.engine import ENGINE, charge_answer
from betterlite._cost.objects import dump_object, hidden_params, response_kind, source_value
from betterlite._cost.wire import (
    CompletionRequestWire,
    JsonSource,
    ResponseWire,
    number_value,
    object_value,
    text_value,
)


def completion_request(
    completion_response: JsonSource | None,
    model: str | None,
    prompt: str,
    messages: Sequence[JsonSource],
    completion: str,
    total_time: float | None,
    call_type: str | None,
    custom_llm_provider: str | None,
    region_name: str | None,
    custom_cost_per_token: JsonSource | None,
    custom_cost_per_second: float | None,
    optional_params: Mapping[str, JsonValue] | None,
    custom_pricing: bool | None,
    base_model: str | None,
    router_model_id: str | None,
    service_tier: str | None,
    data_residency: str | None,
    vertex_location: str | None,
    request_model: str | None,
) -> CompletionRequestWire:
    response: ResponseWire | None = None
    if completion_response is not None:
        dumped = dump_object(completion_response)
        response = ResponseWire(
            kind=response_kind(completion_response),
            model=text_value(dumped.get("model")),
            usage=object_value(dumped.get("usage")),
            service_tier=text_value(dumped.get("service_tier")),
            hidden_params=hidden_params(completion_response),
            created=number_value(dumped.get("created")),
            ended=number_value(dumped.get("ended")),
        )
    return CompletionRequestWire(
        model=model,
        prompt=prompt,
        messages=[dump_object(message) for message in messages],
        completion=completion,
        total_time=total_time,
        call_type=call_type,
        custom_llm_provider=custom_llm_provider,
        region_name=region_name,
        custom_cost_per_token=source_value(custom_cost_per_token),
        custom_cost_per_second=custom_cost_per_second,
        optional_params=None if optional_params is None else dict(optional_params),
        custom_pricing=custom_pricing,
        base_model=base_model,
        router_model_id=router_model_id,
        service_tier=service_tier,
        data_residency=data_residency,
        vertex_location=vertex_location,
        request_model=request_model,
        response=response,
    )


# completion_cost: the response's usage priced by B068, with LiteLLM's parameter list unchanged.
def completion_cost(
    completion_response: JsonSource | None = None,
    model: str | None = None,
    prompt: str = "",
    messages: list[JsonSource] = [],
    completion: str = "",
    total_time: float | None = 0.0,
    call_type: str | None = None,
    custom_llm_provider: str | None = None,
    region_name: str | None = None,
    size: str | None = None,
    quality: str | None = None,
    n: int | None = None,
    custom_cost_per_token: JsonSource | None = None,
    custom_cost_per_second: float | None = None,
    optional_params: Mapping[str, JsonValue] | None = None,
    custom_pricing: bool | None = None,
    base_model: str | None = None,
    standard_built_in_tools_params: JsonValue = None,
    litellm_model_name: str | None = None,
    router_model_id: str | None = None,
    litellm_logging_obj: CostLogging | None = None,
    service_tier: str | None = None,
    data_residency: str | None = None,
    vertex_location: str | None = None,
) -> float:
    request = completion_request(
        completion_response=completion_response,
        model=model,
        prompt=prompt,
        messages=messages,
        completion=completion,
        total_time=total_time,
        call_type=call_type,
        custom_llm_provider=custom_llm_provider,
        region_name=region_name,
        custom_cost_per_token=custom_cost_per_token,
        custom_cost_per_second=custom_cost_per_second,
        optional_params=optional_params,
        custom_pricing=custom_pricing,
        base_model=base_model,
        router_model_id=router_model_id,
        service_tier=service_tier,
        data_residency=data_residency,
        vertex_location=vertex_location,
        request_model=logging_model(litellm_logging_obj),
    )
    return charge_answer(request, litellm_logging_obj, ENGINE.completion_cost)
