from decimal import Decimal
from typing import TypeAlias, cast

from litellm import cost_calculator
from litellm.cost_calculator import _batch_prompt_cost  # pyright: ignore[reportPrivateUsage]
from litellm.types.utils import (
    CacheCreationTokenDetails,
    ModelInfo,
    PromptTokensDetailsWrapper,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    # Cases keep every computed value to a short exact product, so repr carries the decimal digits.
    return format(Decimal(repr(value)).normalize(), "f")


def _dict_or_none(value: Json) -> dict[str, Json] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("expected an object or null")
    return value


def _optional_int(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int):
        raise ValueError(f"{name} must be a JSON integer or null")
    return value


def _tokens(value: Json, name: str) -> int:
    count = _optional_int(value, name)
    return 0 if count is None else count


def _rate(value: Json, name: str) -> float:
    if not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a JSON number")
    return float(value)


def _optional_rate(value: Json, name: str) -> float | None:
    return None if value is None else _rate(value, name)


def _cache_creation_details(value: Json) -> CacheCreationTokenDetails | None:
    fields = _dict_or_none(value)
    if fields is None:
        return None
    return CacheCreationTokenDetails.model_construct(
        ephemeral_5m_input_tokens=_optional_int(
            fields.get("ephemeral_5m_input_tokens"), "ephemeral_5m_input_tokens"
        ),
        ephemeral_1h_input_tokens=_optional_int(
            fields.get("ephemeral_1h_input_tokens"), "ephemeral_1h_input_tokens"
        ),
    )


def _prompt_details(
    usage_fields: dict[str, Json], extras: dict[str, Json]
) -> PromptTokensDetailsWrapper | None:
    details = _dict_or_none(usage_fields.get("prompt_details"))
    if details is None and not extras and usage_fields.get("cache_read_tokens") is None:
        return None
    fields = {} if details is None else details
    return PromptTokensDetailsWrapper.model_construct(
        text_tokens=_optional_int(fields.get("text_tokens"), "text_tokens"),
        audio_tokens=_optional_int(fields.get("audio_tokens"), "audio_tokens"),
        image_tokens=_optional_int(fields.get("image_tokens"), "image_tokens"),
        video_tokens=_optional_int(fields.get("video_tokens"), "video_tokens"),
        cached_tokens=_optional_int(usage_fields.get("cache_read_tokens"), "cache_read_tokens"),
        cache_write_tokens=_optional_int(extras.get("cache_write_tokens"), "cache_write_tokens"),
        cache_creation_tokens=_optional_int(
            extras.get("cache_creation_tokens"), "cache_creation_tokens"
        ),
        cache_creation_token_details=_cache_creation_details(
            extras.get("cache_creation_token_details")
        ),
        cached_tokens_details=_dict_or_none(extras.get("cached_tokens_details")),
        character_count=_optional_int(extras.get("character_count"), "character_count"),
        image_count=_optional_int(extras.get("image_count"), "image_count"),
        video_length_seconds=_optional_rate(
            extras.get("video_length_seconds"), "video_length_seconds"
        ),
        audio_length_seconds=_optional_rate(
            extras.get("audio_length_seconds"), "audio_length_seconds"
        ),
        query_count=_optional_int(extras.get("query_count"), "query_count"),
    )


def _usage(input_value: dict[str, Json]) -> Usage:
    usage_fields = _dict_or_none(input_value.get("usage"))
    if usage_fields is None:
        raise ValueError("usage must be an object")
    extras = _dict_or_none(input_value.get("prompt_extras"))
    extras_fields = {} if extras is None else extras
    return Usage.model_construct(
        prompt_tokens=_tokens(usage_fields.get("prompt_tokens"), "prompt_tokens"),
        completion_tokens=_tokens(usage_fields.get("completion_tokens"), "completion_tokens"),
        total_tokens=0,
        prompt_tokens_details=_prompt_details(usage_fields, extras_fields),
        completion_tokens_details=None,
    )


def _run_calculator(input_value: dict[str, Json]) -> Json:
    prompt_cost, completion_cost = cost_calculator.batch_cost_calculator(
        usage=_usage(input_value),
        model=cast(str, input_value["model"]),
        custom_llm_provider=cast("str | None", input_value.get("custom_llm_provider")),
        model_info=cast("ModelInfo | None", _dict_or_none(input_value.get("model_info"))),
        data_residency=cast("str | None", input_value.get("data_residency")),
    )
    return {
        "prompt_cost": _decimal_str(prompt_cost),
        "completion_cost": _decimal_str(completion_cost),
    }


def _run_prompt(input_value: dict[str, Json]) -> Json:
    model_info = _dict_or_none(input_value.get("model_info"))
    if model_info is None:
        raise ValueError("model_info must be an object")
    prompt_cost = _batch_prompt_cost(
        usage=_usage(input_value),
        model_info=cast(ModelInfo, model_info),
        input_rate=_rate(input_value["input_rate"], "input_rate"),
        cache_read_rate=_optional_rate(input_value.get("cache_read_rate"), "cache_read_rate"),
        cache_creation_rate=_optional_rate(
            input_value.get("cache_creation_rate"), "cache_creation_rate"
        ),
    )
    return {"prompt_cost": _decimal_str(prompt_cost)}


def run(input_value: dict[str, Json]) -> Json:
    kind = input_value.get("kind")
    if kind == "calculator":
        return _run_calculator(input_value)
    if kind == "prompt":
        return _run_prompt(input_value)
    raise ValueError(f"unknown kind: {kind!r}")
