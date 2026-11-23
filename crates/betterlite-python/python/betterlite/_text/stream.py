from collections.abc import AsyncIterator, Iterator

from pydantic import JsonValue

from betterlite._completion import CustomStreamWrapper
from betterlite._completion.stream import ChunkSource
from betterlite.types.utils import (
    ModelResponseStream,
    TextChoices,
    TextCompletionResponse,
)


# the shaped chunks the native stream already read, handed to CustomStreamWrapper as its source
class ChunkList(ChunkSource):
    chunks: list[str]
    position: int

    def __init__(self, chunks: list[str]) -> None:
        self.chunks = chunks
        self.position = 0

    def next_chunk(self) -> str | None:
        if self.position >= len(self.chunks):
            return None
        chunk = self.chunks[self.position]
        self.position += 1
        return chunk

    async def anext_chunk(self) -> str | None:
        return self.next_chunk()

    def close(self) -> None:
        self.position = len(self.chunks)

    async def aclose(self) -> None:
        self.close()


# utils.py's TextCompletionStreamWrapper: a chat stream's chunks read as text completion chunks.
class TextCompletionStreamWrapper:
    completion_stream: CustomStreamWrapper
    model: str | None
    stream_options: dict[str, JsonValue] | None
    custom_llm_provider: str | None

    def __init__(
        self,
        completion_stream: CustomStreamWrapper,
        model: str | None,
        stream_options: dict[str, JsonValue] | None = None,
        custom_llm_provider: str | None = None,
    ) -> None:
        self.completion_stream = completion_stream
        self.model = model
        self.stream_options = stream_options
        self.custom_llm_provider = custom_llm_provider

    def __iter__(self) -> Iterator[TextCompletionResponse]:
        return self

    def __next__(self) -> TextCompletionResponse:
        # a usage-only chunk carries no choice, which LiteLLM's conversion reads as an error
        for chunk in self.completion_stream:
            if chunk.choices:
                return self.text_chunk(chunk)
        raise StopIteration

    def __aiter__(self) -> AsyncIterator[TextCompletionResponse]:
        return self

    async def __anext__(self) -> TextCompletionResponse:
        async for chunk in self.completion_stream:
            if chunk.choices:
                return self.text_chunk(chunk)
        raise StopAsyncIteration

    def text_chunk(self, chunk: ModelResponseStream) -> TextCompletionResponse:
        choice = chunk.choices[0]
        converted = TextChoices(
            index=choice.index,
            text=choice.delta.content,
            finish_reason=choice.finish_reason,
        )
        converted["reasoning_content"] = choice.delta.reasoning_content
        response = TextCompletionResponse(
            id=chunk.id,
            created=chunk.created,
            model=chunk.model,
            choices=[converted],
        )
        # only pass usage when stream_options["include_usage"] is True
        if self.include_usage:
            response["usage"] = chunk.get("usage")
        return response

    @property
    def include_usage(self) -> bool:
        options = self.stream_options
        return options is not None and options.get("include_usage") is True
