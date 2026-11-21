import asyncio
import ssl as ssl_module
import sys
from datetime import UTC, datetime
from types import ModuleType, TracebackType
from typing import Final, Literal, Protocol, Self, TypedDict, TypeAlias

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.realtime import RealtimeQueryParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override
from websockets import exceptions as ws_exceptions
from websockets.datastructures import Headers
from websockets.http11 import Response

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ProviderDocument(TypedDict):
    kind: Literal["setup", "deferred", "error"]


class ConnectDial(TypedDict):
    kind: Literal["connect"]


class MessageDial(TypedDict):
    kind: Literal["timeout", "os", "websocket"]
    message: str


class StatusDial(TypedDict):
    kind: Literal["invalid_status"]
    status_code: int


class StatusCodeDial(TypedDict):
    kind: Literal["invalid_status_code"]
    status_code: int
    message: str


DialDocument: TypeAlias = ConnectDial | MessageDial | StatusDial | StatusCodeDial


class Input(TypedDict):
    provider: ProviderDocument
    model: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str]
    ssl_verify: bool
    dials: list[DialDocument]
    send_error: str | None
    close_error: dict[str, str] | None
    intent: str | None


class ClientPeer(Protocol):
    async def send_text(self, data: str) -> None: ...

    async def receive_text(self) -> str: ...

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


class ProbeClient(ClientPeer):
    def __init__(self, input_value: Input) -> None:
        self.sent: list[str] = []
        self.closed: dict[str, JsonValue] | None = None
        self._send_error = input_value["send_error"]
        self._close_error = input_value["close_error"]

    @override
    async def send_text(self, data: str) -> None:
        if self._send_error is not None:
            raise RuntimeError(self._send_error)
        self.sent.append(data)

    @override
    async def receive_text(self) -> str:
        raise RuntimeError("the scripted client disconnected")

    @override
    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.closed = {"code": code, "reason": reason}
        document = self._close_error
        if document is None:
            return
        if document["error_class"] == "RuntimeError":
            raise RuntimeError(document["message"])
        raise OSError(document["message"])


class BackendPeer(Protocol):
    async def __aenter__(self) -> Self: ...

    async def __aexit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...

    async def send(self, message: str) -> None: ...

    async def recv(self, *, _decode: bool | None = None) -> str: ...

    async def close(self) -> None: ...


class ProbeBackend(BackendPeer):
    def __init__(self) -> None:
        self.sent: list[str] = []
        self.closed = False

    @override
    async def __aenter__(self) -> Self:
        return self

    @override
    async def __aexit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.closed = True

    @override
    async def send(self, message: str) -> None:
        self.sent.append(message)

    @override
    async def recv(self, *, _decode: bool | None = None) -> str:
        return await asyncio.Future[str]()

    @override
    async def close(self) -> None:
        self.closed = True


class InvalidStatusCodeError(ws_exceptions.WebSocketException): ...


class FakeExceptions:
    WebSocketException: type[ws_exceptions.WebSocketException] = ws_exceptions.WebSocketException
    ConnectionClosed: type[ws_exceptions.ConnectionClosed] = ws_exceptions.ConnectionClosed
    InvalidStatus: type[ws_exceptions.InvalidStatus] = ws_exceptions.InvalidStatus
    InvalidStatusCode: type[InvalidStatusCodeError] = InvalidStatusCodeError


class FakeWebsockets(ModuleType):
    def __init__(self, backend: ProbeBackend, script: list[DialDocument]) -> None:
        super().__init__("websockets")
        self.exceptions = FakeExceptions
        self._backend = backend
        self._script = script
        self.dials: list[JsonValue] = []

    async def connect(
        self,
        url: str,
        *,
        additional_headers: dict[str, str],
        max_size: int | None,
        ssl: bool | ssl_module.SSLContext,
        open_timeout: float,
    ) -> ProbeBackend:
        record = dial_record(url, additional_headers, ssl, max_size, open_timeout)
        self.dials.append(JSON.validate_python(record))
        index = len(self.dials) - 1
        attempt: DialDocument = MessageDial(kind="os", message="the dial script ran out")
        if index < len(self._script):
            attempt = self._script[index]
        if attempt["kind"] == "connect":
            if url.startswith("ws://"):
                raise ValueError("ssl argument is incompatible with a ws:// URI")
            return self._backend
        if attempt["kind"] == "invalid_status":
            raise ws_exceptions.InvalidStatus(Response(attempt["status_code"], "", Headers(), b""))
        if attempt["kind"] == "invalid_status_code":
            raise InvalidStatusCodeError(attempt["message"])
        if attempt["kind"] == "timeout":
            raise TimeoutError(attempt["message"])
        if attempt["kind"] == "websocket":
            raise ws_exceptions.WebSocketException(attempt["message"])
        raise OSError(attempt["message"])


def dial_record(
    url: str,
    additional_headers: dict[str, str],
    ssl: bool | ssl_module.SSLContext,
    max_size: int | None,
    open_timeout: float,
) -> dict[str, JsonValue]:
    if ssl is False:
        ssl_value = "disabled"
    elif ssl is True:
        ssl_value = "shared"
    elif ssl.verify_mode == ssl_module.CERT_NONE and not ssl.check_hostname:
        ssl_value = "insecure"
    else:
        ssl_value = "shared"
    return {
        "url": url,
        "headers": [[name, value] for name, value in sorted(additional_headers.items())],
        "ssl": ssl_value,
        "max_size": max_size,
        "open_timeout": open_timeout,
    }


def frame(payload: str) -> JsonValue:
    try:
        value = JSON.validate_json(payload)
    except ValidationError:
        return JSON.validate_python({"kind": "text", "text": payload})
    return JSON.validate_python({"kind": "json", "value": value})


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


def provider_config(input_value: Input) -> BaseRealtimeConfig:
    kind = input_value["provider"]["kind"]
    provider = LlmProviders.META if kind == "error" else LlmProviders.GEMINI
    config = ProviderConfigManager.get_provider_realtime_config(input_value["model"], provider)
    if config is None:
        raise ValueError("the case names a model without a realtime config")
    return config


def snapshot(client: ProbeClient, backend: ProbeBackend, dials: list[JsonValue]) -> JsonValue:
    return JSON.validate_python({
        "dials": JSON.validate_python(dials),
        "backend": JSON.validate_python([frame(text) for text in backend.sent]),
        "backend_closed": backend.closed,
        "client": JSON.validate_python([frame(text) for text in client.sent]),
        "client_close": None if client.closed is None else JSON.validate_python(client.closed),
    })


class RealtimeRunner(Protocol):
    async def async_realtime(
        self, *, model: str, websocket: ClientPeer, logging_obj: Logging,
        provider_config: BaseRealtimeConfig, headers: dict[str, str],
        api_base: str | None = None, api_key: str | None = None,
        query_params: RealtimeQueryParams | None = None,
    ) -> None: ...


RUNNER: Final[RealtimeRunner] = BaseLLMHTTPHandler()


def run(input_value: Input) -> JsonValue:
    backend = ProbeBackend()
    client = ProbeClient(input_value)
    fake = FakeWebsockets(backend, input_value["dials"])
    intent = input_value["intent"]
    previous_module = sys.modules["websockets"]
    previous_defer = litellm.gemini_live_defer_setup
    previous_ssl_verify = litellm.ssl_verify
    litellm.gemini_live_defer_setup = input_value["provider"]["kind"] == "deferred"
    litellm.ssl_verify = input_value["ssl_verify"]
    sys.modules["websockets"] = fake
    try:
        asyncio.run(
            RUNNER.async_realtime(
                model=input_value["model"],
                websocket=client,
                logging_obj=logging_object(input_value),
                provider_config=provider_config(input_value),
                headers=input_value["headers"],
                api_base=input_value["api_base"],
                api_key=input_value["api_key"],
                query_params=None if intent is None else RealtimeQueryParams(intent=intent),
            )
        )
    finally:
        sys.modules["websockets"] = previous_module
        litellm.gemini_live_defer_setup = previous_defer
        litellm.ssl_verify = previous_ssl_verify
    return snapshot(client, backend, fake.dials)
