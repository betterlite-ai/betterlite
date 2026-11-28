import json
from collections.abc import Iterable, Iterator
from io import BytesIO
from types import ModuleType
from typing import Final, Literal, Self, TypedDict, Unpack

import litellm
import numpy as np
from numpy.typing import NDArray
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

ServiceKind = Literal["timeout", "variadic", "plain"]
EndpointingMode = Literal["ok", "fails", "copy_fails", "missing_field"]
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
LIST: Final = TypeAdapter(list[JsonValue])


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


class Actor:
    def __init__(self, record: Record, call: Call) -> None:
        self.record = record
        self.call = call


class AuthFactory(Actor):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> Self:
        if kwargs:
            self.record.events.append("auth.kwargs")
            self.record.auth_kwargs = OBJECT.validate_json(json.dumps(kwargs))
            if self.call["legacy_auth"]:
                raise TypeError("this riva-client takes positional auth arguments")
            return self
        self.record.events.append("auth.positional")
        self.record.auth_args = LIST.validate_json(json.dumps(args))
        return self


class EndpointingInstance:
    def __init__(self, values: dict[str, JsonValue]) -> None:
        self.values = values


class CopyCall(Actor):
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


class EndpointingFactory(Actor):
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


class RecognitionFactory(Actor):
    def __init__(self, record: Record, call: Call) -> None:
        super().__init__(record, call)
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


class StreamingFactory(Actor):
    def __init__(self, record: Record, call: Call, recognition: RecognitionFactory) -> None:
        super().__init__(record, call)
        self.recognition = recognition
        self.built: StreamingInstance | None = None

    def __call__(self, **kwargs: JsonValue) -> StreamingInstance:
        self.record.events.append("streaming")
        self.record.interim_results = kwargs["interim_results"]
        built = self.recognition.built
        self.record.config_is_recognition = built is not None and id(kwargs["config"]) == id(built)
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


class FakeRivaClient(ModuleType):
    def __init__(self, record: Record, call: Call) -> None:
        super().__init__("riva.client")
        recognition = RecognitionFactory(record, call)
        streaming = StreamingFactory(record, call, recognition)
        self.Auth = AuthFactory(record, call)
        self.ASRService = ASRServiceFactory(record, call, streaming)
        self.RecognitionConfig = recognition
        self.StreamingRecognitionConfig = streaming
        self.EndpointingConfig = EndpointingFactory(record, call)
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
