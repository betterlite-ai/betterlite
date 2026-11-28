import json
import sys
from collections.abc import Iterable, Iterator
from io import BytesIO
from types import ModuleType
from typing import Final, Literal, Protocol, Self, TypedDict, Unpack, runtime_checkable

import litellm
import numpy as np
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.nvidia_riva.audio_transcription.handler import NvidiaRivaAudioTranscription
from litellm.llms.nvidia_riva.common_utils import NvidiaRivaException
from litellm.types.utils import FileTypes, TranscriptionResponse
from numpy.typing import NDArray
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
LIST: Final = TypeAdapter(list[JsonValue])
MANAGED: Final = ("riva", "riva.client", "soundfile")
CONFIG: Final = ConfigDict(arbitrary_types_allowed=True)

ServiceKind = Literal["timeout", "variadic", "plain"]
EndpointingMode = Literal["ok", "fails", "copy_fails", "missing_field"]


class TranscriptionArguments(TypedDict):
    model: str
    audio_file: FileTypes
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    model_response: TranscriptionResponse
    timeout: float
    logging_obj: Logging
    api_key: str | None
    api_base: str | None
    atranscription: bool


@runtime_checkable
class Handler(Protocol):
    def audio_transcriptions(
        self, **kwargs: Unpack[TranscriptionArguments]
    ) -> TranscriptionResponse: ...


HANDLER: Final = TypeAdapter(Handler, config=CONFIG).validate_python(
    NvidiaRivaAudioTranscription()
)


class WordSpec(TypedDict):
    word: str
    start_time: int
    end_time: int


class AlternativeSpec(TypedDict):
    transcript: str
    words: list[WordSpec]


class ResultSpec(TypedDict):
    is_final: bool
    alternatives: list[AlternativeSpec]


class ResponseSpec(TypedDict):
    results: list[ResultSpec]


class Call(TypedDict):
    model: str
    audio_name: str
    file_hex: str
    samples: int
    optional_params: dict[str, JsonValue]
    api_base: str | None
    api_key: str | None
    timeout_seconds: float
    service: ServiceKind
    legacy_auth: bool
    endpointing: EndpointingMode
    stream: list[ResponseSpec]


class Input(TypedDict):
    calls: list[Call]


class Answer(TypedDict):
    status: int | None
    message: str | None
    events: list[str]
    auth: dict[str, JsonValue]
    recognition: JsonValue
    endpointing: dict[str, JsonValue]
    streaming: dict[str, JsonValue]
    chunk_sizes: list[int] | None
    timeout_seconds: float | None
    pre: JsonValue
    post: JsonValue
    response: JsonValue
    hidden: JsonValue


class HiddenRecord(BaseModel):
    hidden_params: dict[str, JsonValue] = Field(alias="_hidden_params")


class ErrorRecord(BaseModel):
    status_code: int
    message: str


class NamedAudio(BytesIO):
    name: str

    def __init__(self, filename: str, content: bytes) -> None:
        super().__init__(content)
        self.name = filename


class Record:
    def __init__(self) -> None:
        self.events: list[str] = []
        self.auth_kwargs: JsonValue = None
        self.auth_args: JsonValue = None
        self.recognition: JsonValue = None
        self.endpointing_kwargs: JsonValue = None
        self.endpointing_copied: bool = False
        self.interim_results: JsonValue = None
        self.config_is_recognition: bool = False
        self.chunk_sizes: list[int] | None = None
        self.timeout_seconds: float | None = None
        self.pre: JsonValue = None
        self.post: JsonValue = None
        self.response: JsonValue = None


class FakeWord:
    def __init__(self, spec: WordSpec) -> None:
        self.word = spec["word"]
        self.start_time = spec["start_time"]
        self.end_time = spec["end_time"]


class FakeAlternative:
    def __init__(self, spec: AlternativeSpec) -> None:
        self.transcript = spec["transcript"]
        self.words = [FakeWord(word) for word in spec["words"]]


class FakeResult:
    def __init__(self, spec: ResultSpec) -> None:
        self.is_final = spec["is_final"]
        self.alternatives = [FakeAlternative(alternative) for alternative in spec["alternatives"]]


class FakeResponse:
    def __init__(self, spec: ResponseSpec) -> None:
        self.results = [FakeResult(result) for result in spec["results"]]


class AuthFactory:
    def __init__(self, record: Record, call: Call) -> None:
        self.record = record
        self.call = call

    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> Self:
        if kwargs:
            self.record.events.append("auth.kwargs")
            self.record.auth_kwargs = OBJECT.validate_json(json.dumps(kwargs))
            if self.call["legacy_auth"]:
                raise TypeError("this riva-client takes positional auth arguments")
            return self
        self.record.events.append("auth.positional")
        self.record.auth_args = LIST.validate_json(json.dumps(list(args)))
        return self


class EndpointingInstance:
    def __init__(self, values: dict[str, JsonValue]) -> None:
        self.values = values


class CopyCall:
    def __init__(self, record: Record, call: Call) -> None:
        self.record = record
        self.call = call

    def __call__(self, source: EndpointingInstance) -> None:
        self.record.events.append("endpointing.copy")
        if not source.values:
            raise ValueError("the endpointing config carries no field")
        if self.call["endpointing"] == "copy_fails":
            raise TypeError("cannot copy the endpointing config")
        self.record.endpointing_copied = True


class EndpointingTarget:
    def __init__(self, record: Record, call: Call) -> None:
        self.CopyFrom = CopyCall(record, call)


class EndpointingFactory:
    def __init__(self, record: Record, call: Call) -> None:
        self.record = record
        self.call = call

    def __call__(self, **kwargs: JsonValue) -> EndpointingInstance:
        self.record.events.append("endpointing")
        self.record.endpointing_kwargs = dict(kwargs)
        if self.call["endpointing"] == "fails":
            raise ValueError("this riva-client knows no such endpointing field")
        return EndpointingInstance(dict(kwargs))


class RecognitionInstance:
    def __init__(self, record: Record, call: Call) -> None:
        if call["endpointing"] != "missing_field":
            self.endpointing_config = EndpointingTarget(record, call)


class RecognitionFactory:
    def __init__(self, record: Record, call: Call) -> None:
        self.record = record
        self.call = call
        self.built: RecognitionInstance | None = None

    def __call__(self, **kwargs: JsonValue) -> RecognitionInstance:
        self.record.events.append("recognition")
        self.record.recognition = dict(kwargs)
        instance = RecognitionInstance(self.record, self.call)
        self.built = instance
        return instance


class StreamingInstance:
    def __init__(self, config: JsonValue) -> None:
        self.config = config


class StreamingFactory:
    def __init__(self, record: Record, call: Call, recognition: RecognitionFactory) -> None:
        self.record = record
        self.call = call
        self.recognition = recognition
        self.built: StreamingInstance | None = None

    def __call__(self, **kwargs: JsonValue) -> StreamingInstance:
        self.record.events.append("streaming")
        self.record.interim_results = kwargs["interim_results"]
        self.record.config_is_recognition = kwargs["config"] is self.recognition.built
        instance = StreamingInstance(kwargs["config"])
        self.built = instance
        return instance


class FakeAudioEncoding:
    LINEAR_PCM: Final = "LINEAR_PCM"
    FLAC: Final = "FLAC"
    MULAW: Final = "MULAW"


class StreamingCall(TypedDict):
    audio_chunks: Iterable[bytes]
    streaming_config: StreamingInstance
    timeout: float


class ScriptedService:
    def __init__(self, record: Record, call: Call, streaming: StreamingFactory) -> None:
        self.record = record
        self.call = call
        self.streaming = streaming

    def emit(
        self,
        audio_chunks: Iterable[bytes],
        streaming_config: StreamingInstance,
        timeout: float | None,
    ) -> Iterator[FakeResponse]:
        if streaming_config is not self.streaming.built:
            raise RuntimeError("the stream runs a streaming config that was never built")
        self.record.events.append("stream.call")
        self.record.chunk_sizes = [len(chunk) for chunk in audio_chunks]
        self.record.timeout_seconds = None if timeout is None else float(timeout)
        for spec in self.call["stream"]:
            yield FakeResponse(spec)


class TimeoutService(ScriptedService):
    def streaming_response_generator(
        self,
        audio_chunks: Iterable[bytes],
        streaming_config: StreamingInstance,
        timeout: float | None = None,
    ) -> Iterator[FakeResponse]:
        return self.emit(audio_chunks, streaming_config, timeout)


class VariadicService(ScriptedService):
    def streaming_response_generator(
        self, **kwargs: Unpack[StreamingCall]
    ) -> Iterator[FakeResponse]:
        return self.emit(kwargs["audio_chunks"], kwargs["streaming_config"], kwargs["timeout"])


class PlainService(ScriptedService):
    def streaming_response_generator(
        self, audio_chunks: Iterable[bytes], streaming_config: StreamingInstance
    ) -> Iterator[FakeResponse]:
        return self.emit(audio_chunks, streaming_config, None)


class ASRServiceFactory:
    def __init__(self, record: Record, call: Call, streaming: StreamingFactory) -> None:
        self.record = record
        self.call = call
        self.streaming = streaming

    def __call__(self, _auth: AuthFactory) -> ScriptedService:
        self.record.events.append("asr")
        if self.call["service"] == "variadic":
            return VariadicService(self.record, self.call, self.streaming)
        if self.call["service"] == "plain":
            return PlainService(self.record, self.call, self.streaming)
        return TimeoutService(self.record, self.call, self.streaming)


class Scripts:
    def __init__(self, record: Record, call: Call) -> None:
        self.record = record
        self.call = call
        self.auth = AuthFactory(record, call)
        self.endpointing = EndpointingFactory(record, call)
        self.recognition = RecognitionFactory(record, call)
        self.streaming = StreamingFactory(record, call, self.recognition)
        self.services = ASRServiceFactory(record, call, self.streaming)


class FakeRivaClient(ModuleType):
    def __init__(self, scripts: Scripts) -> None:
        super().__init__("riva.client")
        self.Auth = scripts.auth
        self.ASRService = scripts.services
        self.RecognitionConfig = scripts.recognition
        self.StreamingRecognitionConfig = scripts.streaming
        self.EndpointingConfig = scripts.endpointing
        self.AudioEncoding = FakeAudioEncoding


class FakeRivaPackage(ModuleType):
    def __init__(self, client: FakeRivaClient) -> None:
        super().__init__("riva")
        self.client = client


class FakeSoundFile(ModuleType):
    def __init__(self, samples: int) -> None:
        super().__init__("soundfile")
        self.samples = samples

    def read(
        self, buffer: BytesIO, *, dtype: str, always_2d: bool
    ) -> tuple[NDArray[np.float32], int]:
        if buffer.closed or dtype != "float32" or always_2d:
            raise ValueError("the fake soundfile reads only an open float32 stream")
        return (np.zeros(self.samples, dtype=np.float32), 16000)


def response_view(values: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "text": values["text"],
        "task": values["task"],
        "words": values.get("words"),
        "duration": values.get("duration"),
    }


class RecordingLogging(Logging):
    @override
    def __init__(self, record: Record) -> None:
        self.record = record

    @override
    def pre_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        self.record.pre = {
            "api_base": arguments["api_base"],
            "atranscription": arguments["atranscription"],
            "complete_input_dict": arguments["complete_input_dict"],
        }

    @override
    def post_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        response = response_view(OBJECT.validate_python(kwargs["original_response"]))
        self.record.post = {
            "input": kwargs["input"],
            "complete_input_dict": arguments["complete_input_dict"],
            "original_response": response,
        }
        self.record.response = response


def restore_modules(saved: dict[str, ModuleType | None]) -> None:
    for name, module in saved.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module


def install(record: Record, call: Call) -> None:
    for name in MANAGED:
        sys.modules.pop(name, None)
    scripts = Scripts(record, call)
    client = FakeRivaClient(scripts)
    sys.modules["riva"] = FakeRivaPackage(client)
    sys.modules["riva.client"] = client
    sys.modules["soundfile"] = FakeSoundFile(call["samples"])


def hidden_view(response: TranscriptionResponse) -> dict[str, JsonValue]:
    hidden = HiddenRecord.model_validate(response, from_attributes=True).hidden_params
    return {
        "model": hidden["model"],
        "custom_llm_provider": hidden["custom_llm_provider"],
        "audio_transcription_duration": hidden["audio_transcription_duration"],
    }


def base_answer(record: Record) -> Answer:
    return {
        "status": None,
        "message": None,
        "events": record.events,
        "auth": {"kwargs": record.auth_kwargs, "args": record.auth_args},
        "recognition": record.recognition,
        "endpointing": {"kwargs": record.endpointing_kwargs, "copied": record.endpointing_copied},
        "streaming": {
            "interim_results": record.interim_results,
            "config_is_recognition": record.config_is_recognition,
        },
        "chunk_sizes": record.chunk_sizes,
        "timeout_seconds": record.timeout_seconds,
        "pre": record.pre,
        "post": record.post,
        "response": record.response,
        "hidden": None,
    }


def failure_answer(record: Record, error: ErrorRecord) -> Answer:
    answer = base_answer(record)
    answer["status"] = error.status_code
    answer["message"] = error.message
    return answer


def success_answer(record: Record, response: TranscriptionResponse) -> Answer:
    answer = base_answer(record)
    answer["hidden"] = hidden_view(response)
    return answer


def perform(call: Call) -> Answer:
    record = Record()
    saved = {name: sys.modules.get(name) for name in MANAGED}
    try:
        install(record, call)
        model_response = TranscriptionResponse(text="")
        arguments: TranscriptionArguments = {
            "model": call["model"],
            "audio_file": NamedAudio(call["audio_name"], bytes.fromhex(call["file_hex"])),
            "optional_params": call["optional_params"],
            "litellm_params": {},
            "model_response": model_response,
            "timeout": call["timeout_seconds"],
            "logging_obj": RecordingLogging(record),
            "api_key": call["api_key"],
            "api_base": call["api_base"],
            "atranscription": False,
        }
        try:
            HANDLER.audio_transcriptions(**arguments)
        except NvidiaRivaException as error:
            return failure_answer(record, ErrorRecord.model_validate(error, from_attributes=True))
        else:
            return success_answer(record, model_response)
    finally:
        restore_modules(saved)


def run(input_value: Input) -> JsonValue:
    answers = [perform(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
