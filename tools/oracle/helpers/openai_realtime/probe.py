import asyncio
import ssl as ssl_module
import sys
from types import ModuleType, TracebackType
from typing import Self

from typing_extensions import override
from websockets import exceptions as websockets_exceptions
from websockets import frames as websockets_frames
from websockets.asyncio import client as websockets_client
from websockets.datastructures import Headers
from websockets.exceptions import InvalidStatus
from websockets.http11 import Response

from .types import ClientSocket, CloseFrame, FailMode, FailureSetup, SslKind

REAL_WEBSOCKETS: ModuleType = sys.modules["websockets"]


class ProbeClient(ClientSocket):
    def __init__(self, scope_headers: list[tuple[str, str]], close_error: str | None) -> None:
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {
            "headers": [
                (name.encode("latin-1"), value.encode("latin-1"))
                for name, value in scope_headers
            ]
        }
        self.close_error = close_error
        self.sent: list[str] = []
        self.closed: CloseFrame | None = None
        self.close_raised = False

    @override
    async def send_text(self, data: str) -> None:
        self.sent.append(data)

    @override
    async def receive_text(self) -> str:
        raise ConnectionError("the probe client never sends a frame")

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.closed = {"code": code, "reason": reason}
        if self.close_error is None:
            return
        self.close_raised = True
        raise RuntimeError(self.close_error)


class ProbeConnection:
    def __init__(self, failure: FailureSetup | None, mode: FailMode) -> None:
        self.failure = failure
        self.mode = mode

    async def __aenter__(self) -> Self:
        failure = self.failure
        if failure is None:
            return self
        if self.mode == "invalid_status":
            rejected = Response(failure["status_code"], "Handshake rejected", Headers())
            raise InvalidStatus(rejected)
        raise RuntimeError(failure["message"])

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...

    async def recv(self, _decode: bool = True) -> str:
        # the relay task waits for the client loop to cancel it; no upstream frame is relayed
        await asyncio.Future[None]()
        return ""


class ProbeServer:
    def __init__(
        self,
        scope_headers: list[tuple[str, str]],
        close_error: str | None,
        mode: FailMode,
    ) -> None:
        self.client = ProbeClient(scope_headers, close_error)
        self.failure: FailureSetup | None = None
        self.mode = mode
        self.url: str | None = None
        self.headers: dict[str, str] | None = None
        self.max_size: int | None = None
        self.ssl_kind: SslKind | None = None

    def connect(
        self,
        url: str,
        *,
        additional_headers: dict[str, str],
        max_size: int | None,
        ssl: bool | str | ssl_module.SSLContext | None,
    ) -> ProbeConnection:
        self.url = url
        self.headers = additional_headers
        self.max_size = max_size
        self.ssl_kind = ssl_kind(ssl)
        return ProbeConnection(self.failure, self.mode)


class FakeWebsockets(ModuleType):
    exceptions: ModuleType
    frames: ModuleType
    ClientConnection: type[websockets_client.ClientConnection]

    def __init__(self, server: ProbeServer) -> None:
        super().__init__("websockets")
        self.exceptions = websockets_exceptions
        self.frames = websockets_frames
        self.ClientConnection = websockets_client.ClientConnection
        self.server = server

    def connect(
        self,
        url: str,
        *,
        additional_headers: dict[str, str],
        max_size: int | None,
        ssl: bool | str | ssl_module.SSLContext | None,
    ) -> ProbeConnection:
        return self.server.connect(
            url, additional_headers=additional_headers, max_size=max_size, ssl=ssl
        )


def ssl_kind(value: bool | str | ssl_module.SSLContext | None) -> SslKind:
    if value is None:
        return "none"
    if value is True:
        return "insecure"
    return "shared"
