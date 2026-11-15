import asyncio
import ssl as ssl_module
import sys
from datetime import UTC, datetime
from types import ModuleType
from typing import Literal, NoReturn, Protocol, TypeAlias, TypedDict, Unpack

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.xai.realtime.handler import XAIRealtime
from litellm.llms.xai.realtime.transformation import XAIRealtimeNormalizer
from litellm.types.realtime import RealtimeQueryParams
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override
from websockets import exceptions as websockets_exceptions
from websockets.asyncio import client as websockets_client

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
SslKind: TypeAlias = Literal["none", "insecure", "shared"]


class Input(TypedDict):
    kind: Literal["sequence", "connection"]
    events: list[dict[str, JsonValue]]
    sessions: list[dict[str, JsonValue]]
    model: str
    api_key: str | None
    scope_headers: list[tuple[str, str]]
    max_message_size_bytes: int


class Normalizer(Protocol):
    def should_drop(self, event: JsonValue) -> bool: ...

    def normalize(self, event: dict[str, JsonValue]) -> JsonValue: ...

    def patch_outgoing_session(self, session: dict[str, JsonValue]) -> JsonValue: ...


class ClientSocket(Protocol):
    scope: dict[str, list[tuple[bytes, bytes]]]

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


# the handler only matches this protocol when the call names timeout and litellm_metadata too
class RealtimeCall(TypedDict):
    model: str
    websocket: ClientSocket
    logging_obj: Logging
    api_base: str | None
    api_key: str | None
    timeout: float | None
    query_params: RealtimeQueryParams | None
    litellm_metadata: dict[str, JsonValue] | None


class RealtimeRunner(Protocol):
    async def async_realtime(self, **call: Unpack[RealtimeCall]) -> None: ...


class ProbeClient(ClientSocket):
    def __init__(self, scope_headers: list[tuple[str, str]]) -> None:
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {
            "headers": [
                (name.encode("latin-1"), value.encode("latin-1"))
                for name, value in scope_headers
            ]
        }
        self.url: str | None = None
        self.headers: dict[str, str] | None = None
        self.max_size: int | None = None
        self.ssl_kind: SslKind | None = None
        self.closed: dict[str, JsonValue] | None = None

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.closed = {"code": code, "reason": reason}


class FakeWebsockets(ModuleType):
    exceptions: ModuleType
    ClientConnection: type[websockets_client.ClientConnection]

    def __init__(self, socket: ProbeClient) -> None:
        super().__init__("websockets")
        self.exceptions = websockets_exceptions
        self.ClientConnection = websockets_client.ClientConnection
        self.socket = socket

    def connect(
        self,
        url: str,
        *,
        additional_headers: dict[str, str],
        max_size: int | None,
        ssl: bool | str | ssl_module.SSLContext | None,
    ) -> NoReturn:
        self.socket.url = url
        self.socket.headers = additional_headers
        self.socket.max_size = max_size
        self.socket.ssl_kind = ssl_kind(value=ssl)
        raise RuntimeError("the probe refuses the upstream connection")


RUNNER: RealtimeRunner = XAIRealtime()


def ssl_kind(*, value: bool | str | ssl_module.SSLContext | None) -> SslKind:
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


def logging_object(input_value: Input) -> Logging:
    return Logging(
        model=input_value["model"],
        messages=None,
        stream=False,
        call_type="realtime",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )


def sequence_answer(current: Normalizer, input_value: Input) -> JsonValue:
    emitted: list[JsonValue] = []
    for event in input_value["events"]:
        if current.should_drop(event):
            continue
        emitted.append(current.normalize(event))
    sessions: list[JsonValue] = [
        current.patch_outgoing_session(session) for session in input_value["sessions"]
    ]
    answer: dict[str, JsonValue] = {"emitted": emitted, "sessions": sessions}
    return JSON.validate_python(answer)


def connection_answer(input_value: Input) -> JsonValue:
    api_key = input_value["api_key"]
    if api_key is None:
        raise ValueError("a connection case names the api key")
    socket = ProbeClient(input_value["scope_headers"])
    previous = sys.modules["websockets"]
    sys.modules["websockets"] = FakeWebsockets(socket)
    try:
        call: RealtimeCall = {
            "model": input_value["model"],
            "websocket": socket,
            "logging_obj": logging_object(input_value),
            "api_base": None,
            "api_key": api_key,
            "timeout": None,
            "query_params": None,
            "litellm_metadata": None,
        }
        asyncio.run(RUNNER.async_realtime(**call))
    finally:
        sys.modules["websockets"] = previous
    payload: dict[str, JsonValue] = {
        "url": socket.url,
        "headers": header_pairs(socket.headers),
        "max_size": socket.max_size,
        "ssl": socket.ssl_kind,
    }
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "sequence":
        return sequence_answer(XAIRealtimeNormalizer(), input_value)
    return connection_answer(input_value)
