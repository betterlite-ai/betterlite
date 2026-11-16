import asyncio
import sys
from datetime import UTC, datetime
from types import ModuleType, TracebackType
from typing import Self

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.realtime.handler import OpenAIRealtime
from pydantic import JsonValue
from typing_extensions import override
from websockets import exceptions as websockets_exceptions
from websockets import frames as websockets_frames
from websockets.asyncio import client as websockets_client

from .types import FAILURE, JSON, ClientSocket, Input, RealtimeCall, RealtimeRunner, SslValue

REAL_WEBSOCKETS: ModuleType = sys.modules["websockets"]
RUNNER: RealtimeRunner = OpenAIRealtime()


class Server(ClientSocket):
    def __init__(self, input_value: Input) -> None:
        self.input_value = input_value
        self.scope = {
            "headers": [
                (name.encode("latin-1"), value.encode("latin-1"))
                for name, value in input_value["scope_headers"]
            ]
        }
        self.url: str | None = None
        self.headers: dict[str, str] | None = None
        self.max_size: int | None = None
        self.ssl: SslValue = None
        self.sent: list[str] = []
        self.closed: dict[str, JsonValue] | None = None

    def connect(self, url: str, *, additional_headers: dict[str, str], max_size: int | None, ssl: SslValue) -> Self:
        self.url = url
        self.headers = additional_headers
        self.max_size = max_size
        self.ssl = ssl
        return self

    async def __aenter__(self) -> Self:
        raise RuntimeError(FAILURE)

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...

    @override
    async def send_text(self, data: str) -> None:
        self.sent.append(data)

    @override
    async def receive_text(self) -> str:
        raise ConnectionError("the probe never reads a frame")

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.closed = {"code": code, "reason": reason}
        close_error = self.input_value["close_error"]
        if close_error is not None:
            raise RuntimeError(close_error)


class FakeWebsockets(ModuleType):
    exceptions: ModuleType
    frames: ModuleType
    ClientConnection: type[websockets_client.ClientConnection]

    def __init__(self, server: Server) -> None:
        super().__init__("websockets")
        self.exceptions = websockets_exceptions
        self.frames = websockets_frames
        self.ClientConnection = websockets_client.ClientConnection
        self.server = server

    def connect(self, url: str, *, additional_headers: dict[str, str], max_size: int | None, ssl: SslValue) -> Server:
        return self.server.connect(url, additional_headers=additional_headers, max_size=max_size, ssl=ssl)


def ssl_kind(*, value: SslValue) -> str:
    if value is None:
        return "none"
    if value is True:
        return "insecure"
    return "shared"


def header_pairs(headers: dict[str, str] | None) -> list[JsonValue]:
    if headers is None:
        return []
    pairs: list[JsonValue] = [[name, value] for name, value in sorted(headers.items())]
    return pairs


async def probe(input_value: Input, server: Server) -> None:
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
        "websocket": server,
        "logging_obj": logging_obj,
        "api_base": input_value["api_base"],
        "api_key": input_value["api_key"],
        "timeout": None,
        "query_params": input_value["query_params"],
        "litellm_metadata": None,
    }
    sys.modules["websockets"] = FakeWebsockets(server)
    try:
        await RUNNER.async_realtime(**call)
    finally:
        sys.modules["websockets"] = REAL_WEBSOCKETS


def realtime_answer(input_value: Input) -> JsonValue:
    server = Server(input_value)
    asyncio.run(probe(input_value, server))
    payload: dict[str, JsonValue] = {
        "url": server.url,
        "headers": header_pairs(server.headers),
        "max_size": server.max_size,
        "ssl": ssl_kind(value=server.ssl),
    }
    return JSON.validate_python(payload)
