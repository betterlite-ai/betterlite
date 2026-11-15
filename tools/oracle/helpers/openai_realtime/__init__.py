import asyncio
import ssl as ssl_module
import sys
from datetime import UTC, datetime
from types import ModuleType, TracebackType
from typing import Self

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.realtime.handler import OpenAIRealtime
from litellm.types.realtime import RealtimeQueryParams
from pydantic import BaseModel, ConfigDict, JsonValue
from typing_extensions import override
from websockets import exceptions as websockets_exceptions
from websockets import frames as websockets_frames
from websockets.asyncio import client as websockets_client
from websockets.datastructures import Headers
from websockets.exceptions import InvalidStatus
from websockets.http11 import Response

from .http import http_answer
from .types import (
    HEADERS,
    JSON,
    ClientSocket,
    CloseFrame,
    FailMode,
    FailureSetup,
    Input,
    QuerySetup,
    RealtimeCall,
    RealtimeRunner,
    SslKind,
)

RUNNER: RealtimeRunner = OpenAIRealtime()
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
        self.mode: FailMode = mode
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
        additional_headers: dict[str, str],
        max_size: int | None,
        ssl: bool | str | ssl_module.SSLContext | None,
    ) -> ProbeConnection:
        return self.server.connect(
            url, additional_headers=additional_headers, max_size=max_size, ssl=ssl
        )


def ssl_kind(*, value: bool | str | ssl_module.SSLContext | None) -> SslKind:
    if value is None:
        return "none"
    if value is True:
        return "insecure"
    return "shared"


class PreCallDetails(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    additional_args: dict[str, JsonValue]


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: PreCallDetails


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


def pre_call_view(logging_obj: Logging) -> JsonValue:
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    additional = details.additional_args
    headers = HEADERS.validate_python(additional["headers"])
    body = HEADERS.validate_python(additional["complete_input_dict"])
    names = sorted(headers)
    payload: dict[str, JsonValue] = {
        "api_base": additional["api_base"],
        "header_names": JSON.validate_python(names),
        "openai_beta": headers.get("OpenAI-Beta"),
        "query_params": body["query_params"],
    }
    return JSON.validate_python(payload)


async def probe_call(
    input_value: Input,
    server: ProbeServer,
    api_base: str | None,
    query_params: RealtimeQueryParams | None,
) -> Logging:
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
    sys.modules["websockets"] = FakeWebsockets(server)
    try:
        call: RealtimeCall = {
            "model": input_value["model"],
            "websocket": server.client,
            "logging_obj": logging_obj,
            "api_base": api_base,
            "api_key": input_value["api_key"],
            "timeout": None,
            "query_params": query_params,
            "litellm_metadata": None,
        }
        await RUNNER.async_realtime(**call)
    finally:
        sys.modules["websockets"] = REAL_WEBSOCKETS
    return logging_obj


async def connect_one(input_value: Input, setup: QuerySetup) -> JsonValue:
    server = ProbeServer(input_value["scope_headers"], None, "error")
    logging_obj = await probe_call(
        input_value, server, setup["api_base"], setup["query_params"]
    )
    payload: dict[str, JsonValue] = {
        "url": server.url,
        "headers": header_pairs(server.headers),
        "max_size": server.max_size,
        "ssl": server.ssl_kind,
        "pre_call": pre_call_view(logging_obj),
    }
    return JSON.validate_python(payload)


async def failure_probe(input_value: Input, failure: FailureSetup) -> JsonValue:
    mode: FailMode = "invalid_status" if input_value["kind"] == "handshake" else "error"
    server = ProbeServer(input_value["scope_headers"], failure["close_error"], mode)
    server.failure = failure
    await probe_call(input_value, server, None, None)
    payload: dict[str, JsonValue] = {
        "sent": [JSON.validate_json(text) for text in server.client.sent],
        "closed": frame_json(server.client.closed),
        "close_raised": server.client.close_raised,
    }
    return JSON.validate_python(payload)


def connect_answer(input_value: Input) -> JsonValue:
    connects = [
        asyncio.run(connect_one(input_value, setup)) for setup in input_value["queries"]
    ]
    return {"connects": connects}


def failure_answer(input_value: Input) -> JsonValue:
    failures = [
        asyncio.run(failure_probe(input_value, failure))
        for failure in input_value["failures"]
    ]
    return {"failures": failures}


def run(input_value: Input) -> JsonValue:
    kind = input_value["kind"]
    if kind == "http":
        return http_answer(input_value)
    if kind == "connect":
        return connect_answer(input_value)
    return failure_answer(input_value)
