from collections.abc import Mapping, Sequence
from typing import TypeAlias, cast

from litellm.cost_calculator import (
    RealtimeAPITokenUsageProcessor,
    ResponsesWebSocketTokenUsageProcessor,
    _get_transcription_model_name_from_results,  # pyright: ignore[reportPrivateUsage]
    _split_responses_ws_logging_object_by_service_tier,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.llms.openai import OpenAIRealtimeStreamList
from litellm.types.utils import (
    CachedTokensDetails,
    LiteLLMRealtimeStreamLoggingObject,
    PromptTokensDetailsWrapper,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROMPT_FIELDS = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "cache_write_tokens",
    "cache_creation_tokens",
    "cached_tokens",
    "web_search_requests",
    "google_maps_grounding_requests",
    "tool_use_tokens",
    "character_count",
    "image_count",
    "video_length_seconds",
    "audio_length_seconds",
    "query_count",
)
_COMPLETION_FIELDS = (
    "text_tokens",
    "reasoning_tokens",
    "accepted_prediction_tokens",
    "rejected_prediction_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
)
_CACHED_FIELDS = ("text_tokens", "audio_tokens", "image_tokens")


def _details(value: object, fields: tuple[str, ...]) -> dict[str, Json]:
    return {field: cast("Json", getattr(value, field, None)) for field in fields}


def _cached_details(details: PromptTokensDetailsWrapper) -> Json:
    cached = cast("CachedTokensDetails | None", getattr(details, "cached_tokens_details", None))
    return None if cached is None else _details(cached, _CACHED_FIELDS)


def _numeric_extras(usage: Usage) -> dict[str, Json]:
    extras = cast("dict[str, object]", usage.model_extra or {})
    return {
        name: cast("Json", value)
        for name, value in extras.items()
        if name != "cost" and not name.startswith("_") and isinstance(value, (int, float))
    }


def _usage_projection(usage: Usage) -> dict[str, Json]:
    prompt = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    prompt_projection: Json = None
    if prompt is not None:
        prompt_projection = {
            **_details(prompt, _PROMPT_FIELDS),
            "cached_tokens_details": _cached_details(prompt),
        }
    completion_projection: Json = (
        None if completion is None else _details(completion, _COMPLETION_FIELDS)
    )
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": prompt_projection,
        "completion_tokens_details": completion_projection,
        "cost": cast("Json", getattr(usage, "cost", None)),
        "numeric_extras": _numeric_extras(usage),
    }


def _event_projection(events: Sequence[Mapping[str, object]]) -> list[Json]:
    return [cast("Json", dict(event)) for event in events]


def _part_projection(
    part: LiteLLMRealtimeStreamLoggingObject,
) -> dict[str, Json]:
    return {
        "service_tier": part.service_tier,
        "usage": _usage_projection(part.usage),
        "results": _event_projection(part.results),
    }


def _collect(events: OpenAIRealtimeStreamList) -> Json:
    realtime_usages = RealtimeAPITokenUsageProcessor.collect_usage_from_realtime_stream_results(
        events
    )
    realtime_combined = RealtimeAPITokenUsageProcessor.combine_usage_objects(realtime_usages)
    responses_usages = (
        ResponsesWebSocketTokenUsageProcessor.collect_usage_from_responses_ws_results(events)
    )
    responses_combined = ResponsesWebSocketTokenUsageProcessor.combine_usage_objects(
        list(responses_usages)
    )
    partition = ResponsesWebSocketTokenUsageProcessor.partition_results_by_service_tier(events)
    split = _split_responses_ws_logging_object_by_service_tier(  # pyright: ignore[reportPrivateUsage]
        LiteLLMRealtimeStreamLoggingObject(results=events, usage=responses_combined)
    )
    return {
        "realtime": {
            "usages": [_usage_projection(usage) for usage in realtime_usages],
            "combined": _usage_projection(realtime_combined),
        },
        "responses_ws": {
            "usages": [_usage_projection(usage) for usage in responses_usages],
            "combined": _usage_projection(responses_combined),
            "partitions": [
                {
                    "service_tier": tier,
                    "results": _event_projection(group),
                }
                for tier, group in partition.items()
            ],
            "split": None if split is None else [_part_projection(part) for part in split],
        },
    }


def _events(value: Json) -> list[dict[str, Json]]:
    if not isinstance(value, list):
        raise ValueError("events must be a list")
    events: list[dict[str, Json]] = []
    for item in value:
        if not isinstance(item, dict):
            raise ValueError("every event must be an object")
        events.append(cast("dict[str, Json]", item))
    return events


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    events = cast("OpenAIRealtimeStreamList", _events(input_value.get("events")))
    if function == "collect":
        return _collect(events)
    if function == "transcription_model":
        model = _get_transcription_model_name_from_results(  # pyright: ignore[reportPrivateUsage]
            events
        )
        return {"model": model}
    raise ValueError(f"unknown function: {function!r}")
