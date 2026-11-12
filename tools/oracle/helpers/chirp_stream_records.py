import asyncio
import json
import sys
from collections.abc import AsyncIterable, AsyncIterator
from dataclasses import dataclass
from datetime import timedelta
from enum import Enum
from types import ModuleType
from typing import TypedDict, cast

from litellm.llms.vertex_ai.audio_transcription import realtime_backend
from litellm.types.llms.vertex_ai_speech_to_text import VertexSpeechStreamingConfigure


class Encoding(Enum):
    LINEAR16 = 1


@dataclass(frozen=True)
class ExplicitDecodingConfig:
    AudioEncoding = Encoding
    encoding: Encoding
    sample_rate_hertz: int
    audio_channel_count: int


@dataclass(frozen=True)
class RecognitionConfig:
    explicit_decoding_config: ExplicitDecodingConfig
    model: str
    language_codes: tuple[str, ...]


@dataclass(frozen=True)
class StreamingRecognitionFeatures:
    interim_results: bool
    enable_voice_activity_events: bool


@dataclass(frozen=True)
class StreamingRecognitionConfig:
    config: RecognitionConfig
    streaming_features: StreamingRecognitionFeatures


@dataclass(frozen=True)
class StreamingRecognizeRequest:
    recognizer: str
    streaming_config: StreamingRecognitionConfig | None = None
    audio: bytes | None = None


class SpeechTypesModule(ModuleType):
    ExplicitDecodingConfig: type[ExplicitDecodingConfig]
    RecognitionConfig: type[RecognitionConfig]
    StreamingRecognitionConfig: type[StreamingRecognitionConfig]
    StreamingRecognitionFeatures: type[StreamingRecognitionFeatures]
    StreamingRecognizeRequest: type[StreamingRecognizeRequest]


class SpeechPackageModule(ModuleType):
    types: SpeechTypesModule


def install_speech_stub() -> None:
    # google-cloud-speech is absent, so the SDK types LiteLLM imports are stubbed
    types_module = SpeechTypesModule("google.cloud.speech_v2.types")
    types_module.ExplicitDecodingConfig = ExplicitDecodingConfig
    types_module.RecognitionConfig = RecognitionConfig
    types_module.StreamingRecognitionConfig = StreamingRecognitionConfig
    types_module.StreamingRecognitionFeatures = StreamingRecognitionFeatures
    types_module.StreamingRecognizeRequest = StreamingRecognizeRequest
    package = SpeechPackageModule("google.cloud.speech_v2")
    package.types = types_module
    sys.modules["google.cloud.speech_v2"] = package
    sys.modules["google.cloud.speech_v2.types"] = types_module


install_speech_stub()


class SpeechEventType(Enum):
    SPEECH_EVENT_TYPE_UNSPECIFIED = 0
    END_OF_SINGLE_UTTERANCE = 1
    SPEECH_ACTIVITY_BEGIN = 2
    SPEECH_ACTIVITY_END = 3


@dataclass(frozen=True)
class Alternative:
    transcript: str


@dataclass(frozen=True)
class RecognitionResult:
    alternatives: tuple[Alternative, ...]
    is_final: bool


@dataclass(frozen=True)
class RecognitionMetadata:
    total_billed_duration: timedelta


@dataclass(frozen=True)
class RecognizeResponse:
    speech_event_type: SpeechEventType
    results: tuple[RecognitionResult, ...]
    metadata: RecognitionMetadata


class IdleTransport:
    def __init__(self) -> None:
        self.closed = False

    async def close(self) -> None:
        self.closed = True
        await asyncio.sleep(0)


class IdleClient:
    def __init__(self) -> None:
        self._transport = IdleTransport()

    @property
    def transport(self) -> IdleTransport:
        return self._transport

    async def streaming_recognize(
        self, requests: AsyncIterator[StreamingRecognizeRequest] | None = None
    ) -> AsyncIterable[RecognizeResponse]:
        del requests
        raise NotImplementedError("the chirp_stream_records helper never starts a stream")


class ConfigureInput(TypedDict):
    model: str
    language_codes: list[str]
    sample_rate_hertz: int


class AlternativeInput(TypedDict):
    transcript: str


class ResultInput(TypedDict):
    alternatives: list[AlternativeInput]
    is_final: bool


class BilledInput(TypedDict):
    seconds: int
    nanos: int


class ResponseInput(TypedDict):
    speech_event_type: str
    results: list[ResultInput]
    total_billed_duration: BilledInput


class Input(TypedDict):
    configure: ConfigureInput
    recognizer: str
    billed_before: float
    responses: list[ResponseInput]


class ConfigOutput(TypedDict):
    model: str
    language_codes: list[str]
    encoding: str
    sample_rate_hertz: int
    audio_channel_count: int
    interim_results: bool
    enable_voice_activity_events: bool


class FirstRequestOutput(TypedDict):
    recognizer: str
    has_streaming_config: bool
    has_audio: bool
    config: ConfigOutput


class ResultOutput(TypedDict):
    transcript: str
    is_final: bool


class EventOutput(TypedDict):
    kind: str
    speech_event: str
    results: list[ResultOutput]
    billed_seconds: float


class Output(TypedDict):
    first_request: FirstRequestOutput
    events: list[EventOutput]
    speech_active: bool
    billed_seconds: float


def recognize_response(fields: ResponseInput) -> RecognizeResponse:
    results: list[RecognitionResult] = []
    for result_field in fields["results"]:
        alternatives = tuple(
            Alternative(transcript=alternative["transcript"])
            for alternative in result_field["alternatives"]
        )
        results.append(
            RecognitionResult(alternatives=alternatives, is_final=result_field["is_final"])
        )
    billed = fields["total_billed_duration"]
    return RecognizeResponse(
        speech_event_type=SpeechEventType[fields["speech_event_type"]],
        results=tuple(results),
        metadata=RecognitionMetadata(
            total_billed_duration=timedelta(
                seconds=billed["seconds"], microseconds=billed["nanos"] // 1000
            )
        ),
    )


def first_request_output(
    request: StreamingRecognizeRequest, config: StreamingRecognitionConfig
) -> FirstRequestOutput:
    decoding = config.config.explicit_decoding_config
    return {
        "recognizer": request.recognizer,
        "has_streaming_config": request.streaming_config is not None,
        "has_audio": request.audio is not None,
        "config": {
            "model": config.config.model,
            "language_codes": list(config.config.language_codes),
            "encoding": decoding.encoding.name,
            "sample_rate_hertz": decoding.sample_rate_hertz,
            "audio_channel_count": decoding.audio_channel_count,
            "interim_results": config.streaming_features.interim_results,
            "enable_voice_activity_events": config.streaming_features.enable_voice_activity_events,
        },
    }


def run(input_value: Input) -> Output:
    command = VertexSpeechStreamingConfigure(
        model=input_value["configure"]["model"],
        language_codes=tuple(input_value["configure"]["language_codes"]),
        sample_rate_hertz=input_value["configure"]["sample_rate_hertz"],
    )
    config = cast(StreamingRecognitionConfig, realtime_backend._streaming_config(command))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/audio_transcription/realtime_backend.py#L116-L137
    request = StreamingRecognizeRequest(
        recognizer=input_value["recognizer"], streaming_config=config
    )
    stream = realtime_backend._RecognizeStream(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/audio_transcription/realtime_backend.py#L160-L182
        client=IdleClient(),
        request_type=StreamingRecognizeRequest,
        first_request=request,
        opened_at=0.0,
        turn=0,
    )
    events: list[EventOutput] = []
    for response_input in input_value["responses"]:
        response = recognize_response(response_input)
        stream._note(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/audio_transcription/realtime_backend.py#L229-L233
        billed = input_value["billed_before"] + stream.billed_seconds
        events.append(
            cast(EventOutput, json.loads(realtime_backend._response_event(response, billed)))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/audio_transcription/realtime_backend.py#L138-L150
        )
    return {
        "first_request": first_request_output(request, config),
        "events": events,
        "speech_active": stream.speech_active,
        "billed_seconds": input_value["billed_before"] + stream.billed_seconds,
    }
