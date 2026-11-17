from collections.abc import Sequence
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.litellm_core_utils.realtime_streaming import RealTimeStreaming
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig
from litellm.types import realtime as realtime_types
from litellm.types.realtime import RealtimeInputAudioTranscriptionUsage as LiteLLMUsage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
UsageObject: TypeAlias = dict[str, JsonValue]


class ProviderDocument(TypedDict):
    requires_session_configuration: bool
    unbilled_usage: NotRequired[LiteLLMUsage | None]


class TextFrame(TypedDict):
    kind: Literal["text"]
    text: str


class BytesFrame(TypedDict):
    kind: Literal["bytes"]
    text: str


class RecordFrame(TypedDict):
    kind: Literal["record"]
    values: UsageObject


Frame: TypeAlias = TextFrame | BytesFrame | RecordFrame


class TextMessage(TypedDict):
    kind: Literal["text"]
    text: str


class RecordMessage(TypedDict):
    kind: Literal["record"]
    values: UsageObject


ClientMessage: TypeAlias = TextMessage | RecordMessage


class Input(TypedDict):
    model: str
    logged_event_types: list[str] | Literal["*"] | None
    client_wants_beta: bool
    backend_uses_beta_protocol: bool | None
    force_transcription_model: str | None
    request_data: UsageObject | None
    user_api_key_dict: JsonValue
    provider: ProviderDocument | None
    store: list[Frame]
    inputs: list[ClientMessage]
    backend_events: list[UsageObject]
    usage_events: list[UsageObject]
    flush: bool


class Socket:
    scope: dict[str, Sequence[tuple[bytes, bytes]]]

    def __init__(self, headers: Sequence[tuple[bytes, bytes]]) -> None:
        self.scope = {"headers": headers}


class ProviderConfig(BaseRealtimeConfig):
    def __init__(self, *, requires: bool, usage: LiteLLMUsage | None) -> None:
        self._requires = requires
        self._usage = usage

    @override
    def validate_environment(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L43-L51
        self, headers: dict[str, JsonValue], _model: str, _api_key: str | None = None
    ) -> dict[str, JsonValue]:
        return dict(headers)

    @override
    def get_complete_url(
        self, api_base: str | None, model: str, api_key: str | None = None
    ) -> str:
        return super().get_complete_url(api_base, model, api_key)

    @override
    def transform_realtime_request(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L71-L78
        self, _message: str, _model: str, _session_request: str | None = None
    ) -> list[str]:
        return []

    @override
    def transform_realtime_response(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L117-L127
        self,
        _message: str | bytes,
        _model: str,
        _logging_obj: LiteLLMLogging,
        _input: realtime_types.RealtimeResponseTransformInput,
    ) -> realtime_types.RealtimeResponseTypedDict:
        return cast(realtime_types.RealtimeResponseTypedDict, {})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L117-L127

    @override
    def requires_session_configuration(self) -> bool:
        return self._requires

    @override
    def unbilled_usage_on_session_close(  # ast-grep-ignore: no-long-names
        self, model: str  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L97-L98
    ) -> LiteLLMUsage | None:
        return self._usage


def socket_for(*, client_wants_beta: bool) -> Socket:
    if client_wants_beta:
        return Socket([(b"openai-beta", b"realtime=v1")])
    return Socket([])


def provider_for(document: ProviderDocument | None) -> ProviderConfig | None:
    if document is None:
        return None
    return ProviderConfig(
        requires=document["requires_session_configuration"],
        usage=document.get("unbilled_usage"),
    )


def frame_argument(frame: Frame) -> str | bytes | UsageObject:
    value: str | bytes | UsageObject
    match frame["kind"]:
        case "text":
            value = frame["text"]
        case "bytes":
            value = frame["text"].encode("utf-8")
        case "record":
            value = frame["values"]
    return value


def client_argument(message: ClientMessage) -> str | UsageObject:
    value: str | UsageObject
    match message["kind"]:
        case "text":
            value = message["text"]
        case "record":
            value = message["values"]
    return value


def snapshot(streaming: RealTimeStreaming) -> dict[str, JsonValue]:
    return {
        "logged_event_types": JSON.validate_python(streaming.logged_real_time_event_types),
        "model": streaming.model,
        "user_api_key_dict": JSON.validate_python(streaming.user_api_key_dict),
        "request_data": cast(JsonValue, streaming.request_data),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L173
        "client_wants_beta": streaming._client_wants_beta,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L153
        "backend_uses_beta_protocol": streaming._backend_uses_beta_protocol,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L154-L156
        "backend_sent_frames": streaming._backend_sent_frames,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L146
        "has_provider_config": streaming.provider_config is not None,
        "force_transcription_model": streaming._force_transcription_model,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L197
        "is_transcription_session": streaming._is_transcription_session,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L198
        "messages": cast(JsonValue, streaming.messages),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L145
        "input_message": cast(JsonValue, streaming.input_message),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L147
        "input_messages": JSON.validate_python(streaming.input_messages),
        "session_tools": cast(JsonValue, streaming.session_tools),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L149
        "tool_calls": cast(JsonValue, streaming.tool_calls),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L150
        "response": {
            "current_output_item_id": streaming.current_output_item_id,
            "current_response_id": streaming.current_response_id,
            "current_conversation_id": streaming.current_conversation_id,
            "current_delta_chunks": cast(JsonValue, streaming.current_delta_chunks),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L165
            "current_item_chunks": cast(JsonValue, streaming.current_item_chunks),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L169
            "current_delta_type": streaming.current_delta_type,
            "session_configuration_request": streaming.session_configuration_request,
        },
        "setup": {
            "violation_count": streaming._violation_count,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L175
            "pending_guardrail_message": streaming._pending_guardrail_message,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L178
            "session_created_sent_to_client": streaming._session_created_sent_to_client,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L181
            "turn_detection_update_sent": streaming._guardrail_turn_detection_update_sent,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L184
            "backend_setup_complete": streaming._backend_setup_complete,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L187
            "flushing_pending_messages": streaming._flushing_pending_messages_until_setup,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L188
            "pending_messages_until_setup": JSON.validate_python(
                streaming._pending_messages_until_setup  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L189
            ),
            "pending_messages_byte_total": streaming._pending_messages_byte_total,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L190
            "content_sent_after_setup": streaming._content_sent_after_setup,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L193
        },
    }


def run(input_value: Input) -> JsonValue:
    litellm.logged_real_time_event_types = input_value["logged_event_types"]
    streaming = RealTimeStreaming(socket_for(client_wants_beta=input_value["client_wants_beta"]), None, None, provider_config=provider_for(input_value["provider"]), model=input_value["model"], user_api_key_dict=input_value["user_api_key_dict"], request_data=input_value["request_data"], backend_uses_beta_protocol=input_value["backend_uses_beta_protocol"], force_transcription_model=input_value["force_transcription_model"])  # type: ignore[arg-type]  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L127-L144
    for frame in input_value["store"]:
        streaming.store_message(frame_argument(frame))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L247-L255
    for message in input_value["inputs"]:
        streaming.store_input(client_argument(message))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L404-L409
    for event in input_value["backend_events"]:
        streaming._collect_user_input_from_backend_event(event)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L308-L317
        streaming._detect_transcription_session_from_backend(event)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L319-L333
    for event in input_value["usage_events"]:
        streaming._capture_transcription_usage(event)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L335-L361
    if input_value["flush"]:
        streaming._flush_unbilled_transcription_usage()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L364-L380
    return JSON.validate_python(snapshot(streaming))
