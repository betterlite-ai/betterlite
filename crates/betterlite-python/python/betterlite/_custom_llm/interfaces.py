from typing import Protocol

__all__ = [
    "AsyncClientSession",
    "AsyncHTTPClient",
    "HTTPResponse",
    "SyncHTTPClient",
    "TimeoutConfig",
    "Tokenizer",
]


class TimeoutConfig(Protocol):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


class SyncHTTPClient(Protocol):
    def close(self) -> None: ...


class AsyncHTTPClient(Protocol):
    async def close(self) -> None: ...


class AsyncClientSession(Protocol):
    async def aclose(self) -> None: ...


class HTTPResponse(Protocol):
    status_code: int

    @property
    def text(self) -> str: ...


class Tokenizer(Protocol):
    def encode(self, text: str) -> list[int]: ...
