from datetime import UTC, datetime
from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.utils import (
    apply_provider_cache_read_default,
    generic_cost_per_token,
    get_provider_specific_geo_multiplier,
)
from litellm.types.utils import (
    CacheCreationTokenDetails,
    CompletionTokensDetailsWrapper,
    ModelInfo,
    PromptTokensDetailsWrapper,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _current_time(input_value: dict[str, Json]) -> datetime:
    moment = input_value.get("moment")
    return _EPOCH if moment is None else datetime.fromisoformat(str(moment))


def _cache_creation_token_details(value: Json) -> CacheCreationTokenDetails | None:
    if value is None:
        return None
    fields = cast("dict[str, Json]", value)
    return CacheCreationTokenDetails.model_construct(
        ephemeral_5m_input_tokens=cast("int | None", fields.get("ephemeral_5m_input_tokens")),
        ephemeral_1h_input_tokens=cast("int | None", fields.get("ephemeral_1h_input_tokens")),
    )


def _prompt_tokens_details(
    usage_fields: dict[str, Json], prompt_extras: dict[str, Json]
) -> PromptTokensDetailsWrapper | None:
    prompt_details = usage_fields.get("prompt_details")
    cache_read_tokens = usage_fields.get("cache_read_tokens")
    if prompt_details is None and not prompt_extras and cache_read_tokens is None:
        return None
    details = cast("dict[str, Json]", prompt_details or {})
    return PromptTokensDetailsWrapper.model_construct(
        text_tokens=cast("int | None", details.get("text_tokens")),
        audio_tokens=cast("int | None", details.get("audio_tokens")),
        image_tokens=cast("int | None", details.get("image_tokens")),
        video_tokens=cast("int | None", details.get("video_tokens")),
        cached_tokens=cast("int | None", cache_read_tokens),
        cache_write_tokens=cast("int | None", prompt_extras.get("cache_write_tokens")),
        cache_creation_tokens=cast("int | None", prompt_extras.get("cache_creation_tokens")),
        cache_creation_token_details=_cache_creation_token_details(prompt_extras.get("cache_creation_token_details")),
        cached_tokens_details=prompt_extras.get("cached_tokens_details"),
        character_count=cast("int | None", prompt_extras.get("character_count")),
        image_count=cast("int | None", prompt_extras.get("image_count")),
        video_length_seconds=cast("float | None", prompt_extras.get("video_length_seconds")),
        audio_length_seconds=cast("float | None", prompt_extras.get("audio_length_seconds")),
        query_count=cast("int | None", prompt_extras.get("query_count")),
    )


def _completion_tokens_details(
    usage_fields: dict[str, Json],
) -> CompletionTokensDetailsWrapper | None:
    completion_details = usage_fields.get("completion_details")
    if completion_details is None:
        return None
    fields = cast("dict[str, Json]", completion_details)
    return CompletionTokensDetailsWrapper.model_construct(
        text_tokens=cast("int | None", fields.get("text_tokens")),
        audio_tokens=cast("int | None", fields.get("audio_tokens")),
        reasoning_tokens=cast("int | None", fields.get("reasoning_tokens")),
        image_tokens=cast("int | None", fields.get("image_tokens")),
        video_tokens=cast("int | None", fields.get("video_tokens")),
    )


def _generic_usage(input_value: dict[str, Json]) -> Usage:
    usage_fields = cast("dict[str, Json]", input_value.get("usage", {}))
    prompt_extras = cast("dict[str, Json]", input_value.get("prompt_extras") or {})
    return Usage.model_construct(
        prompt_tokens=cast(int, usage_fields.get("prompt_tokens", 0)),
        completion_tokens=cast(int, usage_fields.get("completion_tokens", 0)),
        total_tokens=cast(int, usage_fields.get("total_tokens", 0)),
        prompt_tokens_details=_prompt_tokens_details(usage_fields, prompt_extras),
        completion_tokens_details=_completion_tokens_details(usage_fields),
    )


def _generic_model_info(input_value: dict[str, Json]) -> ModelInfo | None:
    base = input_value.get("supplied_model_info")
    if base is None:
        return None
    model_info = dict(cast("dict[str, Json]", base))
    tiered_pricing = input_value.get("tiered_pricing")
    if tiered_pricing:
        model_info["tiered_pricing"] = tiered_pricing
    off_peak_pricing = input_value.get("off_peak_pricing")
    if off_peak_pricing is not None:
        model_info["off_peak_pricing"] = off_peak_pricing
    return cast(ModelInfo, model_info)


def _run_generic(input_value: dict[str, Json]) -> Json:
    prompt_cost, completion_cost = generic_cost_per_token(
        model="oracle-model",
        usage=_generic_usage(input_value),
        custom_llm_provider=cast(str, input_value.get("custom_llm_provider") or ""),
        service_tier=cast("str | None", input_value.get("service_tier")),
        data_residency=cast("str | None", input_value.get("data_residency")),
        model_info=_generic_model_info(input_value),
        vertex_location=cast("str | None", input_value.get("vertex_location")),
        current_time=_current_time(input_value),
    )
    return {
        "prompt_cost": _decimal_str(prompt_cost),
        "completion_cost": _decimal_str(completion_cost),
    }


def _run_provider_geo(input_value: dict[str, Json]) -> Json:
    model_info = {"provider_specific_entry": input_value.get("provider_specific_entry")}
    usage = Usage.model_construct(prompt_tokens=0, completion_tokens=0, total_tokens=0)
    inference_geo = input_value.get("inference_geo")
    if inference_geo is not None:
        setattr(usage, "inference_geo", inference_geo)  # noqa: B010 - Usage allows extra attributes
    multiplier = get_provider_specific_geo_multiplier(cast(ModelInfo, model_info), usage)
    return {"multiplier": _decimal_str(multiplier)}


def _run_fireworks_cache_default(input_value: dict[str, Json]) -> Json:
    model_info = dict(cast("dict[str, Json]", input_value["model_info"]))
    off_peak_pricing = input_value.get("off_peak_pricing")
    if off_peak_pricing is not None:
        model_info["off_peak_pricing"] = off_peak_pricing
    result = apply_provider_cache_read_default(
        cast(ModelInfo, model_info), cast("str | None", input_value.get("custom_llm_provider"))
    )
    off_peak_result = cast("dict[str, Json] | None", result.get("off_peak_pricing"))
    return {
        "cache_read_input_token_cost": result.get("cache_read_input_token_cost"),
        "off_peak_cache_read_input_token_cost": (
            None if off_peak_result is None else off_peak_result.get("cache_read_input_token_cost")
        ),
    }


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    if operation == "generic":
        return _run_generic(input_value)
    if operation == "provider_geo":
        return _run_provider_geo(input_value)
    if operation == "fireworks_cache_default":
        return _run_fireworks_cache_default(input_value)
    raise ValueError(f"unknown token_cost operation: {operation}")
