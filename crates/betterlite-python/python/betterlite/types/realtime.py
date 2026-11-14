from typing import ClassVar, Literal, TypeAlias

from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.llms.openai.realtime import (
    OpenAIRealtimeEvents,
    OpenAIRealtimeOutputItemDone,
    OpenAIRealtimeResponseDelta,
)
from betterlite.types.utils.base import ResponseObject

ALL_DELTA_TYPES: TypeAlias = Literal["text", "audio"]


class RealtimeResponseTransformInput(TypedDict):
    session_configuration_request: str | None
    current_output_item_id: str | None
    current_response_id: str | None
    current_delta_chunks: list[OpenAIRealtimeResponseDelta] | None
    current_item_chunks: list[OpenAIRealtimeOutputItemDone] | None
    current_conversation_id: str | None
    current_delta_type: ALL_DELTA_TYPES | None


class RealtimeResponseTypedDict(TypedDict):
    response: OpenAIRealtimeEvents | list[OpenAIRealtimeEvents]
    current_output_item_id: str | None
    current_response_id: str | None
    current_delta_chunks: list[OpenAIRealtimeResponseDelta] | None
    current_conversation_id: str | None
    current_item_chunks: list[OpenAIRealtimeOutputItemDone] | None
    current_delta_type: ALL_DELTA_TYPES | None
    session_configuration_request: str | None


# ast-grep-ignore: no-long-names
class RealtimeModalityResponseTransformOutput(TypedDict):
    returned_message: list[OpenAIRealtimeEvents]
    current_output_item_id: str | None
    current_response_id: str | None
    current_conversation_id: str | None
    current_delta_chunks: list[OpenAIRealtimeResponseDelta] | None
    current_delta_type: ALL_DELTA_TYPES | None


class RealtimeQueryParams(TypedDict, total=False):
    model: str
    intent: str | None


class RealtimeExpiresAfter(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    anchor: str | None = "created_at"
    seconds: int | None = None


class RealtimeSessionConfig(ResponseObject):
    type: str | None = None
    model: str | None = None
    instructions: str | None = None
    audio: dict[str, JsonValue] | None = None
    include: list[str] | None = None
    max_output_tokens: int | str | None = None
    output_modalities: list[str] | None = None
    tool_choice: JsonValue = None
    tools: list[dict[str, JsonValue]] | None = None
    tracing: JsonValue = None
    truncation: JsonValue = None
    prompt: dict[str, JsonValue] | None = None


class RealtimeClientSecretRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    expires_after: RealtimeExpiresAfter | None = None
    session: RealtimeSessionConfig | None = None
    model: str | None = None


class RealtimeClientSecretResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    expires_at: int | None = None
    value: str
    session: dict[str, JsonValue] | None = None


# ast-grep-ignore: no-long-names
class RealtimeTranscriptionSessionRequest(ResponseObject):
    model: str | None = None
    input_audio_transcription: dict[str, JsonValue] | None = None

    def resolved_model(self) -> str | None:
        if self.model is not None:
            return self.model
        if self.input_audio_transcription is None:
            return None
        found: JsonValue = self.input_audio_transcription.get("model")
        return TypeAdapter(str | None).validate_python(found)


# ast-grep-ignore: no-long-names
class RealtimeTranscriptionSessionResponse(ResponseObject):
    client_secret: dict[str, JsonValue] | None = None


class RealtimeErrorDetail(TypedDict):
    type: ReadOnly[str]
    message: ReadOnly[str]


class RealtimeErrorEvent(TypedDict):
    type: ReadOnly[Literal["error"]]
    error: ReadOnly[RealtimeErrorDetail]


# ast-grep-ignore: no-long-names
class RealtimeInputAudioTranscriptionUsageInputTokenDetails(TypedDict):
    text_tokens: ReadOnly[int]
    audio_tokens: ReadOnly[int]


# ast-grep-ignore: no-long-names
class RealtimeInputAudioTranscriptionTokenUsage(TypedDict):
    type: ReadOnly[Literal["tokens"]]
    input_tokens: ReadOnly[int]
    output_tokens: ReadOnly[int]
    total_tokens: ReadOnly[int]
    # ast-grep-ignore: no-long-names
    input_token_details: ReadOnly[RealtimeInputAudioTranscriptionUsageInputTokenDetails]


# ast-grep-ignore: no-long-names
class RealtimeInputAudioTranscriptionDurationUsage(TypedDict):
    type: ReadOnly[Literal["duration"]]
    seconds: ReadOnly[float]


# ast-grep-ignore: no-long-names
RealtimeInputAudioTranscriptionUsage: TypeAlias = (
    # ast-grep-ignore: no-long-names
    RealtimeInputAudioTranscriptionTokenUsage | RealtimeInputAudioTranscriptionDurationUsage
)
