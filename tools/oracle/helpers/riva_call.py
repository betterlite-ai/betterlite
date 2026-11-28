import sys
from io import BytesIO
from types import ModuleType
from typing import Final, Protocol, TypedDict, Unpack, runtime_checkable

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.nvidia_riva.audio_transcription.handler import NvidiaRivaAudioTranscription
from litellm.llms.nvidia_riva.common_utils import NvidiaRivaException
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from riva_call_fakes import Call, FakeRivaClient, FakeRivaPackage, FakeSoundFile, Record

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
CONFIG: Final = ConfigDict(arbitrary_types_allowed=True)
MANAGED: Final = ("riva", "riva.client", "soundfile")


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


def install(record: Record, call: Call) -> None:
    for name in MANAGED:
        sys.modules.pop(name, None)
    client = FakeRivaClient(record, call)
    sys.modules["riva"] = FakeRivaPackage(client)
    sys.modules["riva.client"] = client
    sys.modules["soundfile"] = FakeSoundFile(call["samples"])


def restore_modules(saved: dict[str, ModuleType | None]) -> None:
    for name, module in saved.items():
        if module is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = module


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
        except NvidiaRivaException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/nvidia_riva/common_utils.py#L10
            return failure_answer(record, ErrorRecord.model_validate(error, from_attributes=True))
        else:
            return success_answer(record, model_response)
    finally:
        restore_modules(saved)


def run(input_value: Input) -> JsonValue:
    answers = [perform(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
