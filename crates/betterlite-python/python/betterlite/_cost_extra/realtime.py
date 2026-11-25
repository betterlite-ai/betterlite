import json
from collections.abc import Mapping, Sequence

from pydantic import JsonValue

from betterlite._cost.cost_logging import CostLogging, store_breakdown
from betterlite._cost.wire import CostAnswerWire
from betterlite._cost_extra.wire import (
    ENGINE,
    FLOAT,
    REALTIME_COST,
    RealtimeCostRequestWire,
    RealtimeTranscriptionWire,
    native_answer,
    usage_wire,
)
from betterlite.types.utils.usage import Usage


def handle_realtime_stream_cost_calculation(  # ast-grep-ignore: no-long-names  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2984
    results: Sequence[Mapping[str, JsonValue]],
    combined_usage_object: Usage,
    custom_llm_provider: str,
    litellm_model_name: str,
    data_residency: str | None = None,
    litellm_logging_obj: CostLogging | None = None,
    custom_pricing_model: str | None = None,
    base_pricing_model: str | None = None,
) -> float:
    request: RealtimeCostRequestWire = {
        "results": [dict(result) for result in results],
        "combined_usage": usage_wire(combined_usage_object),
        "custom_llm_provider": custom_llm_provider,
        "litellm_model_name": litellm_model_name,
        "data_residency": data_residency,
        "custom_pricing_model": custom_pricing_model,
        "base_pricing_model": base_pricing_model,
    }
    answer = native_answer(REALTIME_COST, ENGINE.realtime_cost, json.dumps(request))
    store_breakdown(
        litellm_logging_obj,
        CostAnswerWire(cost=answer[0], breakdown=answer[1], billed_token_rates=None),
    )
    return answer[0]


def handle_realtime_transcription_cost_calculation(  # ast-grep-ignore: no-long-names
    results: Sequence[Mapping[str, JsonValue]],
    custom_llm_provider: str,
    litellm_model_name: str,
    custom_pricing_model: str | None = None,
) -> float:
    request: RealtimeTranscriptionWire = {
        "results": [dict(result) for result in results],
        "custom_llm_provider": custom_llm_provider,
        "litellm_model_name": litellm_model_name,
        "custom_pricing_model": custom_pricing_model,
    }
    return native_answer(FLOAT, ENGINE.realtime_transcription_cost, json.dumps(request))
