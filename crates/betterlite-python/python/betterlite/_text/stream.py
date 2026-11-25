from collections.abc import AsyncIterator, Iterator

from pydantic import JsonValue
from typing_extensions import override

from betterlite._completion import CustomStreamWrapper
from betterlite._completion.stream import ChunkSource
from betterlite.types.utils import (
    ModelResponseStream,
    TextChoices,
    TextCompletionResponse,
)


class ChunkList(ChunkSource):
    def __init__(self, chunks: list[str]) -> None:
        self.chunks = chunks
        self.position = 0

    @override
    def next_chunk(self) -> str | None:
        if self.position >= len(self.chunks):
            return None
        chunk = self.chunks[self.position]
        self.position += 1
        return chunk

    @override
    async def anext_chunk(self) -> str | None:
        return self.next_chunk()

    @override
    def close(self) -> None:
        self.position = len(self.chunks)

    @override
    async def aclose(self) -> None:
        self.close()


class TextCompletionStreamWrapper:
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
                return self.completion_object(chunk)
        raise StopIteration

    def __aiter__(self) -> AsyncIterator[TextCompletionResponse]:
        return self

    async def __anext__(self) -> TextCompletionResponse:
        async for chunk in self.completion_stream:
            if chunk.choices:
                return self.completion_object(chunk)
        raise StopAsyncIteration

    def completion_object(self, chunk: ModelResponseStream) -> TextCompletionResponse:
        choice = chunk.choices[0]
        converted = TextChoices(
            index=choice.index,
            text=choice.delta.content,
            finish_reason=choice.finish_reason,
        )
        converted["reasoning_content"] = choice.delta.get("reasoning_content")
        response = TextCompletionResponse(
            id=chunk.id,
            created=chunk.created,
            model=chunk.model,
            choices=[converted],
        )
        # only pass usage when stream_options["include_usage"] is True
        if self.stream_options is not None and self.stream_options.get("include_usage") is True:
            response["usage"] = chunk.get("usage")
        return response
