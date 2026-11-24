from pydantic import JsonValue

from betterlite._cost.engine import price_pair
from betterlite._cost.objects import source_value
from betterlite._cost.wire import CallRequestWire, JsonSource


# cost_per_token: the prompt and completion priced apart, as LiteLLM's signature counts them.
def cost_per_token(
    model: str = "",
    prompt_tokens: int = 0,
    completion_tokens: int = 0,
    response_time_ms: float | None = 0.0,
    custom_llm_provider: str | None = None,
    region_name: str | None = None,
    prompt_characters: int | None = None,
    completion_characters: int | None = None,
    cache_creation_input_tokens: int | None = 0,
    cache_read_input_tokens: int | None = 0,
    custom_cost_per_token: JsonSource | None = None,
    custom_cost_per_second: float | None = None,
    number_of_queries: int | None = None,
    usage_object: JsonSource | None = None,
    rerank_billed_units: JsonSource | None = None,
    call_type: str = "completion",
    audio_transcription_file_duration: float = 0.0,  # ast-grep-ignore: no-long-names
    service_tier: str | None = None,
    data_residency: str | None = None,
    vertex_location: str | None = None,
    response: JsonValue = None,
    request_model: str | None = None,
    custom_model_info: JsonValue = None,
) -> tuple[float, float]:
    request = CallRequestWire(
        model=model,
        prompt_tokens=prompt_tokens,
        completion_tokens=completion_tokens,
        response_time_ms=response_time_ms,
        custom_llm_provider=custom_llm_provider,
        region_name=region_name,
        prompt_characters=prompt_characters,
        completion_characters=completion_characters,
        cache_creation_input_tokens=cache_creation_input_tokens,
        cache_read_input_tokens=cache_read_input_tokens,
        custom_cost_per_token=source_value(custom_cost_per_token),
        custom_cost_per_second=custom_cost_per_second,
        number_of_queries=number_of_queries,
        usage=source_value(usage_object),
        rerank_billed_units=source_value(rerank_billed_units),
        call_type=call_type,
        # ast-grep-ignore: no-long-names
        audio_seconds=audio_transcription_file_duration,
        service_tier=service_tier,
        data_residency=data_residency,
        vertex_location=vertex_location,
        request_model=request_model,
    )
    return price_pair(request)
