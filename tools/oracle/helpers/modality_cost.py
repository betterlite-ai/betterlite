from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.utils import (
    _calculate_input_cost,  # pyright: ignore[reportPrivateUsage]
    get_billable_input_tokens,
    parse_completion_tokens_details,
    parse_prompt_tokens_details,
)
from litellm.types.utils import (
    CacheCreationTokenDetails,
    CompletionTokensDetailsWrapper,
    ModelInfo,
    PromptTokensDetailsWrapper,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _cache_creation_token_details(value: Json) -> CacheCreationTokenDetails | None:
    if value is None:
        return None
    ttl = cast("dict[str, Json]", value)
    return CacheCreationTokenDetails.model_construct(
        ephemeral_5m_input_tokens=cast("int | None", ttl.get("ephemeral_5m_input_tokens")),
        ephemeral_1h_input_tokens=cast("int | None", ttl.get("ephemeral_1h_input_tokens")),
    )


def _prompt_usage(fields: dict[str, Json]) -> Usage:
    details = PromptTokensDetailsWrapper.model_construct(
        text_tokens=cast("int | None", fields.get("text_tokens")),
        audio_tokens=cast("int | None", fields.get("audio_tokens")),
        image_tokens=cast("int | None", fields.get("image_tokens")),
        video_tokens=cast("int | None", fields.get("video_tokens")),
        cached_tokens=cast("int | None", fields.get("cached_tokens")),
        cache_write_tokens=cast("int | None", fields.get("cache_write_tokens")),
        cache_creation_tokens=cast("int | None", fields.get("cache_creation_tokens")),
        cache_creation_token_details=_cache_creation_token_details(fields.get("cache_creation_token_details")),
        cached_tokens_details=fields.get("cached_tokens_details"),
        character_count=cast("int | None", fields.get("character_count")),
        image_count=cast("int | None", fields.get("image_count")),
        video_length_seconds=cast("float | None", fields.get("video_length_seconds")),
        audio_length_seconds=cast("float | None", fields.get("audio_length_seconds")),
        query_count=cast("int | None", fields.get("query_count")),
    )
    return Usage.model_construct(
        prompt_tokens=cast(int, fields["prompt_tokens"]),
        completion_tokens=0,
        total_tokens=cast(int, fields["prompt_tokens"]),
        prompt_tokens_details=details,
        completion_tokens_details=None,
    )


def _run_prompt(fields: dict[str, Json]) -> Json:
    usage = _prompt_usage(fields)
    details = parse_prompt_tokens_details(usage)
    billable = get_billable_input_tokens(usage)
    model_info = cast(ModelInfo, fields.get("model_info", {}))
    cost = _calculate_input_cost(
        details,
        model_info,
        float(cast("float | int", fields["prompt_base_cost"])),
        float(cast("float | int", fields["cache_read_cost"])),
        float(cast("float | int", fields["cache_creation_cost"])),
        float(cast("float | int", fields["cache_creation_cost_above_1hr"])),
        cast("str | None", fields.get("service_tier")),
    )
    ttl = details["cache_creation_token_details"]
    return {
        "cache_hit_tokens": details["cache_hit_tokens"],
        "cache_hit_audio_tokens": details["cache_hit_audio_tokens"],
        "cache_creation_tokens": details["cache_creation_tokens"],
        "cache_creation_token_details": (
            None
            if ttl is None
            else {
                "ephemeral_5m_input_tokens": ttl.ephemeral_5m_input_tokens,
                "ephemeral_1h_input_tokens": ttl.ephemeral_1h_input_tokens,
            }
        ),
        "text_tokens": details["text_tokens"],
        "audio_tokens": details["audio_tokens"],
        "image_tokens": details["image_tokens"],
        "video_tokens": details["video_tokens"],
        "character_count": details["character_count"],
        "image_count": details["image_count"],
        "video_length_seconds": _decimal_str(details["video_length_seconds"]),
        "audio_length_seconds": _decimal_str(details["audio_length_seconds"]),
        "query_count": details["query_count"],
        "billable_input_tokens": billable,
        "input_cost": _decimal_str(cost),
    }


def _run_completion(fields: dict[str, Json]) -> Json:
    completion_details = CompletionTokensDetailsWrapper.model_construct(
        audio_tokens=cast("int | None", fields.get("audio_tokens")),
        text_tokens=cast("int | None", fields.get("text_tokens")),
        reasoning_tokens=cast("int | None", fields.get("reasoning_tokens")),
        image_tokens=cast("int | None", fields.get("image_tokens")),
        video_tokens=cast("int | None", fields.get("video_tokens")),
    )
    usage = Usage.model_construct(
        prompt_tokens=0,
        completion_tokens=cast(int, fields["completion_tokens"]),
        total_tokens=cast(int, fields["completion_tokens"]),
        prompt_tokens_details=None,
        completion_tokens_details=completion_details,
    )
    details = parse_completion_tokens_details(usage)
    return {
        "audio_tokens": details["audio_tokens"],
        "text_tokens": details["text_tokens"],
        "reasoning_tokens": details["reasoning_tokens"],
        "image_tokens": details["image_tokens"],
        "video_tokens": details["video_tokens"],
    }


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    if operation == "prompt":
        return _run_prompt(input_value)
    if operation == "completion":
        return _run_completion(input_value)
    raise ValueError(f"unknown modality_cost operation: {operation}")
