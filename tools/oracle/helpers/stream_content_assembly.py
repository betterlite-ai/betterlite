import base64
import binascii
import time
from typing import TypeAlias, cast

from litellm.litellm_core_utils.streaming_chunk_builder_utils import ChunkProcessor

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    chunks = cast(list[dict[str, Json]], input_value.get("chunks", []))
    processor = ChunkProcessor(chunks)
    if function == "content_and_reasoning":
        return {"content": processor.get_combined_content(chunks), "reasoning": processor.get_combined_reasoning_content(chunks)}
    if function == "thinking_signature_boundaries":
        return processor.get_combined_thinking_content(chunks)
    if function == "audio_fragments_and_expiry":
        fixed_time = cast(int, input_value["now"])
        original_time = time.time
        time.time = lambda: fixed_time
        try:
            return processor.get_combined_audio_content(chunks)
        finally:
            time.time = original_time
    if function == "audio_base64_decoder":
        try:
            return {"ok": True, "bytes": list(base64.b64decode(cast(str, input_value["fragment"]), validate=False))}
        except (ValueError, binascii.Error) as error:
            return {"ok": False, "error": str(error)}
    raise ValueError(f"unknown stream_content_assembly function: {function}")
