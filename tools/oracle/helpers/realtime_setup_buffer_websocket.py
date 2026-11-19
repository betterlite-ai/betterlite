import asyncio
import socketserver
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

from litellm.litellm_core_utils import realtime_streaming
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig, RealtimeBackend
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import ServerConnection, serve

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MAX_OPERATIONS = 256
MAX_FRAMES = 64
MAX_TEXT_BYTES = 16 * 1024
MAX_CAPTURE_BYTES = 1024 * 1024
SETTLE_SECONDS = 10.0
DEADLINE_SECONDS = 120.0


class ProviderDocument(TypedDict):
    requires_session_configuration: bool
    fail_transform: bool
    dropped: list[str]


class BareOperation(TypedDict):
    kind: Literal["deferred", "flush"]


class TextOperation(TypedDict):
    kind: Literal["should_buffer", "buffer"]
    text: str


class ArrivalOperation(TypedDict):
    kind: Literal["flush_with_arrival"]
    text: str


class SwitchOperation(TypedDict):
    kind: Literal["complete", "flushing"]
    value: bool


Operation: TypeAlias = BareOperation | TextOperation | ArrivalOperation | SwitchOperation


class Input(TypedDict):
    provider: ProviderDocument | None
    backend_failures: list[int]
    operations: list[Operation]


class SendFailureError(Exception): ...


class ProviderMethods(Protocol):
    def is_setup_message(self, _msg_obj: dict[str, JsonValue]) -> bool: ...

    def is_content_message(self, _msg_obj: dict[str, JsonValue]) -> bool: ...

    def requires_session_configuration(self) -> bool: ...


def operation_text(operation: Operation) -> str | None:
    text: str | None = None
    match operation["kind"]:
        case "should_buffer":
            text = operation["text"]
        case "buffer":
            text = operation["text"]
        case "flush_with_arrival":
            text = operation["text"]
        case "deferred" | "flush" | "complete" | "flushing":
            text = None
    return text


def checked(input_value: Input) -> None:
    provider = input_value["provider"]
    dropped: list[str] = [] if provider is None else provider["dropped"]
    texts = [operation_text(operation) for operation in input_value["operations"]]
    sizes = [len(text.encode("utf-8")) for text in [*texts, *dropped] if text is not None]
    if (
        len(input_value["operations"]) > MAX_OPERATIONS
        or len(input_value["backend_failures"]) > MAX_FRAMES
        or any(size > MAX_TEXT_BYTES for size in sizes)
        or sum(sizes) > MAX_CAPTURE_BYTES
    ):
        raise ValueError("the scripted setup buffer input is past its limits")


def recorded(wire: str, data: bytes) -> dict[str, JsonValue]:
    try:
        payload: JsonValue = {"kind": "json", "value": WIRE.validate_json(data)}
    except ValidationError:
        payload = {"kind": "text", "text": data.decode("utf-8", errors="replace")}
    return {"wire": wire, "payload": payload}


class ScriptedProvider(ProviderMethods):
    def __init__(self, document: ProviderDocument) -> None:
        self._document = document

    def transform_realtime_request(
        self, message: str, _model: str, _session_request: str | None = None
    ) -> list[str | bytes]:
        if self._document["fail_transform"]:
            raise SendFailureError("transform")
        if message in self._document["dropped"]:
            return []
        return [message]

    @override
    def is_setup_message(self, _msg_obj: dict[str, JsonValue]) -> bool:
        return False

    @override
    def is_content_message(self, _msg_obj: dict[str, JsonValue]) -> bool:
        return False

    @override
    def requires_session_configuration(self) -> bool:
        return self._document["requires_session_configuration"]


class WireBackend:
    def __init__(self, socket: ClientConnection, failures: list[int]) -> None:
        self.attempts = 0
        self.delivered = 0
        self.failures = failures
        self.blocked = asyncio.Event()
        self.release = asyncio.Event()
        self.blocked_attempt = failures[0] if failures else None
        self._socket = socket

    async def send(self, message: str | bytes) -> None:
        attempt = self.attempts
        if attempt == self.blocked_attempt:
            self.blocked.set()
            await self.release.wait()
        self.attempts += 1
        if attempt in self.failures:
            raise SendFailureError("backend")
        await self._socket.send(message)
        self.delivered += 1


class Upstream:
    def __init__(self) -> None:
        self.frames: list[JsonValue] = []
        self.arrived = asyncio.Event()

    async def __call__(self, connection: ServerConnection) -> None:
        async for message in connection:
            match message:
                case str():
                    self.frames.append(recorded("text", message.encode("utf-8")))
                case bytes():
                    self.frames.append(
                        {"wire": "binary", "payload": {"kind": "bytes", "bytes": list(message)}}
                    )
            self.arrived.set()

    async def wait_for(self, delivered: int) -> None:
        while len(self.frames) < delivered:
            self.arrived.clear()
            await self.arrived.wait()


class Probe(realtime_streaming.RealTimeStreaming):
    def __init__(self, backend: WireBackend, provider: ScriptedProvider | None) -> None:
        backend_argument = cast(RealtimeBackend, backend)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L26-L40
        provider_argument = cast(BaseRealtimeConfig | None, provider)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L43-L127
        logging_argument = cast(LiteLLMLogging, None)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127-L144
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127-L144
            None,
            backend_argument,
            logging_argument,
            provider_config=provider_argument,
            model="scripted",
        )

    def deferred(self) -> bool:
        return self._uses_deferred_backend_setup()

    def should_buffer(self, text: str) -> bool:
        return self._should_buffer_client_message_until_setup(text)

    def buffer(self, text: str) -> None:
        self._buffer_pending_message_until_setup(text)

    async def flush(self) -> bool:
        return await self._flush_pending_messages_until_setup()

    def complete(self, *, value: bool) -> None:
        self._backend_setup_complete = value

    def flushing(self, *, value: bool) -> None:
        self._flushing_pending_messages_until_setup = value

    def pending(self) -> list[str]:
        return self._pending_messages_until_setup

    def facts(self) -> dict[str, JsonValue]:
        return {
            "deferred": self._uses_deferred_backend_setup(),
            "setup_complete": self._backend_setup_complete,
            "flushing": self._flushing_pending_messages_until_setup,
            "pending_count": len(self._pending_messages_until_setup),
            "pending_bytes": self._pending_messages_byte_total,
        }


async def apply(probe: Probe, backend: WireBackend, operation: Operation) -> JsonValue:
    result: JsonValue = None
    try:
        match operation["kind"]:
            case "deferred":
                result = {"ok": probe.deferred()}
            case "should_buffer":
                result = {"ok": probe.should_buffer(operation["text"])}
            case "buffer":
                probe.buffer(operation["text"])
                result = {"ok": True}
            case "complete":
                probe.complete(value=operation["value"])
                result = {"ok": True}
            case "flushing":
                probe.flushing(value=operation["value"])
                result = {"ok": True}
            case "flush":
                result = {"ok": await probe.flush()}
            case "flush_with_arrival":
                flush_task = asyncio.create_task(probe.flush())
                waiter = asyncio.create_task(backend.blocked.wait())
                await asyncio.wait({flush_task, waiter}, return_when=asyncio.FIRST_COMPLETED)
                probe.buffer(operation["text"])
                backend.release.set()
                waiter.cancel()
                result = {"ok": await flush_task}
    except AttributeError:
        result = {"error": "non-object"}
    return result


async def session(
    input_value: Input, upstream: Upstream, socket: ClientConnection
) -> JsonValue:
    document = input_value["provider"]
    provider = None if document is None else ScriptedProvider(document)
    backend = WireBackend(socket, input_value["backend_failures"])
    probe = Probe(backend, provider)
    results: list[JsonValue] = []
    states: list[JsonValue] = []
    for operation in input_value["operations"]:
        results.append(await apply(probe, backend, operation))
        await asyncio.wait_for(upstream.wait_for(backend.delivered), SETTLE_SECONDS)
        states.append(probe.facts() | {"upstream": len(upstream.frames)})
    pending = [recorded("text", text.encode("utf-8"))["payload"] for text in probe.pending()]
    payload: dict[str, JsonValue] = {
        "results": results,
        "states": states,
        "pending": pending,
        "upstream": upstream.frames,
    }
    return WIRE.validate_python(payload)


async def bounded(
    input_value: Input, upstream: Upstream, server: socketserver.TCPServer
) -> JsonValue:
    url = f"ws://127.0.0.1:{server.server_address[1]}"
    async with serve(upstream, sock=server.socket), connect(url) as socket:
        call = session(input_value, upstream, socket)
        return await asyncio.wait_for(call, DEADLINE_SECONDS)


def run(input_value: Input) -> JsonValue:
    checked(input_value)
    upstream = Upstream()
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as server:
        return asyncio.run(bounded(input_value, upstream, server))
