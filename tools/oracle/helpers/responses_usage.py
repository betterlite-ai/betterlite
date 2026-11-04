from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.responses.utils import ResponseAPILoggingUtils
from litellm.types.llms.openai import ResponseAPIUsage
from litellm.types.utils import CompletionTokensDetailsWrapper, PromptTokensDetailsWrapper, Usage

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
_PROMPT_FIELDS = (
    "cached_tokens", "audio_tokens", "text_tokens", "image_tokens", "video_tokens",
    "cache_write_tokens", "web_search_requests", "google_maps_grounding_requests",
)
_COMPLETION_FIELDS = ("text_tokens", "reasoning_tokens", "audio_tokens", "image_tokens")


def _detail_projection(value: object, fields: tuple[str, ...]) -> dict[str, Json] | None:
    if value is None:
        return None
    return {
        field: cast("Json", field_value)
        for field in fields
        if (field_value := getattr(value, field, None)) is not None
    }


def _json_projection(value: object) -> Json:
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, list):
        items = cast("list[object]", value)
        return [_json_projection(item) for item in items]
    if isinstance(value, dict):
        fields = cast("dict[object, object]", value)
        if not all(isinstance(key, str) for key in fields):
            raise TypeError("usage extension object has a non-string key")
        string_fields = cast("dict[str, object]", fields)
        return {key: _json_projection(item) for key, item in string_fields.items()}
    raise TypeError("usage extension is not JSON")


def _usage_projection(usage: Usage) -> dict[str, Json]:
    result: dict[str, Json] = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }
    prompt: PromptTokensDetailsWrapper | None = usage.prompt_tokens_details
    completion: CompletionTokensDetailsWrapper | None = usage.completion_tokens_details
    prompt_fields = _detail_projection(prompt, _PROMPT_FIELDS)
    completion_fields = _detail_projection(completion, _COMPLETION_FIELDS)
    if prompt_fields is not None:
        nested = getattr(prompt, "cached_tokens_details", None)
        prompt_fields["cached_tokens_details"] = _detail_projection(
            nested, ("text_tokens", "audio_tokens", "image_tokens")
        )
        result["prompt_tokens_details"] = prompt_fields
    if completion_fields is not None:
        result["completion_tokens_details"] = completion_fields
    extra: object = usage.model_extra or {}
    fields = cast("dict[str, object]", extra)
    for key, value in fields.items():
        result[key] = _json_projection(value)
    cost = getattr(usage, "cost", None)
    if cost is not None:
        result["cost"] = cast("Json", cost)
    return result


def run(input_value: dict[str, Json]) -> Json:
    transform = cast(
        "Callable[[object], Usage]",
        ResponseAPILoggingUtils._transform_response_api_usage_to_chat_usage,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    raw = input_value.get("usage")
    if input_value.get("typed_object") is True and isinstance(raw, dict):
        parsed = ResponseAPIUsage.model_validate(raw)
        usage = transform(parsed)
    elif raw is None:
        usage = transform(None)
    elif isinstance(raw, dict):
        usage = transform(raw)
    else:
        usage = transform(None)
    return {"usage": _usage_projection(usage)}
