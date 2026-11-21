import asyncio
import ssl as ssl_module
import sys
from datetime import UTC, datetime
from types import ModuleType, TracebackType
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.realtime import RealtimeQueryParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import Unpack
from websockets import exceptions as websockets_exceptions
from websockets.datastructures import Headers
from websockets.http11 import Response

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
MAX_DIALS: Final = 8


class ProviderDocument(TypedDict):
    kind: Literal["setup", "deferred", "error"]


class DialDocument(TypedDict, total=False):
    kind: Literal["connect", "timeout", "os", "websocket", "invalid_status", "invalid_status_code"]
    message: str
    status_code: int


class CloseErrorDocument(TypedDict):
    error_class: str
    message: str


class Input(TypedDict):
    provider: ProviderDocument
    model: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str]
    ssl_verify: bool
    dials: list[DialDocument]
    send_error: str | None
    close_error: CloseErrorDocument | None
    intent: str | None


class InvalidStatusCodeError(websockets_exceptions.WebSocketException):
    def __init__(self, status_code: int, message: str) -> None:
        super().__init__(message)
        self.status_code: int = status_code


class FakeExceptions:
    WebSocketException: type[websockets_exceptions.WebSocketException] = (
        websockets_exceptions.WebSocketException
    )
    ConnectionClosed: type[websockets_exceptions.ConnectionClosed] = (
        websockets_exceptions.ConnectionClosed
    )
    InvalidStatus: type[websockets_exceptions.InvalidStatus] = websockets_exceptions.InvalidStatus
    InvalidStatusCode: type[InvalidStatusCodeError] = InvalidStatusCodeError


class ProbeClient:
    def __init__(self, input_value: Input) -> None:
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {"headers": []}
        self.sent: list[str] = []
        self.closed: dict[str, JsonValue] | None = None
        self._send_error = input_value["send_error"]
        self._close_error = input_value["close_error"]

    async def send_text(self, data: str) -> None:
        if self._send_error is not None:
            raise RuntimeError(self._send_error)
        self.sent.append(data)

    async def receive_text(self) -> str:
        raise RuntimeError("the scripted client disconnected")

    async def close(self, code: int = 1000, reason: str | None = None) -> None:
        self.closed = {"code": code, "reason": reason}
        document = self._close_error
        if document is None:
            return
        if document["error_class"] == "RuntimeError":
            raise RuntimeError(document["message"])
        raise OSError(document["message"])


class ProbeBackend:
    def __init__(self) -> None:
        self.sent: list[str | bytes] = []
        self.closed = False

    async def __aenter__(self) -> "ProbeBackend":
        return self

    async def __aexit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.closed = True

    async def send(self, message: str | bytes) -> None:
        self.sent.append(message)

    async def recv(self, _decode: bool | None = None) -> str | bytes:
        future: asyncio.Future[str] = asyncio.Future()
        return await future

    async def close(self) -> None:
        self.closed = True


class FakeWebsockets(ModuleType):
    exceptions: type[FakeExceptions]

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
        ssl: bool | str | ssl_module.SSLContext,
        open_timeout: float,
    ) -> ProbeBackend:
        self.dials.append(
            JSON.validate_python(
                {
                    "url": url,
                    "headers": [[name, value] for name, value in sorted(additional_headers.items())],
                    "ssl": ssl_kind(value=ssl),
                    "max_size": max_size,
                    "open_timeout": open_timeout,
                }
            )
        )
        attempt: DialDocument
        if len(self.dials) <= len(self._script):
            attempt = self._script[len(self.dials) - 1]
        else:
            attempt = {"kind": "os", "message": "the dial script ran out"}
        kind = attempt["kind"]
        if kind == "connect":
            if url.startswith("ws://"):
                raise ValueError("ssl argument is incompatible with a ws:// URI")
            return self._backend
        if kind == "invalid_status":
            raise websockets_exceptions.InvalidStatus(
                Response(attempt["status_code"], "", Headers(), b"")
            )
        if kind == "invalid_status_code":
            raise InvalidStatusCodeError(attempt["status_code"], attempt["message"])
        if kind == "timeout":
            raise TimeoutError(attempt["message"])
        if kind == "websocket":
            raise websockets_exceptions.WebSocketException(attempt["message"])
        raise OSError(attempt["message"])


def ssl_kind(*, value: bool | str | ssl_module.SSLContext) -> str:
    match value:
        case False:
            return "disabled"
        case ssl_module.SSLContext():
            if value.verify_mode == ssl_module.CERT_NONE and not value.check_hostname:
                return "insecure"
            return "shared"
        case True | str():
            return "shared"


def frame(payload: str | bytes) -> JsonValue:
    match payload:
        case str():
            text = payload
        case bytes():
            text = payload.decode()
    try:
        value = JSON.validate_json(text)
    except ValidationError:
        return JSON.validate_python({"kind": "text", "text": text})
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
    if input_value["provider"]["kind"] == "error":
        config = ProviderConfigManager.get_provider_realtime_config(
            input_value["model"], LlmProviders.META
        )
    else:
        config = ProviderConfigManager.get_provider_realtime_config(
            input_value["model"], LlmProviders.GEMINI
        )
    if config is None:
        raise ValueError("the case names a model without a realtime config")
    return config


def query_params(input_value: Input) -> RealtimeQueryParams | None:
    intent = input_value["intent"]
    if intent is None:
        return None
    return {"intent": intent}


class RealtimeCall(TypedDict):
    model: str
    websocket: ProbeClient
    logging_obj: Logging
    provider_config: BaseRealtimeConfig
    headers: dict[str, str]
    api_base: str | None
    api_key: str | None
    client: None
    timeout: float | None
    user_api_key_dict: None
    litellm_metadata: None
    query_params: RealtimeQueryParams | None


class RealtimeRunner(Protocol):
    async def async_realtime(self, **call: Unpack[RealtimeCall]) -> None: ...


RUNNER: RealtimeRunner = BaseLLMHTTPHandler()


def snapshot(
    client: ProbeClient, backend: ProbeBackend, dials: list[JsonValue]
) -> JsonValue:
    client_frames: list[JsonValue] = [frame(text) for text in client.sent]
    backend_frames: list[JsonValue] = [frame(text) for text in backend.sent]
    close: JsonValue = None if client.closed is None else JSON.validate_python(client.closed)
    payload: dict[str, JsonValue] = {
        "dials": JSON.validate_python(dials),
        "backend": JSON.validate_python(backend_frames),
        "backend_closed": backend.closed,
        "client": JSON.validate_python(client_frames),
        "client_close": close,
    }
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    if len(input_value["dials"]) > MAX_DIALS:
        raise ValueError("the connection input is past its dial limit")
    backend = ProbeBackend()
    client = ProbeClient(input_value)
    fake = FakeWebsockets(backend, input_value["dials"])
    call: RealtimeCall = {
        "model": input_value["model"],
        "websocket": client,
        "logging_obj": logging_object(input_value),
        "provider_config": provider_config(input_value),
        "headers": input_value["headers"],
        "api_base": input_value["api_base"],
        "api_key": input_value["api_key"],
        "client": None,
        "timeout": None,
        "user_api_key_dict": None,
        "litellm_metadata": None,
        "query_params": query_params(input_value),
    }
    previous_module = sys.modules["websockets"]
    previous_defer = litellm.gemini_live_defer_setup
    previous_ssl_verify = litellm.ssl_verify
    litellm.gemini_live_defer_setup = input_value["provider"]["kind"] == "deferred"
    litellm.ssl_verify = input_value["ssl_verify"]
    sys.modules["websockets"] = fake
    try:
        asyncio.run(RUNNER.async_realtime(**call))
    finally:
        sys.modules["websockets"] = previous_module
        litellm.gemini_live_defer_setup = previous_defer
        litellm.ssl_verify = previous_ssl_verify
    return snapshot(client, backend, fake.dials)
