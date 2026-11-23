from collections.abc import Mapping

from pydantic import JsonValue

from betterlite._cost.completion import completion_request
from betterlite._cost.engine import ENGINE, charge_answer
from betterlite._cost.helpers import get_response_cost_from_hidden_params
from betterlite._cost.logging import CostLogging, logging_model
from betterlite._cost.objects import hidden_params
from betterlite._cost.wire import JsonSource


# response_cost_calculator: a cache hit is free, a provider-stated cost wins, and the rest is completion_cost.
def response_cost_calculator(
    response_object: JsonSource,
    model: str,
    custom_llm_provider: str | None,
    call_type: str,
    optional_params: Mapping[str, JsonValue],
    cache_hit: bool | None = None,
    base_model: str | None = None,
    custom_pricing: bool | None = None,
    prompt: str = "",
    standard_built_in_tools_params: JsonValue = None,
    litellm_model_name: str | None = None,
    router_model_id: str | None = None,
    litellm_logging_obj: CostLogging | None = None,
    service_tier: str | None = None,
    data_residency: str | None = None,
    vertex_location: str | None = None,
    region_name: str | None = None,
) -> float:
    if cache_hit is True:
        return 0.0
    found = hidden_params(response_object)
    if found is not None:
        provider_cost = get_response_cost_from_hidden_params(found)
        if provider_cost is not None:
            return provider_cost
    request = completion_request(
        completion_response=response_object,
        model=model,
        prompt=prompt,
        messages=[],
        completion="",
        total_time=0.0,
        call_type=call_type,
        custom_llm_provider=custom_llm_provider,
        region_name=region_name,
        custom_cost_per_token=None,
        custom_cost_per_second=None,
        optional_params=optional_params,
        custom_pricing=custom_pricing,
        base_model=base_model,
        router_model_id=router_model_id,
        service_tier=service_tier,
        data_residency=data_residency,
        vertex_location=vertex_location,
        request_model=logging_model(litellm_logging_obj),
    )
    return charge_answer(request, litellm_logging_obj, ENGINE.response_cost)
