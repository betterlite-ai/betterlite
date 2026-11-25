import datetime
from collections.abc import Awaitable, Iterator
from typing import Final, Protocol, Self

from pydantic import JsonValue
from typing_extensions import override

from betterlite._completion.errors import raise_native_error
from betterlite._native import NativeFileContentError
from betterlite.files.types import FileContentProvider

SYNC_REFUSED: Final = "File content stream does not support sync iteration"
ASYNC_REFUSED: Final = "File content stream does not support async iteration"


class FileChunkStream(Protocol):
    is_async: bool

    def next_chunk(self) -> bytes | None: ...

    def anext_chunk(self) -> Awaitable[bytes | None]: ...

    def close(self) -> None: ...

    def aclose(self) -> Awaitable[None]: ...


class StreamLogging(Protocol):
    start_time: datetime.datetime


class _ClosedFileChunks(FileChunkStream):
    # Both closes swap the stream for this first, as LiteLLM swaps in iter(()) before closing the real one.
    is_async = False

    @override
    def next_chunk(self) -> None:
        ...

    @override
    async def anext_chunk(self) -> None:
        ...

    @override
    def close(self) -> None:
        ...

    @override
    async def aclose(self) -> None:
        ...


class FileContentStreamingResponse(Iterator[bytes]):
    def __init__(
        self,
        stream_iterator: FileChunkStream,
        file_id: str,
        model: str | None,
        custom_llm_provider: FileContentProvider | str | None,
        logging_obj: StreamLogging | None = None,
    ) -> None:
        self.stream_iterator = stream_iterator
        self.file_id = file_id
        self.model = model
        self.custom_llm_provider = custom_llm_provider
        self.logging_obj = logging_obj
        self.standard_logging_object: dict[str, JsonValue] | None = None
        self._hidden_params: dict[str, JsonValue] = {
            "litellm_model_name": model,
            "response_cost": None,
        }
        self._logging_completed = False
        self._close_completed = False
        now = datetime.datetime.now(tz=datetime.UTC)
        self._start_time = now if logging_obj is None else logging_obj.start_time

    @override
    def __iter__(self) -> Self:
        if self.stream_iterator.is_async:
            raise TypeError(SYNC_REFUSED)
        return self

    @override
    def __next__(self) -> bytes:
        if self.stream_iterator.is_async:
            raise TypeError(SYNC_REFUSED)
        try:
            chunk = self.stream_iterator.next_chunk()
        except NativeFileContentError as failure:
            raise_native_error(failure)
        if chunk is None:
            raise StopIteration
        return chunk

    def __aiter__(self) -> Self:
        if not self.stream_iterator.is_async:
            raise TypeError(ASYNC_REFUSED)
        return self

    async def __anext__(self) -> bytes:
        if not self.stream_iterator.is_async:
            raise TypeError(ASYNC_REFUSED)
        try:
            chunk = await self.stream_iterator.anext_chunk()
        except NativeFileContentError as failure:
            raise_native_error(failure)
        if chunk is None:
            raise StopAsyncIteration
        return chunk

    async def aclose(self) -> None:
        if self._close_completed:
            return
        self._close_completed = True
        self._logging_completed = True
        stream_to_close = self.stream_iterator
        self.stream_iterator = _ClosedFileChunks()
        try:
            await stream_to_close.aclose()
        except NativeFileContentError as failure:
            raise_native_error(failure)

    def close(self) -> None:
        if self._close_completed:
            return
        self._close_completed = True
        self._logging_completed = True
        stream_to_close = self.stream_iterator
        self.stream_iterator = _ClosedFileChunks()
        try:
            stream_to_close.close()
        except NativeFileContentError as failure:
            raise_native_error(failure)
