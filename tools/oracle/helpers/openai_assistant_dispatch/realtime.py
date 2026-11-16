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
from websockets.datastructures import Headers
from websockets.exceptions import InvalidStatus
from websockets.http11 import Response

from .support import ClientSocket, Input, RealtimeCall, RealtimeRunner, SslValue

RUNNER: RealtimeRunner = OpenAIRealtime()
REAL_WEBSOCKETS: ModuleType = sys.modules["websockets"]


class ProxyClient(ClientSocket):
    def __init__(self, input_value: Input) -> None:
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {
            "headers": [
                (name.encode("latin-1"), value.encode("latin-1"))
                for name, value in input_value["scope_headers"]
            ]
        }
        self.close_error = input_value["close_error"]
        self.sent: list[str] = []
        self.closed: dict[str, JsonValue] | None = None

    @override
    async def send_text(self, data: str) -> None:
        self.sent.append(data)

    @override
    async def receive_text(self) -> str:
        raise ConnectionError("the proxy client never sends a frame")

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.closed = {"code": code, "reason": reason}
        if self.close_error is not None:
            raise RuntimeError(self.close_error)


# The upstream refuses the handshake with the case's status, or fails with "stop" once it is open.
class Upstream:
    def __init__(self, handshake_status: int | None) -> None:
        self.handshake_status = handshake_status

    async def __aenter__(self) -> Self:
        if self.handshake_status is None:
            raise RuntimeError("stop")
        raise InvalidStatus(Response(self.handshake_status, "Handshake rejected", Headers()))

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...


class FakeWebsockets(ModuleType):
    exceptions: ModuleType
    frames: ModuleType
    ClientConnection: type[websockets_client.ClientConnection]

    def __init__(self, handshake_status: int | None) -> None:
        super().__init__("websockets")
        self.exceptions = websockets_exceptions
        self.frames = websockets_frames
        self.ClientConnection = websockets_client.ClientConnection
        self.handshake_status = handshake_status
        self.session: dict[str, JsonValue] = {}

    def connect(
        self,
        url: str,
        *,
        additional_headers: dict[str, str],
        max_size: int | None,
        ssl: SslValue,
    ) -> Upstream:
        headers: list[JsonValue] = [
            [name, value] for name, value in sorted(additional_headers.items())
        ]
        ssl_kind = "none" if ssl is None else "insecure" if ssl is True else "shared"
        self.session = {"url": url, "headers": headers, "max_size": max_size, "ssl": ssl_kind}
        return Upstream(self.handshake_status)


async def probe(input_value: Input, upstream: FakeWebsockets, client: ProxyClient) -> None:
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
        "websocket": client,
        "logging_obj": logging_obj,
        "api_base": input_value["api_base"],
        "api_key": input_value["api_key"],
        "timeout": None,
        "query_params": input_value["query_params"],
        "litellm_metadata": None,
    }
    sys.modules["websockets"] = upstream
    try:
        await RUNNER.async_realtime(**call)
    finally:
        sys.modules["websockets"] = REAL_WEBSOCKETS


def realtime_answer(input_value: Input) -> dict[str, JsonValue]:
    upstream = FakeWebsockets(input_value["handshake_status"])
    client = ProxyClient(input_value)
    asyncio.run(probe(input_value, upstream, client))
    return {"session": upstream.session, "closed": client.closed}
