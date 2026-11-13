import logging
from collections.abc import AsyncIterable, AsyncIterator, Iterable, Iterator
from typing import Final

from betterlite.types.utils.values import StoredValue

LOGGER: Final = logging.getLogger("LiteLLM")


class AdapterStreamError(Exception): ...


class AdapterCompletionStreamWrapper:
    completion_stream: Iterable[StoredValue] | AsyncIterable[StoredValue]

    def __init__(
        self, completion_stream: Iterable[StoredValue] | AsyncIterable[StoredValue],
    ) -> None:
        self.completion_stream = completion_stream

    def __iter__(self) -> Iterator[StoredValue]:
        return self

    def __aiter__(self) -> AsyncIterator[StoredValue]:
        return self

    def __next__(self) -> StoredValue:
        match self.completion_stream:
            case Iterable():
                return self._next_chunk(self.completion_stream)
            case _other:
                raise TypeError("the stream is not synchronous")

    async def __anext__(self) -> StoredValue:
        match self.completion_stream:
            case AsyncIterable():
                return await self._anext_chunk(self.completion_stream)
            case _other:
                raise TypeError("the stream is not asynchronous")

    def _next_chunk(self, stream: Iterable[StoredValue]) -> StoredValue:
        try:
            for chunk in stream:
                if chunk == "None" or chunk is None:
                    raise AdapterStreamError
                return chunk
            raise StopIteration
        except StopIteration:
            raise
        except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2829
            LOGGER.debug("AdapterCompletionStreamWrapper - %s", error)
            return None

    async def _anext_chunk(self, stream: AsyncIterable[StoredValue]) -> StoredValue:
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
    ) -> AsyncIterator[StoredValue]:
        raise NotImplementedError(
            "a decoder subclass implements aiter_bytes",
        )

    def iter_bytes(self, iterator: Iterator[bytes]) -> Iterator[StoredValue]:
        raise NotImplementedError(
            "a decoder subclass implements iter_bytes",
        )
