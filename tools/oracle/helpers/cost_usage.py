from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.usage_object_transformation import (
    InteractionsUsageObjectTransformation,
    TranscriptionUsageObjectTransformation,
)
from litellm.types.utils import (
    TranscriptionUsageDurationObject,
    TranscriptionUsageTokensObject,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROMPT_DETAIL_KEYS = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "web_search_requests",
)
_COMPLETION_DETAIL_KEYS = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "reasoning_tokens",
)


class ModalityEntryNotObject(ValueError):  # noqa: N818 - name must match betterlite's error class exactly
    pass


def _transcription_usage_object(
    item: dict[str, Json],
) -> TranscriptionUsageDurationObject | TranscriptionUsageTokensObject:
    kind = item.get("type")
    if kind == "duration":
        return TranscriptionUsageDurationObject.model_validate(item)
    if kind == "tokens":
        return TranscriptionUsageTokensObject.model_validate(item)
    raise ValueError(f"unknown transcription usage type {kind!r}")


def _details(details: dict[str, Json] | None, keys: tuple[str, ...]) -> Json:
    if details is None:
        return None
    return {key: details.get(key) for key in keys}


def _usage_to_json(usage: Usage | None) -> Json:
    if usage is None:
        return None
    dump = usage.model_dump()
    cache_read = getattr(usage, "_cache_read_input_tokens", 0) or None
    return {
        "prompt_tokens": dump["prompt_tokens"],
        "completion_tokens": dump["completion_tokens"],
        "total_tokens": dump["total_tokens"],
        "prompt_details": _details(dump.get("prompt_tokens_details"), _PROMPT_DETAIL_KEYS),
        "completion_details": _details(
            dump.get("completion_tokens_details"), _COMPLETION_DETAIL_KEYS
        ),
        "cache_read_tokens": cache_read,
    }


def _interactions_result(raw: Json) -> Json:
    if not InteractionsUsageObjectTransformation.is_interactions_usage_object(raw):
        return {"recognized": False, "usage": None}
    try:
        usage = InteractionsUsageObjectTransformation.transform_interactions_usage_object(
            cast(dict[str, Json], raw)
        )
    except AttributeError as error:
        raise ModalityEntryNotObject(str(error)) from error
    return {"recognized": True, "usage": _usage_to_json(usage)}


def run(input_value: dict[str, Json]) -> Json:
    transcription = input_value.get("transcription", [])
    interactions = input_value.get("interactions", [])
    if not isinstance(transcription, list) or not isinstance(interactions, list):
        raise ValueError("transcription and interactions must be arrays")
    transcription_results = [
        _usage_to_json(
            TranscriptionUsageObjectTransformation.transform_transcription_usage_object(
                _transcription_usage_object(cast(dict[str, Json], item))
            )
        )
        for item in transcription
    ]
    interactions_results = [_interactions_result(item) for item in interactions]
    return {"transcription": transcription_results, "interactions": interactions_results}
