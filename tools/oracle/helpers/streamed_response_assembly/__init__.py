import sys
from collections.abc import Callable
from contextlib import redirect_stdout
from typing import Final

from litellm.main import stream_chunk_builder
from litellm.types.utils import ModelResponse, TextCompletionResponse
from pydantic import JsonValue as Json

from .build import ChunkWire, build_chunk
from .fixture import START_TIME, logging_fixture, prompt_callback
from .projection import response_view
from .specs import INPUT, Input

# LiteLLM's annotation says list, but its runtime raises the None-chunks APIError, so null passes.
BUILD: Final[Callable[..., ModelResponse | TextCompletionResponse | None]] = stream_chunk_builder


def assemble(request: Input) -> Json:
    raw_chunks = request.get("chunks")
    chunks: list[ChunkWire] | None = None
    if raw_chunks is not None:
        chunks = [build_chunk(chunk) for chunk in raw_chunks]
    response = BUILD(
        chunks=chunks,
        messages=request.get("messages"),
        start_time=START_TIME,
        end_time=START_TIME,
        logging_obj=logging_fixture(request.get("logging")),
        count_prompt_tokens=prompt_callback(request.get("count_prompt_tokens")),
    )
    if response is None:
        return None
    return response_view(response)


def run(input_value: dict[str, Json]) -> Json:
    request = INPUT.validate_python(input_value)
    with redirect_stdout(sys.stderr):
        return assemble(request)
