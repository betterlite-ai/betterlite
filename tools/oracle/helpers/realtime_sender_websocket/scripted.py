import asyncio
from typing import Protocol, cast

from litellm.litellm_core_utils import realtime_streaming
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig, RealtimeBackend
from pydantic import JsonValue, ValidationError
from typing_extensions import override
from websockets.asyncio.client import ClientConnection
from websockets.asyncio.server import ServerConnection

from .records import (
    EVENT,
    MAX_FRAMES,
    Capture,
    Input,
    ProviderDocument,
    SendFailureError,
    decoded,
    recorded,
)


class WireLog:
    def __init__(self, failures: list[int], capture: Capture) -> None:
        self.capture = capture
        self.attempts = 0
        self.delivered = 0
        self.sent: list[str] = []
        self.failures = failures

    def begin(self, label: str) -> None:
        attempt = self.attempts
        self.attempts += 1
        if attempt in self.failures:
            raise SendFailureError(label)


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
    def __init__(
        self, document: ProviderDocument, timeline: list[JsonValue], capture: Capture
    ) -> None:
        self._capture = capture
        self._document = document
        self._transforms = list(document["transforms"])
        self._timeline = timeline

    def transform_realtime_request(
        self, message: str, model: str, session_request: str | None = None
    ) -> list[str | bytes]:
        texts = [message, model] if session_request is None else [message, model, session_request]
        self._capture.reserve([len(text.encode("utf-8")) for text in texts], "transform")
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
        match message:
            case str():
                self.log.capture.reserve([len(message.encode("utf-8"))], "backend")
            case bytes():
                self.log.capture.reserve([len(message)], "backend")
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
        self.log.capture.reserve([len(data.encode("utf-8"))], "client")
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
                    self.frames.append(
                        {"wire": "binary", "payload": {"kind": "bytes", "bytes": list(message)}}
                    )
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
