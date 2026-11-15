from typing import Literal, Protocol, Required, TypedDict

from litellm.llms.nvidia_riva.audio_transcription.transformation import (
    NvidiaRivaAudioTranscriptionConfig as RivaConfig,
)
from litellm.llms.nvidia_riva.common_utils import grpc_error_to_litellm_exception
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue, TypeAdapter

_MODEL: str = "openai/whisper-1"
_OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
_FINALS: TypeAdapter[list[dict[str, JsonValue]]] = TypeAdapter(list[dict[str, JsonValue]])

Function = Literal[
    "language_cases",
    "timing_flags",
    "endpointing_precedence",
    "empty_model",
    "concatenated_finals",
    "verbose_words",
    "grpc_error_table",
]


class Word(TypedDict):
    word: str
    start_time_ms: int
    end_time_ms: int


class FinalResult(TypedDict):
    transcript: str
    words: list[Word]


AttributeKind = Literal["missing", "not_callable", "raises", "returns"]


class Attribute(TypedDict, total=False):
    kind: Required[AttributeKind]
    name: JsonValue
    value: JsonValue


class ErrorSpec(TypedDict):
    code: Attribute
    details: Attribute
    text: str


class Call(TypedDict):
    function: Function
    params: dict[str, JsonValue]
    final_results: list[FinalResult]
    response_format: str
    duration_seconds: float | None
    timestamp_granularities: list[str] | None
    error: ErrorSpec


class Input(TypedDict):
    calls: list[Call]


class RequestData(Protocol):
    data: dict[str, JsonValue] | bytes


# The source's bare dict annotations leave its methods partly unknown; this names the shapes we pass
class Transformation(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        audio_file: bytes,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> RequestData: ...

    @staticmethod
    def build_transcription_response(
        final_results: list[dict[str, JsonValue]],
        response_format: str,
        duration_seconds: float | None,
        timestamp_granularities: list[str] | None,
    ) -> TranscriptionResponse: ...


class ConfigFactory(Protocol):
    def __call__(self) -> Transformation: ...


CONFIG: ConfigFactory = RivaConfig


class FakeCode:
    def __init__(self, name: JsonValue) -> None:
        self.name = name


class Returns:
    def __init__(self, value: JsonValue | FakeCode) -> None:
        self.value = value

    def __call__(self) -> JsonValue | FakeCode:
        return self.value


class Raises:
    def __init__(self, message: str) -> None:
        self.message = message

    def __call__(self) -> JsonValue:
        raise RuntimeError(self.message)


class FakeError(Exception):
    code: JsonValue | Returns | Raises = None
    details: JsonValue | Returns | Raises = None


def attribute_value(
    kind: AttributeKind, payload: JsonValue | FakeCode
) -> JsonValue | Returns | Raises:
    if kind == "returns":
        return Returns(payload)
    if kind == "raises":
        return Raises("the attribute raised")
    if kind == "not_callable":
        return "not callable"
    return None


def fake_error(spec: ErrorSpec) -> FakeError:
    error = FakeError(spec["text"])
    code = spec["code"]
    details = spec["details"]
    error.code = attribute_value(code["kind"], FakeCode(code.get("name")))
    error.details = attribute_value(details["kind"], details.get("value"))
    return error


def transform(call: Call) -> JsonValue:
    config = CONFIG()
    optional = config.map_openai_params(
        non_default_params=call["params"],
        optional_params={},
        model=_MODEL,
        drop_params=False,
    )
    request = config.transform_audio_transcription_request(
        model=_MODEL,
        audio_file=b"",
        optional_params=optional,
        litellm_params={},
    )
    return _OBJECT.validate_python(request.data)


def transcription(call: Call) -> JsonValue:
    built = CONFIG().build_transcription_response(
        final_results=_FINALS.validate_python(call["final_results"]),
        response_format=call["response_format"],
        duration_seconds=call["duration_seconds"],
        timestamp_granularities=call["timestamp_granularities"],
    )
    payload = _OBJECT.validate_json(built.model_dump_json())
    # usage is the response class's unset declared field, not something this method writes
    payload.pop("usage", None)
    return payload


def grpc(call: Call) -> JsonValue:
    exception = grpc_error_to_litellm_exception(fake_error(call["error"]))
    return {"status": exception.status_code, "message": exception.message}


def answer(call: Call) -> JsonValue:
    if call["function"] == "grpc_error_table":
        return grpc(call)
    if call["function"] in ("concatenated_finals", "verbose_words"):
        return transcription(call)
    return transform(call)


def run(input_value: Input) -> JsonValue:
    calls = input_value["calls"]
    return [answer(call) for call in calls]
