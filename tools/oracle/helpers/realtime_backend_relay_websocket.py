import asyncio
from collections.abc import Sequence
from datetime import UTC, datetime
from typing import Final, Protocol, TypedDict, cast

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.realtime_streaming import BackendClose, RealTimeStreaming
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig, RealtimeBackend
from litellm.types.llms.openai import OpenAIRealtimeOutputItemDone, OpenAIRealtimeResponseDelta
from litellm.types.realtime import RealtimeInputAudioTranscriptionUsage as InputUsage
from litellm.types.realtime import RealtimeResponseTransformInput as TransformInput
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override
from websockets.exceptions import ConnectionClosed
from websockets.frames import Close

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
MAX_FRAMES: Final = 32
MAX_TEXT_BYTES: Final = 16 * 1024
MAX_TOTAL_BYTES: Final = 256 * 1024


class CloseDocument(TypedDict):
    code: int
    reason: str


class ProviderDocument(TypedDict):
    requires_session_configuration: bool


class Input(TypedDict):
    provider: ProviderDocument | None
    frames: list[str]
    close: CloseDocument | None
    pending: list[str]
    force_transcription_model: str | None


class ResponseDocument(TypedDict):
    response: JsonValue
    current_output_item_id: str | None
    current_response_id: str | None
    current_delta_chunks: list[OpenAIRealtimeResponseDelta] | None
    current_conversation_id: str | None
    current_item_chunks: list[OpenAIRealtimeOutputItemDone] | None
    current_delta_type: str | None
    session_configuration_request: str | None


class CallFlags(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    success: JsonValue = Field(default=None, alias="realtime_session_success_logged")
    failure: JsonValue = Field(default=None, alias="realtime_session_failure_logged")


class LoggedDetails(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: CallFlags


class ProviderMethods(Protocol):
    def transform_realtime_request(
        self, message: str, _model: str, _session_configuration_request: str | None = None
    ) -> Sequence[str | bytes]: ...

    def transform_realtime_response(
        self,
        message: str | bytes,
        _model: str,
        _logging_obj: Logging,
        realtime_response_transform_input: TransformInput,  # ast-grep-ignore: no-long-names
    ) -> ResponseDocument: ...

    def requires_session_configuration(self) -> bool: ...

    def unbilled_usage_on_session_close(  # ast-grep-ignore: no-long-names
        self, _model: str
    ) -> InputUsage | None: ...


class ClientMethods(Protocol):
    async def send_text(self, data: str) -> None: ...


class BackendMethods(Protocol):
    async def send(self, message: str) -> None: ...

    async def recv(self, *, decode: bool | None = None) -> str: ...

    async def close(self) -> None: ...


class ScriptedProvider(ProviderMethods):
    def __init__(self, document: ProviderDocument) -> None:
        self._document = document

    @override
    def transform_realtime_request(
        self, message: str, _model: str, _session_configuration_request: str | None = None
    ) -> Sequence[str | bytes]:
        return [message]

    @override
    def transform_realtime_response(
        self,
        message: str | bytes,
        _model: str,
        _logging_obj: Logging,
        realtime_response_transform_input: TransformInput,  # ast-grep-ignore: no-long-names
    ) -> ResponseDocument:
        # LiteLLM replaces all seven assembly fields with the input it passed in, and relays the response it gets back
        return {
            **realtime_response_transform_input,  # ast-grep-ignore: no-long-names
            "response": JSON.validate_json(message),
        }

    @override
    def requires_session_configuration(self) -> bool:
        return self._document["requires_session_configuration"]

    @override
    def unbilled_usage_on_session_close(  # ast-grep-ignore: no-long-names
        self, _model: str
    ) -> InputUsage | None:
        return None


class ProbeClient(ClientMethods):
    def __init__(self) -> None:
        self.scope: dict[str, Sequence[tuple[bytes, bytes]]] = {"headers": []}
        self.sent: list[str] = []

    @override
    async def send_text(self, data: str) -> None:
        self.sent.append(data)


class ScriptedBackend(BackendMethods):
    def __init__(self, frames: list[str], close: CloseDocument | None) -> None:
        self._frames = frames
        self._close = close
        self.sent: list[str] = []
        self.decode_calls: list[bool | None] = []

    @override
    async def send(self, message: str) -> None:
        self.sent.append(message)

    @override
    async def recv(self, *, decode: bool | None = None) -> str:
        self.decode_calls.append(decode)
        if not self._frames:
            document = self._close
            if document is None:
                raise ConnectionClosed(None, None)
            raise ConnectionClosed(Close(document["code"], document["reason"]), None)
        return self._frames.pop(0)

    @override
    async def close(self) -> None:
        pass


class Probe(RealTimeStreaming):
    def __init__(
        self,
        client: ProbeClient,
        backend: ScriptedBackend,
        logging_obj: Logging,
        provider: ScriptedProvider | None,
        force_transcription_model: str | None,
    ) -> None:
        backend_argument = cast(RealtimeBackend, backend)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L26-L40
        provider_argument = cast(BaseRealtimeConfig | None, provider)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L43-L127
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127-L144
            client,
            backend_argument,
            logging_obj,
            provider_config=provider_argument,
            model="scripted",
            force_transcription_model=force_transcription_model,
        )

    def pending(self, messages: list[str]) -> None:
        self._pending_messages_until_setup = messages
        self._pending_messages_byte_total = sum(len(message.encode()) for message in messages)


def frame(text: str) -> JsonValue:
    try:
        value = JSON.validate_json(text)
    except ValidationError:
        return {"kind": "text", "text": text}
    return {"kind": "json", "value": value}


def snapshot(
    streaming: Probe,
    close: BackendClose,
    client: ProbeClient,
    backend: ScriptedBackend,
    logging_obj: Logging,
) -> JsonValue:
    flags = LoggedDetails.model_validate(logging_obj).model_call_details
    close_frame: dict[str, JsonValue] = {"code": close.code, "reason": close.reason}
    client_frames: list[JsonValue] = [frame(text) for text in client.sent]
    backend_frames: list[JsonValue] = [frame(text) for text in backend.sent]
    payload: dict[str, JsonValue] = {
        "close": JSON.validate_python(close_frame),
        "client": JSON.validate_python(client_frames),
        "backend": JSON.validate_python(backend_frames),
        "decode_calls": JSON.validate_python(backend.decode_calls),
        "messages": JSON.validate_python(streaming.messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L145
        "input_messages": JSON.validate_python(streaming.input_messages),
        "logged": {"success": flags.success, "failure": flags.failure},
    }
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    texts = [*input_value["frames"], *input_value["pending"]]
    if (
        len(input_value["frames"]) > MAX_FRAMES
        or len(input_value["pending"]) > MAX_FRAMES
        or any(len(text.encode()) > MAX_TEXT_BYTES for text in texts)
        or sum(len(text.encode()) for text in texts) > MAX_TOTAL_BYTES
    ):
        raise ValueError("the scripted relay input is past its limits")
    litellm.logged_real_time_event_types = "*"
    logging_obj = Logging(
        model="scripted",
        messages=None,
        stream=False,
        call_type="realtime",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )
    client = ProbeClient()
    backend = ScriptedBackend(input_value["frames"], input_value["close"])
    document = input_value["provider"]
    streaming = Probe(
        client,
        backend,
        logging_obj,
        None if document is None else ScriptedProvider(document),
        input_value["force_transcription_model"],
    )
    streaming.pending(input_value["pending"])
    close = asyncio.run(streaming.backend_to_client_send_messages())
    return snapshot(streaming, close, client, backend, logging_obj)
