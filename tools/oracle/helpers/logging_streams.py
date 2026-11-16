from typing import Final, Literal, TypeAlias, TypedDict

from litellm.litellm_core_utils.served_output_texts import (
    overlay_served_output_texts,
    served_output_texts,
    served_stream_output_texts,
)
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)
_CHUNKS: Final = TypeAdapter(list[ModelResponseStream])
_TEXTS: Final = TypeAdapter(tuple[str | None, ...])
_OVERLAY_RESPONSE: Final = TypeAdapter(
    dict[str, JsonValue] | str | list[JsonValue] | None
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


Input: TypeAlias = ChatProbe | StreamProbe | OverlayProbe


class ProbesInput(TypedDict):
    probes: list[Input]


def chat_value(probe: ChatProbe) -> JsonValue:
    values: list[JsonValue] = []
    for response in probe["responses"]:
        values.append(_JSON.validate_python(served_output_texts(response)))
    return values


def stream_value(probe: StreamProbe) -> JsonValue:
    values: list[JsonValue] = []
    for chunks in probe["chunk_lists"]:
        values.append(
            _JSON.validate_python(served_stream_output_texts(_CHUNKS.validate_python(chunks)))
        )
    return values


def overlay_value(probe: OverlayProbe) -> JsonValue:
    response = _OVERLAY_RESPONSE.validate_python(probe["response"])
    texts = probe["texts"]
    served = None if texts is None else _TEXTS.validate_python(texts)
    overlaid = overlay_served_output_texts(response, served)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/served_output_texts.py#L142-L148
    return _JSON.validate_python(overlaid)


def probe_value(probe: Input) -> JsonValue:
    if probe["operation"] == "chat":
        return chat_value(probe)
    if probe["operation"] == "stream":
        return stream_value(probe)
    return overlay_value(probe)


def run(input_value: ProbesInput) -> JsonValue:
    values: list[JsonValue] = []
    for probe in input_value["probes"]:
        values.append(probe_value(probe))
    return values
