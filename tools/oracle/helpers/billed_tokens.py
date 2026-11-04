from dataclasses import asdict
from datetime import UTC, datetime
from decimal import Decimal
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm import cost_calculator
from litellm.litellm_core_utils.llm_cost_calc import utils
from litellm.types.utils import (
    CacheCreationTokenDetails,
    CachedTokensDetails,
    CompletionTokensDetailsWrapper,
    CostPerToken,
    ModelInfo,
    PromptTokensDetailsWrapper,
    Usage,
)

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _strings(value: object) -> Json:
    if isinstance(value, (int, float, Decimal)):
        return format(Decimal(str(value)).normalize(), "f")
    if isinstance(value, dict):
        return {str(key): _strings(item) for key, item in cast(dict[object, object], value).items()}
    return cast(Json, value)


def _usage(input_value: dict[str, Json]) -> Usage:
    fields = cast(dict[str, int | None], input_value.get("usage", {}))
    usage = Usage(prompt_tokens=cast(int, input_value.get("prompt_tokens", 0)))
    details = None
    if "detail_cache_creation_5m" in fields or "detail_cache_creation_1h" in fields:
        details = CacheCreationTokenDetails(
            ephemeral_5m_input_tokens=fields.get("detail_cache_creation_5m"),
            ephemeral_1h_input_tokens=fields.get("detail_cache_creation_1h"),
        )
    prompt = PromptTokensDetailsWrapper(
        cached_tokens=fields.get("detail_cache_read"),
        cache_creation_tokens=fields.get("detail_cache_creation"),
        cache_creation_token_details=details,
    )
    prompt.cached_tokens_details = CachedTokensDetails(
        audio_tokens=fields.get("detail_cached_audio", 0)
    )
    usage.prompt_tokens_details = prompt
    usage.completion_tokens_details = CompletionTokensDetailsWrapper(
        reasoning_tokens=fields.get("detail_reasoning")
    )
    extras = {
        "reasoning_tokens": fields.get("top_level_reasoning", 0),
        "_cache_read_input_tokens": fields.get("top_level_cache_read", 0),
        "_cache_creation_input_tokens": fields.get("top_level_cache_creation", 0),
        "inference_geo": input_value.get("inference_geo"),
    }
    for key, value in extras.items():
        setattr(usage, key, value)
    return usage


def _custom_cost(input_value: dict[str, Json]) -> Json:
    helper_name = "_cost_per_token_custom_pricing_helper"
    calculator = cast(
        "Callable[..., tuple[float, float] | None]",
        getattr(cost_calculator, helper_name),
    )
    result = calculator(**{key: value for key, value in input_value.items() if key != "operation"})
    if result is None:
        return None
    return {
        "input_cost": str(_strings(float(result[0]))),
        "output_cost": str(_strings(float(result[1]))),
    }


def _counts(usage: Usage) -> Json:
    cache_name = "_cache_token_counts"
    reasoning_name = "_reasoning_token_count"
    cache = cast(
        "Callable[[Usage], tuple[int, int, int, CacheCreationTokenDetails | None]]",
        getattr(utils, cache_name),
    )(usage)
    reasoning = cast("Callable[[Usage], int]", getattr(utils, reasoning_name))(usage)
    return {
        "reasoning": reasoning,
        "cache_read": cache[0],
        "cached_audio": cache[1],
        "cache_creation": cache[2],
        "cache_creation_5m": None if cache[3] is None else cache[3].ephemeral_5m_input_tokens,
        "cache_creation_1h": None if cache[3] is None else cache[3].ephemeral_1h_input_tokens,
    }


def _catalog(input_value: dict[str, Json], usage: Usage) -> Json:
    model = dict(cast(dict[str, Json], input_value.get("model_info", {})))
    for key in ("tiered_pricing", "off_peak_pricing"):
        if key in input_value:
            model[key] = input_value[key]
    provider = cast(str | None, input_value.get("provider"))
    tier = cast(str | None, input_value.get("service_tier"))
    residency = cast(str | None, input_value.get("residency"))
    vertex = cast(str | None, input_value.get("vertex_location"))
    moment = (
        datetime.fromisoformat(str(input_value["moment"]))
        if "moment" in input_value
        else datetime(1970, 1, 1, tzinfo=UTC)
    )
    savings = utils.calculate_prompt_caching_savings(
        cast(ModelInfo, model), usage, provider, tier, residency, vertex, moment
    )
    if input_value["operation"] == "savings":
        return _strings(float(savings))
    defaulted = utils.apply_provider_cache_read_default(cast(ModelInfo, model), provider)
    rates_name = "_cost_map_billed_rates"
    rates = cast("Callable[..., utils.BilledTokenRates]", getattr(utils, rates_name))(
        defaulted, usage, provider, tier, residency, vertex, moment
    )
    counts = cast(dict[str, int], _counts(usage))
    details_name = "cache_creation_token_details"
    details = cast(
        CacheCreationTokenDetails | None, getattr(usage.prompt_tokens_details, details_name, None)
    )
    result = utils.TokenTypeCostBreakdown(
        counts["reasoning"] * rates.output_cost_per_reasoning_token,
        (counts["cache_read"] - counts["cached_audio"]) * rates.cache_read_input_token_cost
        + counts["cached_audio"] * rates.cache_read_input_audio_token_cost,
        utils.calculate_cache_writing_cost(
            counts["cache_creation"],
            details,
            rates.cache_creation_input_token_cost_above_1hr,
            rates.cache_creation_input_token_cost,
        ),
        rates,
    )
    return {"breakdown": _strings(asdict(result)), "savings": _strings(float(savings))}


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm.__name__
    operation = input_value.get("operation", "custom_cost")
    if operation == "custom_cost":
        return _custom_cost(input_value)
    usage = _usage(input_value)
    if operation == "counts":
        return _counts(usage)
    if operation == "custom_breakdown":
        result = utils.get_token_type_cost_breakdown(
            model="oracle",
            custom_llm_provider=None,
            usage=usage,
            custom_cost_per_token=cast(CostPerToken, input_value["custom_cost_per_token"]),
        )
        return _strings(asdict(result))
    return _catalog(input_value, usage)
