import base64
import json
import os
from typing import Final, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.mistral.audio_speech.transformation import (
    MistralTextToSpeechConfig as Speech,
)
from litellm.llms.mistral.audio_transcription.transformation import (
    MistralAudioTranscriptionConfig as Transcription,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

Object = dict[str, Json]


class LoggingStub(Logging):
    def __init__(self) -> None: ...


LOGGING: Final = LoggingStub()


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class SpeechProbe(TypedDict, total=False):
    voice: str | dict[str, Json]
    optional_params: Object
    kwargs: Object
    reply: Reply


class SpeechInput(TypedDict):
    probes: list[SpeechProbe]


class TranscriptionProbe(TypedDict):
    model: str
    filename: str
    content: str
    optional_params: Object
    non_default_params: Object
    headers: dict[str, str]
    api_key: str | None
    environment_api_key: str | None
    api_base: str | None
    reply: Reply | None


class TranscriptionInput(TypedDict):
    probes: list[TranscriptionProbe]


class SpeechConfig(Protocol):
    def map_openai_params(
        self,
        _model: str,
        _optional_params: Object,
        _voice: str | dict[str, Json] | None,
        /,
        *,
        drop_params: bool,
        kwargs: Object | None,
    ) -> tuple[str | None, Object]: ...


class TranscriptionConfig(Protocol):
    def map_openai_params(
        self,
        _non_default_params: Object,
        _optional_params: Object,
        _model: str,
        /,
        *,
        drop_params: bool,
    ) -> Object: ...

    def validate_environment(
        self,
        _headers: dict[str, str],
        _model: str,
        _messages: list[AllMessageValues],
        _optional_params: Object,
        _litellm_params: Object,
        /,
        *,
        api_key: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self,
        _api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: Object,
        _litellm_params: Object,
        /,
    ) -> str: ...


class SpeechFactory(Protocol):
    def __call__(self) -> Speech: ...


class TranscriptionFactory(Protocol):
    def __call__(self) -> Transcription: ...


class Record(Protocol):
    data: Object | bytes
    files: dict[str, tuple[str, bytes, str]] | None


class Private(TypedDict):
    _hidden_params: Object


class NoMessages(list[AllMessageValues]):
    pass


SPEECH: Final[SpeechFactory] = Speech
TRANSCRIPTION: Final[TranscriptionFactory] = Transcription
EMPTY: Final[Object] = {}
DUMP: Final = TypeAdapter(dict[str, Json])
HIDDEN: Final = TypeAdapter(Private)


def or_empty(value: Object | None) -> Object:
    if value is None:
        return {}
    return value


def string_map(pairs: dict[str, str]) -> Object:
    fields: Object = {}
    for name, value in pairs.items():
        fields[name] = value
    return fields


def install_environment(key: str | None) -> None:
    os.environ.clear()
    if key is not None:
        os.environ["MISTRAL_API_KEY"] = key


def speech_response(probe: SpeechProbe, params: Object) -> Json:
    fixture = probe.get("reply")
    if fixture is None:
        return None
    payload: Object = {}
    if "response_format" in params:
        payload["response_format"] = params["response_format"]
    request = httpx.Request(
        "POST", "https://mistral.test/audio/speech", content=json.dumps(payload)
    )
    reply = httpx.Response(
        fixture["status"],
        headers=fixture["headers"],
        content=fixture["body"].encode(),
        request=request,
    )
    binary = Speech.transform_text_to_speech_response(SPEECH(), "voxtral", reply, LOGGING)
    return {
        "status": binary.response.status_code,
        "headers": string_map(dict(binary.response.headers)),
        "body": base64.b64encode(binary.content).decode(),
    }


def speech_probe(view: SpeechConfig, probe: SpeechProbe) -> Json:
    voice, params = view.map_openai_params(
        "voxtral",
        or_empty(probe.get("optional_params")),
        probe.get("voice"),
        drop_params=False,
        kwargs=or_empty(probe.get("kwargs")),
    )
    response = speech_response(probe, params)
    return {"mapping": {"voice": voice, "params": params}, "response": response}


def run(input_value: SpeechInput) -> Json:
    config = SPEECH()
    return [speech_probe(config, probe) for probe in input_value["probes"]]


def form_value(data: Object | bytes) -> Object:
    match data:
        case dict() as fields:
            return fields
        case bytes():
            raise ValueError("the transcription form is not a mapping")


def request_value(record: Record) -> Json:
    files = record.files
    if files is None:
        raise ValueError("the transcription request carries no file")
    filename, content, content_type = files["file"]
    return {
        "form": form_value(record.data),
        "filename": filename,
        "content_type": content_type,
        "content": base64.b64encode(content).decode(),
    }


def response_value(config: Transcription, fixture: Reply) -> Json:
    reply = httpx.Response(
        fixture["status"],
        headers=fixture["headers"],
        content=fixture["body"].encode(),
    )
    response: TranscriptionResponse = Transcription.transform_audio_transcription_response(
        config, reply
    )
    dumped = DUMP.validate_json(response.model_dump_json())
    fields: Object = {}
    for name in ("segments", "language"):
        if name in dumped:
            fields[name] = dumped[name]
    private: dict[str, Json] | None = response.__pydantic_private__
    hidden = HIDDEN.validate_python(private)["_hidden_params"]
    return {"text": response.text, "fields": fields, "hidden": hidden}


def transcription_probe(view: TranscriptionConfig, probe: TranscriptionProbe) -> Json:
    mapped = view.map_openai_params(
        probe["non_default_params"],
        probe["optional_params"],
        probe["model"],
        drop_params=False,
    )
    headers = string_map(
        view.validate_environment(
            probe["headers"],
            probe["model"],
            NoMessages(),
            EMPTY,
            EMPTY,
            api_key=probe["api_key"],
        )
    )
    record = Transcription.transform_audio_transcription_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/audio_transcription/transformation.py#L96-L104
        TRANSCRIPTION(),
        probe["model"],
        (probe["filename"], probe["content"].encode()),
        mapped,
        EMPTY,
    )
    reply = probe["reply"]
    return {
        "headers": headers,
        "url": view.get_complete_url(
            probe["api_base"], probe["api_key"], probe["model"], EMPTY, EMPTY
        ),
        "request": request_value(record),
        "response": None if reply is None else response_value(TRANSCRIPTION(), reply),
    }


def transcription_run(input_value: TranscriptionInput) -> Json:
    config = TRANSCRIPTION()
    values: list[Json] = []
    for probe in input_value["probes"]:
        install_environment(probe["environment_api_key"])
        values.append(transcription_probe(config, probe))
    return values
