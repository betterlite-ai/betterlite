import asyncio
import socketserver
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import realtime_streaming
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig, RealtimeBackend
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue, TypeAdapter, ValidationError
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import ServerConnection, serve

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EVENT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
SETTLE_SECONDS = 10.0
DROP_TYPE = "conversation.item.added"
OLD_TYPE = "response.output_text.done"
NEW_TYPE = "response.text.done"
Recorded: TypeAlias = dict[str, JsonValue]


class Frame(TypedDict):
    kind: Literal["text", "binary"]
    text: str


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
    text: str


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


class SendFailureError(Exception):
    pass


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


def recorded(wire: str, data: bytes) -> Recorded:
    try:
        payload: JsonValue = {"kind": "json", "value": WIRE.validate_json(data)}
    except ValidationError:
        payload = {"kind": "text", "text": data.decode("utf-8", errors="replace")}
    return {"wire": wire, "payload": payload}


class ScriptedNormalizer:
    def should_drop(self, event: JsonValue) -> bool:
        try:
            kind: JsonValue = EVENT.validate_python(event).get("type")
        except ValidationError:
            return False
        return kind == DROP_TYPE

    def normalize(self, event: dict[str, JsonValue]) -> dict[str, JsonValue]:
        if event.get("type") == OLD_TYPE:
            event["type"] = NEW_TYPE
        return event

    def patch_outgoing_session(self, session: dict[str, JsonValue]) -> dict[str, JsonValue]:
        return session


class ScriptedProvider:
    def __init__(self, document: ProviderDocument, timeline: list[str]) -> None:
        self._document = document
        self._transforms = list(document["transforms"])
        self._timeline = timeline

    def transform_realtime_request(
        self, _message: str, _model: str, _session_request: str | None = None
    ) -> list[str | bytes]:
        if self._document["fail_transform"]:
            raise SendFailureError("transform")
        frames = self._transforms.pop(0) if self._transforms else []
        return [
            frame["text"] if frame["kind"] == "text" else frame["text"].encode()
            for frame in frames
        ]

    async def pace_backend_send(self, _message: bytes) -> None:
        self._timeline.append("pace")

    def is_setup_message(self, msg_obj: dict[str, JsonValue]) -> bool:
        return msg_obj in self._document["setup"]

    def is_content_message(self, msg_obj: dict[str, JsonValue]) -> bool:
        return msg_obj in self._document["content"]

    def requires_session_configuration(self) -> bool:
        return self._document["requires_session_configuration"]


class WireBackend:
    def __init__(self, socket: ClientConnection, log: WireLog, timeline: list[str]) -> None:
        self._socket = socket
        self.log = log
        self._timeline = timeline

    async def send(self, message: str | bytes) -> None:
        self.log.begin("backend")
        await self._socket.send(message)
        self.log.delivered += 1
        self._timeline.append("backend")


class ClientSink:
    def __init__(self, beta: bool, log: WireLog) -> None:
        headers = [(b"openai-beta", b"realtime=v1")] if beta else []
        self.scope: dict[str, list[tuple[bytes, bytes]]] = {"headers": headers}
        self.log = log

    async def send_text(self, data: str) -> None:
        self.log.begin("client")
        self.log.sent.append(data)


class Upstream:
    def __init__(self) -> None:
        self.frames: list[Recorded] = []
        self.arrived = asyncio.Event()

    async def __call__(self, connection: ServerConnection) -> None:
        async for message in connection:
            match message:
                case str():
                    wire, data = "text", message.encode("utf-8")
                case bytes():
                    wire, data = "binary", message
            self.frames.append(recorded(wire, data))
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
        super().__init__(
            client,
            backend_argument,
            logging_argument,
            provider_config=provider_argument,
            model=input_value["model"],
            user_api_key_dict=None,
            request_data={},
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
    litellm.callbacks = callbacks
    timeline: list[str] = []
    provider_document = input_value["provider"]
    provider = None if provider_document is None else ScriptedProvider(provider_document, timeline)
    backend = WireBackend(socket, WireLog(input_value["backend_failures"]), timeline)
    client = ClientSink(input_value["client_wants_beta"], WireLog(input_value["client_failures"]))
    probe = Probe(input_value, backend, client, provider)
    results: list[JsonValue] = []
    for op in input_value["operations"]:
        results.append(await apply(probe, op))
        await asyncio.wait_for(upstream.wait_for(backend.log.delivered), SETTLE_SECONDS)
    return WIRE.validate_python(
        {
            "results": results,
            "timeline": timeline,
            "upstream": upstream.frames,
            "client": [recorded("text", text.encode("utf-8")) for text in client.log.sent],
            "state": probe.facts(),
        }
    )


async def exchange(input_value: Input) -> JsonValue:
    upstream = Upstream()
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as listener:
        url = f"ws://127.0.0.1:{listener.server_address[1]}"
        async with serve(upstream, sock=listener.socket), connect(url) as socket:
            return await session(input_value, upstream, socket)


def run(input_value: Input) -> JsonValue:
    return asyncio.run(exchange(input_value))
