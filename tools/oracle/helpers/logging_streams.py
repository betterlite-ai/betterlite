from typing import Final, Literal, TypeAlias, TypedDict

from litellm.litellm_core_utils.served_output_texts import (
    overlay_served_output_texts,
    served_output_texts,
    served_stream_output_texts,
)
from litellm.litellm_core_utils.streaming_handler import (
    backfill_missing_cache_usage_fields,
)
from litellm.types.utils import ModelResponseStream, Usage
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)
_CHUNKS: Final = TypeAdapter(list[ModelResponseStream])
_TEXTS: Final = TypeAdapter(tuple[str | None, ...])
_OVERLAY_RESPONSE: Final = TypeAdapter(
    dict[str, object] | str | list[object] | None
)


class ChatProbe(TypedDict):
    operation: Literal["chat"]
    responses: list[JsonValue]


class StreamProbe(TypedDict):
    operation: Literal["stream"]
    chunk_lists: list[list[JsonValue]]


class OverlayProbe(TypedDict):
    operation: Literal["overlay"]
    response: JsonValue
    texts: list[str | None] | None


class UsageInput(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: dict[str, int | None] | None
    cache_read_input_tokens: int
    cache_creation_input_tokens: int


class PartialUsageProbe(TypedDict):
    operation: Literal["partial_usage"]
    usages: list[JsonValue]


Input: TypeAlias = ChatProbe | StreamProbe | OverlayProbe | PartialUsageProbe


class ProbesInput(TypedDict):
    probes: list[Input]


_USAGE_INPUT: Final = TypeAdapter(UsageInput)


def chat_value(probe: ChatProbe) -> JsonValue:
    values: list[JsonValue] = []
    for response in probe["responses"]:
        texts = served_output_texts(response)
        values.append(None if texts is None else list(texts))
    return values


def stream_value(probe: StreamProbe) -> JsonValue:
    values: list[JsonValue] = []
    for chunks in probe["chunk_lists"]:
        texts = served_stream_output_texts(_CHUNKS.validate_python(chunks))
        values.append(None if texts is None else list(texts))
    return values


def overlay_value(probe: OverlayProbe) -> JsonValue:
    response = _OVERLAY_RESPONSE.validate_python(probe["response"])
    texts = probe["texts"]
    served = None if texts is None else _TEXTS.validate_python(texts)
    overlaid = overlay_served_output_texts(response, served)
    return _JSON.validate_python(overlaid)


def usage_value(usage: JsonValue) -> JsonValue:
    parsed = _USAGE_INPUT.validate_python(usage)
    filled = Usage(**parsed)
    backfill_missing_cache_usage_fields(filled)
    return _JSON.validate_json(filled.model_dump_json())


def run(input_value: ProbesInput) -> JsonValue:
    results: list[JsonValue] = []
    for probe in input_value["probes"]:
        if probe["operation"] == "chat":
            results.append(chat_value(probe))
        elif probe["operation"] == "stream":
            results.append(stream_value(probe))
        elif probe["operation"] == "partial_usage":
            results.append([usage_value(usage) for usage in probe["usages"]])
        else:
            results.append(overlay_value(probe))
    return results
