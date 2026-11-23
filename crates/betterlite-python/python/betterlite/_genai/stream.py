from collections.abc import AsyncIterator, Iterator

from typing_extensions import override

from betterlite._completion.errors import raise_native_error
from betterlite._native import NativeGenaiError, NativeGenaiStream


# LiteLLM's two streaming iterators as one class: sync iterates it, async awaits chunks
class GenaiStream(Iterator[bytes]):
    def __init__(self, native: NativeGenaiStream) -> None:
        self._native = native

    @override
    def __next__(self) -> bytes:
        chunk = self.pull()
        if chunk is None:
            raise StopIteration
        return chunk

    async def __anext__(self) -> bytes:
        chunk = await self.pull_async()
        if chunk is None:
            raise StopAsyncIteration
        return chunk

    def __aiter__(self) -> AsyncIterator[bytes]:
        return self

    # a stream that fails mid-flight raises the provider's class, with the native error as its cause
    def pull(self) -> bytes | None:
        try:
            return self._native.next_chunk()
        except NativeGenaiError as failure:
            raise_native_error(failure)

    async def pull_async(self) -> bytes | None:
        try:
            return await self._native.anext_chunk()
        except NativeGenaiError as failure:
            raise_native_error(failure)

    def close(self) -> None:
        self._native.close()

    async def aclose(self) -> None:
        await self._native.aclose()
