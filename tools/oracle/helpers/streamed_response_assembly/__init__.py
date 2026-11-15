import sys
from contextlib import redirect_stdout
from typing import Protocol, runtime_checkable

import litellm.main as main_module
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ModelResponse, TextCompletionResponse
from pydantic import JsonValue as Json

from .build import INPUT, ChunkWire, Input, build_chunk
from .fixture import FailingPrompt, FixedPrompt, logging_fixture, prompt_callback
from .projection import response_view


# LiteLLM annotates chunks as list, but null reaches its None-chunks APIError.
class Builder(Protocol):
    def __call__(
        self,
        *,
        chunks: list[ChunkWire] | None,
        messages: list[dict[str, Json]] | None,
        logging_obj: Logging | None,
        count_prompt_tokens: FixedPrompt | FailingPrompt | None,
    ) -> ModelResponse | TextCompletionResponse | None: ...


@runtime_checkable
class AnyModule(Protocol): ...


@runtime_checkable
class BuilderModule(AnyModule, Protocol):
    stream_chunk_builder: Builder


# The class pattern reads the module through the protocol, so LiteLLM's loose signature stays out.
def builder(module: AnyModule) -> Builder:
    match module:
        case BuilderModule():
            return module.stream_chunk_builder
        case AnyModule():
            raise TypeError("litellm.main has no stream_chunk_builder")


def assemble(request: Input) -> Json:
    raw_chunks = request.get("chunks")
    chunks: list[ChunkWire] | None = None
    if raw_chunks is not None:
        chunks = [build_chunk(chunk) for chunk in raw_chunks]
    response = builder(main_module)(
        chunks=chunks,
        messages=request.get("messages"),
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
