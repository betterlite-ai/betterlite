import asyncio
import socketserver
from typing import Literal, NotRequired, Protocol, TypedDict, cast

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import realtime_streaming
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig, RealtimeBackend
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import ServerConnection, serve

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EVENT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
SETTLE_SECONDS = 10.0
DEADLINE_SECONDS = 120.0
MAX_OPERATIONS = 32
MAX_FRAMES = 64
MAX_TEXT_BYTES = 16 * 1024
MAX_CAPTURE_BYTES = 1024 * 1024


class Frame(TypedDict):
    kind: Literal["text", "binary"]
    text: NotRequired[str]
    bytes: NotRequired[list[int]]


class ProviderDocument(TypedDict):
    requires_session_configuration: bool
    setup: list[JsonValue]
    content: list[JsonValue]
    transforms: list[list[Frame]]
    fail_transform: bool


class GuardrailDocument(TypedDict):
    custom: bool
    runs_for: list[str]


class Operation(TypedDict):
    kind: str
    text: NotRequired[str]


class Input(TypedDict):
    model: str
    force_transcription_model: str | None
    client_wants_beta: bool
    backend_uses_beta_protocol: bool | None
    provider: ProviderDocument | None
    guardrails: list[GuardrailDocument]
    backend_failures: list[int]
    client_failures: list[int]
    operations: list[Operation]


def checked(input_value: Input) -> None:
    provider = input_value["provider"]
    batches = [] if provider is None else provider["transforms"]
    frames = [frame for batch in batches for frame in batch]
    texts = [operation["text"] for operation in input_value["operations"] if "text" in operation]
    sizes = [len(text) for text in texts] + [
        len(frame["text"]) if frame["kind"] == "text" else len(frame["bytes"])
        for frame in frames
    ]
    if (
        len(texts) > MAX_OPERATIONS
        or len(frames) > MAX_FRAMES
        or len(input_value["guardrails"]) > MAX_FRAMES
        or len(input_value["backend_failures"]) > MAX_FRAMES
        or len(input_value["client_failures"]) > MAX_FRAMES
        or any(size > MAX_TEXT_BYTES for size in sizes)
        or sum(sizes) > MAX_CAPTURE_BYTES
    ):
        raise ValueError("the scripted sender input is past its limits")


class SendFailureError(Exception): ...


class WireLog:
    def __init__(self, failures: list[int]) -> None:
        self.attempts = 0
        self.delivered = 0
        self.sent: list[str] = []
        self.failures = failures

    def begin(self, label: str) -> None:
        attempt = self.attempts
        self.attempts += 1
        if attempt in self.failures:
            raise SendFailureError(label)


def recorded(wire: str, data: bytes) -> dict[str, JsonValue]:
    try:
        payload: JsonValue = {"kind": "json", "value": WIRE.validate_json(data)}
    except ValidationError:
        payload = {"kind": "text", "text": data.decode("utf-8", errors="replace")}
    return {"wire": wire, "payload": payload}


def decoded(text: str) -> JsonValue:
    try:
        return WIRE.validate_json(text)
    except ValidationError:
        return text


class ScriptedNormalizer:
    def should_drop(self, event: JsonValue) -> bool:
        try:
            kind: JsonValue = EVENT.validate_python(event).get("type")
        except ValidationError:
            return False
        return kind == "conversation.item.added"

    def normalize(self, event: dict[str, JsonValue]) -> dict[str, JsonValue]:
        if event.get("type") == "response.output_text.done":
            event["type"] = "response.text.done"
        return event


class ScriptedProviderView(Protocol):
    # the provider methods the sender calls by name; the rest of the provider enters Probe's cast
    def is_setup_message(self, msg_obj: dict[str, JsonValue]) -> bool: ...

    def is_content_message(self, msg_obj: dict[str, JsonValue]) -> bool: ...

    def requires_session_configuration(self) -> bool: ...


class ScriptedProvider(ScriptedProviderView):
    def __init__(self, document: ProviderDocument, timeline: list[JsonValue]) -> None:
        self._document = document
        self._transforms = list(document["transforms"])
        self._timeline = timeline

    def transform_realtime_request(
        self, message: str, model: str, session_request: str | None = None
    ) -> list[str | bytes]:
        transform: dict[str, JsonValue] = {"message": decoded(message), "model": model}
        transform["session_configuration_request"] = session_request
        self._timeline.append({"transform": transform})
        if self._document["fail_transform"]:
            raise SendFailureError("transform")
        frames = self._transforms.pop(0) if self._transforms else []
        return [
            frame["text"] if frame["kind"] == "text" else bytes(frame["bytes"])
            for frame in frames
        ]

    async def pace_backend_send(self, _message: bytes) -> None:
        self._timeline.append("pace")

    @override
    def is_setup_message(self, msg_obj: dict[str, JsonValue]) -> bool:
        return msg_obj in self._document["setup"]

    @override
    def is_content_message(self, msg_obj: dict[str, JsonValue]) -> bool:
        return msg_obj in self._document["content"]

    @override
    def requires_session_configuration(self) -> bool:
        return self._document["requires_session_configuration"]


class WireBackend:
    def __init__(self, socket: ClientConnection, log: WireLog, timeline: list[JsonValue]) -> None:
        self._socket = socket
        self.log = log
        self._timeline = timeline

    async def send(self, message: str | bytes) -> None:
        self.log.begin("backend")
        await self._socket.send(message)
        self.log.delivered += 1
        self._timeline.append("backend")


class ClientSink:
    def __init__(self, log: WireLog, *, beta: bool) -> None:
        headers = [(b"openai-beta", b"realtime=v1")] if beta else []
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {"headers": headers}
        self.log = log

    async def send_text(self, data: str) -> None:
        self.log.begin("client")
        if len(self.log.sent) >= MAX_FRAMES:
            raise SendFailureError("the scripted client capture is full")
        self.log.sent.append(data)


class Upstream:
    def __init__(self) -> None:
        self.frames: list[JsonValue] = []
        self.arrived = asyncio.Event()

    async def __call__(self, connection: ServerConnection) -> None:
        async for message in connection:
            if len(self.frames) >= MAX_FRAMES:
                raise SendFailureError("the scripted upstream capture is full")
            match message:
                case str():
                    self.frames.append(recorded("text", message.encode("utf-8")))
                case bytes():
                    self.frames.append({"wire": "binary", "payload": {"kind": "bytes", "bytes": list(message)}})
            self.arrived.set()

    async def wait_for(self, delivered: int) -> None:
        while len(self.frames) < delivered:
            self.arrived.clear()
            await self.arrived.wait()


class Probe(realtime_streaming.RealTimeStreaming):
    def __init__(
        self,
        input_value: Input,
        backend: WireBackend,
        client: ClientSink,
        provider: ScriptedProvider | None,
    ) -> None:
        backend_argument = cast(ClientConnection | RealtimeBackend, backend)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L26-L40
        provider_argument = cast(BaseRealtimeConfig | None, provider)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L43-L127
        logging_argument = cast(LiteLLMLogging, None)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127-L144
        normalizer = cast(realtime_streaming.RealtimeEventNormalizer, ScriptedNormalizer())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L111-L114
        request_data: dict[str, JsonValue] = {}
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127-L144
            client,
            backend_argument,
            logging_argument,
            provider_config=provider_argument,
            model=input_value["model"],
            user_api_key_dict=None,
            request_data=request_data,
            backend_uses_beta_protocol=input_value["backend_uses_beta_protocol"],
            force_transcription_model=input_value["force_transcription_model"],
            event_normalizer=normalizer,
        )

    async def send_to_backend(self, message: str) -> bool:
        return await self._send_to_backend(message)

    async def send_event_to_client(self, event: JsonValue, event_str: str) -> bool:
        return await self._send_event_to_client(event, event_str)

    async def disable(self) -> None:
        await self._maybe_send_guardrail_turn_detection_update()

    def cache_setup(self, message: str) -> None:
        self._cache_session_configuration_request(message)

    def facts(self) -> dict[str, JsonValue]:
        request = self.session_configuration_request
        cached: JsonValue = None
        if request is not None:
            cached = recorded("text", request.encode("utf-8"))["payload"]
        return {
            "session_configuration_request": cached,
            "content_sent_after_setup": self._content_sent_after_setup,
            "turn_detection_update_sent": self._guardrail_turn_detection_update_sent,
            "is_transcription_session": self._is_transcription_session,
        }


async def apply(probe: Probe, op: Operation) -> JsonValue:
    try:
        match op["kind"]:
            case "send":
                return {"ok": await probe.send_to_backend(op["text"])}
            case "event":
                event = WIRE.validate_json(op["text"])
                return {"ok": await probe.send_event_to_client(event, op["text"])}
            case "disable":
                await probe.disable()
                return {"ok": True}
            case "cache":
                probe.cache_setup(op["text"])
                return {"ok": True}
            case other:
                raise ValueError(f"unknown operation kind: {other}")
    except SendFailureError as error:
        return {"error": str(error)}
    except AttributeError:
        return {"error": "non-object"}


async def session(input_value: Input, upstream: Upstream, socket: ClientConnection) -> JsonValue:
    callbacks: list[CustomLogger] = []
    for document in input_value["guardrails"]:
        hooks = [GuardrailEventHooks(name) for name in document["runs_for"]]
        custom = CustomGuardrail(guardrail_name=None, event_hook=hooks, default_on=True)
        callbacks.append(custom if document["custom"] else CustomLogger())
    litellm.callbacks = [*callbacks]
    timeline: list[JsonValue] = []
    provider_document = input_value["provider"]
    provider = None if provider_document is None else ScriptedProvider(provider_document, timeline)
    backend = WireBackend(socket, WireLog(input_value["backend_failures"]), timeline)
    client = ClientSink(
        WireLog(input_value["client_failures"]), beta=input_value["client_wants_beta"]
    )
    probe = Probe(input_value, backend, client, provider)
    results: list[JsonValue] = []
    for op in input_value["operations"]:
        results.append(await apply(probe, op))
        await asyncio.wait_for(upstream.wait_for(backend.log.delivered), SETTLE_SECONDS)
    client_frames: list[JsonValue] = [
        recorded("text", text.encode("utf-8")) for text in client.log.sent
    ]
    payload: dict[str, JsonValue] = {
        "results": results,
        "timeline": timeline,
        "upstream": upstream.frames,
        "client": client_frames,
        "state": probe.facts(),
    }
    return WIRE.validate_python(payload)


async def exchange(
    input_value: Input, upstream: Upstream, server: socketserver.TCPServer
) -> JsonValue:
    url = f"ws://127.0.0.1:{server.server_address[1]}"
    async with serve(upstream, sock=server.socket), connect(url) as socket:
        return await session(input_value, upstream, socket)


async def bounded(input_value: Input, upstream: Upstream, server: socketserver.TCPServer) -> JsonValue:
    call = exchange(input_value, upstream, server)
    return await asyncio.wait_for(call, DEADLINE_SECONDS)


def run(input_value: Input) -> JsonValue:
    checked(input_value)
    upstream = Upstream()
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as server:
        return asyncio.run(bounded(input_value, upstream, server))
