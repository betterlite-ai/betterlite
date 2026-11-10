from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import (
    VertexGeminiConfig,
)
from litellm.types.llms.vertex_ai import GenerateContentResponseBody
from litellm.types.utils import Usage

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]

PROMPT_FIELDS: tuple[str, ...] = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "cached_tokens",
    "tool_use_tokens",
    "web_search_requests",
    "google_maps_grounding_requests",
)
COMPLETION_FIELDS: tuple[str, ...] = (
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "reasoning_tokens",
)


def _fields(value: object, names: tuple[str, ...]) -> Json:
    return {name: cast(Json, getattr(value, name, None)) for name in names}


def _usage_state(usage: Usage) -> Json:
    prompt = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": None if prompt is None else _fields(prompt, PROMPT_FIELDS),
        "completion_tokens_details": None
        if completion is None
        else _fields(completion, COMPLETION_FIELDS),
    }


def run(input_value: dict[str, Json]) -> Json:
    body: dict[str, Json] = {}
    metadata = input_value.get("usage_metadata")
    if metadata is not None:
        body["usageMetadata"] = metadata
    candidates = input_value.get("candidates")
    if candidates is not None:
        body["candidates"] = candidates
    usage = VertexGeminiConfig._calculate_usage(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]
        cast(GenerateContentResponseBody, body)
    )
    grounding_metadata = cast(
        "Sequence[Mapping[str, object]]", input_value.get("grounding_metadata", [])
    )
    VertexGeminiConfig._set_grounding_usage_counters(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]
        usage, grounding_metadata
    )
    return _usage_state(usage)
