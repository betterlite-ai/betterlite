import asyncio
import ssl as ssl_module
import sys
from collections.abc import Mapping
from datetime import UTC, datetime
from types import ModuleType, TracebackType
from typing import Self

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.realtime.handler import (
    AzureOpenAIRealtime,
    azure_realtime_protocol_for_client,
)
from pydantic import JsonValue
from typing_extensions import override
from websockets import exceptions as websockets_exceptions
from websockets import frames as websockets_frames
from websockets.asyncio import client as websockets_client
from websockets.datastructures import Headers
from websockets.exceptions import InvalidStatus
from websockets.http11 import Response

from .types import (
    JSON,
    ClientSocket,
    CloseFrame,
    FailMode,
    FailureSetup,
    Input,
    ProtocolSetup,
    QuerySetup,
    RealtimeCall,
    Scope,
    SslKind,
)

RUNNER: AzureOpenAIRealtime = AzureOpenAIRealtime()
REAL_WEBSOCKETS: ModuleType = sys.modules["websockets"]
FAILURE_API_BASE: str = "https://probe.example"
FAILURE_API_VERSION: str = "2024-10-01-preview"


class ProbeClient(ClientSocket):
    def __init__(
        self, scope_headers: list[tuple[str, str]], send_error: str | None, close_error: str | None
    ) -> None:
        self.scope: Scope = {
            "headers": [
                (name.encode("latin-1"), value.encode("latin-1"))
                for name, value in scope_headers
            ]
        }
        self.send_error = send_error
        self.close_error = close_error
        self.sent: list[str] = []
        self.closed: CloseFrame | None = None
        self.send_raised = False
        self.close_raised = False

    @override
    async def send_text(self, data: str) -> None:
        if self.send_error is not None:
            self.send_raised = True
            raise RuntimeError(self.send_error)
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
        self.mode: FailMode = mode

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

    async def recv(self, *, _decode: bool = True) -> str:
        await asyncio.Future[None]()
        return ""


class ProbeServer:
    def __init__(
        self,
        scope_headers: list[tuple[str, str]],
        send_error: str | None,
        close_error: str | None,
        mode: FailMode,
    ) -> None:
        self.client = ProbeClient(scope_headers, send_error, close_error)
        self.failure: FailureSetup | None = None
        self.mode: FailMode = mode
        self.url: str | None = None
        self.headers: dict[str, str] | None = None
        self.max_size: int | None = None
        self.ssl_kind: SslKind | None = None

    def connect(
        self,
        url: str,
        *,
        additional_headers: Mapping[str, str],
        max_size: int | None,
        ssl: bool | str | ssl_module.SSLContext | None,
    ) -> ProbeConnection:
        self.url = url
        self.headers = dict(additional_headers)
        self.max_size = max_size
        self.ssl_kind = ssl_kind(value=ssl)
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
        additional_headers: Mapping[str, str],
        max_size: int | None,
        ssl: bool | str | ssl_module.SSLContext | None,
    ) -> ProbeConnection:
        return self.server.connect(
            url, additional_headers=additional_headers, max_size=max_size, ssl=ssl
        )


def ssl_kind(*, value: bool | str | ssl_module.SSLContext | None) -> SslKind:
    if value is None:
        return "none"
    return "insecure" if value is False else "shared"


def header_pairs(headers: dict[str, str] | None) -> list[JsonValue]:
    if headers is None:
        return []
    pairs: list[JsonValue] = [[name, value] for name, value in sorted(headers.items())]
    return pairs


def frame_json(frame: CloseFrame | None) -> JsonValue:
    if frame is None:
        return None
    payload: dict[str, JsonValue] = {"code": frame["code"], "reason": frame["reason"]}
    return JSON.validate_python(payload)


async def probe_call(input_value: Input, server: ProbeServer, setup: QuerySetup) -> None:
    logging_obj = Logging(
        model=input_value["model"],
        messages=None,
        stream=False,
        call_type="realtime",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )
    call: RealtimeCall = {
        "model": input_value["model"],
        "websocket": server.client,
        "logging_obj": logging_obj,
        "api_base": setup["api_base"],
        "api_key": setup["api_key"],
        "api_version": setup["api_version"],
        "azure_ad_token": setup["azure_ad_token"],
        "realtime_protocol": setup["realtime_protocol"],
        "query_params": setup["query_params"],
    }
    sys.modules["websockets"] = FakeWebsockets(server)
    try:
        await RUNNER.async_realtime(**call)
    finally:
        sys.modules["websockets"] = REAL_WEBSOCKETS


async def connect_one(input_value: Input, setup: QuerySetup) -> JsonValue:
    server = ProbeServer(input_value["scope_headers"], None, None, "error")
    await probe_call(input_value, server, setup)
    payload: dict[str, JsonValue] = {
        "url": server.url,
        "headers": header_pairs(server.headers),
        "max_size": server.max_size,
        "ssl": server.ssl_kind,
    }
    return JSON.validate_python(payload)


def protocol_one(input_value: Input, setup: ProtocolSetup) -> str:
    client = ProbeClient(input_value["scope_headers"], None, None)
    return azure_realtime_protocol_for_client(
        setup["configured_protocol"], query_params=setup["query_params"], websocket=client
    )


async def failure_probe(input_value: Input, failure: FailureSetup) -> JsonValue:
    mode: FailMode = "invalid_status" if input_value["kind"] == "handshake" else "error"
    server = ProbeServer(
        input_value["scope_headers"], failure["send_error"], failure["close_error"], mode
    )
    server.failure = failure
    setup: QuerySetup = {
        "api_base": FAILURE_API_BASE,
        "api_version": FAILURE_API_VERSION,
        "realtime_protocol": None,
        "query_params": None,
        "api_key": failure["api_key"],
        "azure_ad_token": failure["azure_ad_token"],
    }
    await probe_call(input_value, server, setup)
    payload: dict[str, JsonValue] = {
        "sent": [JSON.validate_json(text) for text in server.client.sent],
        "closed": frame_json(server.client.closed),
        "send_raised": server.client.send_raised,
        "close_raised": server.client.close_raised,
    }
    return JSON.validate_python(payload)
