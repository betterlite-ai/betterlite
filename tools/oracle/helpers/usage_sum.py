from typing import TypeAlias, cast

from litellm.cost_calculator import BaseTokenUsageProcessor
from litellm.types.utils import CachedTokensDetails, PromptTokensDetailsWrapper, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROMPT_DETAIL_KEYS = (
    "text_tokens", "audio_tokens", "image_tokens", "video_tokens",
    "cache_write_tokens", "cache_creation_tokens", "cached_tokens",
)
_CACHED_DETAIL_KEYS = ("text_tokens", "audio_tokens", "image_tokens")
_COMPLETION_DETAIL_KEYS = (
    "text_tokens", "reasoning_tokens", "accepted_prediction_tokens", "rejected_prediction_tokens",
)


def _build_usage(item: dict[str, Json]) -> Usage:
    prompt_tokens_details = cast("dict[str, Json] | None", item.get("prompt_tokens_details"))
    completion_tokens_details = cast(
        "dict[str, Json] | None", item.get("completion_tokens_details")
    )
    usage = Usage(
        prompt_tokens=cast("int | None", item.get("prompt_tokens")),
        completion_tokens=cast("int | None", item.get("completion_tokens")),
        total_tokens=cast("int | None", item.get("total_tokens")),
        prompt_tokens_details=prompt_tokens_details,
        completion_tokens_details=completion_tokens_details,
    )
    extras = item.get("numeric_extras")
    if isinstance(extras, dict):
        for key, value in extras.items():
            setattr(usage, key, value)
    return usage


def _details(model: object, keys: tuple[str, ...]) -> dict[str, Json]:
    return {key: getattr(model, key, None) for key in keys}


def _cached_details(details: PromptTokensDetailsWrapper) -> Json:
    cached: CachedTokensDetails | None = getattr(details, "cached_tokens_details", None)
    if cached is None:
        return None
    return _details(cached, _CACHED_DETAIL_KEYS)


def run(input_value: dict[str, Json]) -> Json:
    usages_raw = cast("list[dict[str, Json]]", input_value.get("usages", []))
    usages = [_build_usage(item) for item in usages_raw]
    combined = BaseTokenUsageProcessor.combine_usage_objects(usages)
    prompt_details = combined.prompt_tokens_details
    prompt_json: Json = None
    if prompt_details is not None:
        prompt_json = {
            **_details(prompt_details, _PROMPT_DETAIL_KEYS),
            "cached_tokens_details": _cached_details(prompt_details),
        }
    completion_details = combined.completion_tokens_details
    completion_json: Json = None
    if completion_details is not None:
        completion_json = _details(completion_details, _COMPLETION_DETAIL_KEYS)
    return {
        "prompt_tokens": combined.prompt_tokens,
        "completion_tokens": combined.completion_tokens,
        "total_tokens": combined.total_tokens,
        "prompt_tokens_details": prompt_json,
        "completion_tokens_details": completion_json,
        "numeric_extras": dict(combined.model_extra or {}),
    }
