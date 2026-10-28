import base64
import binascii
import time
from collections.abc import Mapping, Sequence
from typing import TypeAlias, TypedDict, cast

from litellm.litellm_core_utils.streaming_chunk_builder_utils import ChunkProcessor
from litellm.types.llms.openai import ChatCompletionAudioDelta

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ContentChoice(TypedDict, total=False):
    delta: Mapping[str, str | None]


class ContentChunk(TypedDict):
    choices: Sequence[ContentChoice]


class ThinkingFragment(TypedDict, total=False):
    type: str | None
    data: str | None
    thinking: str | None
    signature: str | None


class ThinkingDelta(TypedDict, total=False):
    thinking_blocks: Sequence[ThinkingFragment]


class ThinkingChoice(TypedDict, total=False):
    delta: ThinkingDelta


class ThinkingChunk(TypedDict):
    choices: Sequence[ThinkingChoice]


class AudioDelta(TypedDict, total=False):
    audio: ChatCompletionAudioDelta | None


class AudioChoice(TypedDict, total=False):
    delta: AudioDelta


class AudioChunk(TypedDict):
    choices: Sequence[AudioChoice]


def decode_fragment(fragment: str) -> Json:
    try:
        decoded = list(base64.b64decode(fragment, validate=False))
    except (ValueError, binascii.Error) as error:
        return {"ok": False, "error_type": type(error).__name__, "error": str(error)}
    else:
        return cast(Json, {"ok": True, "bytes": decoded})


def assemble_audio(chunks: list[AudioChunk], fixed_time: int) -> Json:
    processor = ChunkProcessor(chunks)
    original_time = time.time
    time.time = lambda: fixed_time
    try:
        response = processor.get_combined_audio_content(chunks)
        return cast(Json, response.model_dump())
    except (TypeError, ValueError, binascii.Error) as error:
        return {"ok": False, "error_type": type(error).__name__, "error": str(error)}
    finally:
        time.time = original_time


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    if function == "batch":
        cases = cast(list[dict[str, Json]], input_value["cases"])
        return [run(case) for case in cases]
    raw_chunks = cast(list[dict[str, Json]], input_value.get("chunks", []))
    if function == "audio_base64_decoder":
        return decode_fragment(cast(str, input_value["fragment"]))
    if function == "content_and_reasoning":
        chunks = cast(list[ContentChunk], raw_chunks)
        processor = ChunkProcessor(chunks)
        try:
            content = processor.get_combined_content(chunks)
            reasoning = processor.get_combined_reasoning_content(chunks)
        except (TypeError, ValueError) as error:
            return {"ok": False, "error_type": type(error).__name__, "error": str(error)}
        else:
            return cast(Json, {"content": content, "reasoning": reasoning})
    if function == "thinking_signature_boundaries":
        chunks = cast(list[ThinkingChunk], raw_chunks)
        processor = ChunkProcessor(chunks)
        blocks = processor.get_combined_thinking_content(chunks)
        return None if blocks is None else cast(Json, [dict(block) for block in blocks])
    if function == "audio_fragments_and_expiry":
        chunks = cast(list[AudioChunk], raw_chunks)
        return assemble_audio(chunks, cast(int, input_value["now"]))
    raise ValueError(f"unknown stream_content_assembly function: {function}")
