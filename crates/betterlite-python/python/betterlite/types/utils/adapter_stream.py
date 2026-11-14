import logging
from collections.abc import AsyncIterable, AsyncIterator, Iterable, Iterator
from typing import Final

from pydantic import JsonValue

LOGGER: Final = logging.getLogger("LiteLLM")


class AdapterStreamError(Exception): ...


class AdapterCompletionStreamWrapper:
    # LiteLLM stores whatever it is given; a None stream fails when it is first read
    completion_stream: Iterable[JsonValue] | AsyncIterable[JsonValue] | None

    def __init__(
        self, completion_stream: Iterable[JsonValue] | AsyncIterable[JsonValue] | None,
    ) -> None:
        self.completion_stream = completion_stream

    def __iter__(self) -> Iterator[JsonValue]:
        return self

    def __aiter__(self) -> AsyncIterator[JsonValue]:
        return self

    def __next__(self) -> JsonValue:
        match self.completion_stream:
            case Iterable():
                return self._next_chunk(self.completion_stream)
            case AsyncIterable() | None:
                raise TypeError("the stream is not synchronous")

    async def __anext__(self) -> JsonValue:
        match self.completion_stream:
            case AsyncIterable():
                return await self._anext_chunk(self.completion_stream)
            case Iterable() | None:
                raise TypeError("the stream is not asynchronous")

    def _next_chunk(self, stream: Iterable[JsonValue]) -> JsonValue:
        try:
            for chunk in stream:
                if chunk == "None" or chunk is None:
                    raise AdapterStreamError
                return chunk
        except AdapterStreamError as error:
            LOGGER.debug("AdapterCompletionStreamWrapper - %s", error)
            return None
        raise StopIteration

    async def _anext_chunk(self, stream: AsyncIterable[JsonValue]) -> JsonValue:
        try:
            async for chunk in stream:
                if chunk == "None" or chunk is None:
                    raise AdapterStreamError
                return chunk
            raise StopIteration
        except StopIteration as error:
            raise StopAsyncIteration from error


class CustomStreamingDecoder:
    async def aiter_bytes(
        self, iterator: AsyncIterator[bytes],
    ) -> AsyncIterator[JsonValue]:
        raise NotImplementedError(
            "a decoder subclass implements aiter_bytes",
        )

    def iter_bytes(self, iterator: Iterator[bytes]) -> Iterator[JsonValue]:
        raise NotImplementedError(
            "a decoder subclass implements iter_bytes",
        )
