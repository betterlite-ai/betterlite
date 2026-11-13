import base64
import os
from typing import Final, Protocol, TypedDict

import httpx
from litellm.llms.mistral.audio_transcription.transformation import (
    MistralAudioTranscriptionConfig,
)
from litellm.types.llms.openai import (
    AllMessageValues,
    OpenAIAudioTranscriptionOptionalParams,
)
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

Object = dict[str, Json]


class Required(TypedDict):
    model: str
    filename: str
    content: str


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Probe(Required, total=False):
    optional_params: Object
    non_default_params: Object
    headers: dict[str, str]
    api_key: str
    environment_api_key: str
    api_base: str
    reply: Reply


class Input(TypedDict):
    probes: list[Probe]


class Dumped(TypedDict, total=False):
    segments: Json
    language: Json


DUMPED: Final = TypeAdapter(Dumped)


class Private(TypedDict):
    _hidden_params: Object


PRIVATE: Final = TypeAdapter(Private)


class RequestRecord(Protocol):
    data: Object | bytes
    files: dict[str, tuple[str, bytes, str]] | None


class TranscriptionConfig(Protocol):
    def get_supported_openai_params(
        self, model: str
    ) -> list[OpenAIAudioTranscriptionOptionalParams]: ...

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
        api_base: str | None = None,
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

    def transform_audio_transcription_request(
        self,
        _model: str,
        _audio_file: tuple[str, bytes],
        _optional_params: Object,
        _litellm_params: Object,
        /,
    ) -> RequestRecord: ...

    def transform_audio_transcription_response(
        self, _raw_response: httpx.Response, /
    ) -> TranscriptionResponse: ...


class TranscriptionFactory(Protocol):
    def __call__(self) -> TranscriptionConfig: ...


class NoMessages(list[AllMessageValues]):
    pass


FACTORY: TranscriptionFactory = MistralAudioTranscriptionConfig


def install_environment(probe: Probe) -> None:
    os.environ.clear()
    key = probe.get("environment_api_key")
    if key is not None:
        os.environ["MISTRAL_API_KEY"] = key


def string_map(pairs: dict[str, str]) -> Object:
    fields: Object = {}
    for name, value in pairs.items():
        fields[name] = value
    return fields


def form_value(data: Object | bytes) -> Object:
    match data:
        case dict() as fields:
            return fields
        case bytes():
            raise ValueError("the Mistral transcription form is not a mapping")


def record_view(record: RequestRecord) -> Json:
    data = form_value(record.data)
    files = record.files
    if files is None:
        raise ValueError("the Mistral transcription request carries no file")
    filename, content, content_type = files["file"]
    return {
        "form": data,
        "filename": filename,
        "content_type": content_type,
        "content": base64.b64encode(content).decode(),
    }


def response_value(config: TranscriptionConfig, fixture: Reply) -> Json:
    reply = httpx.Response(
        fixture["status"],
        headers=fixture["headers"],
        content=fixture["body"].encode(),
    )
    response = config.transform_audio_transcription_response(reply)
    dumped = DUMPED.validate_json(response.model_dump_json())
    fields: Object = {}
    if "segments" in dumped:
        fields["segments"] = dumped["segments"]
    if "language" in dumped:
        fields["language"] = dumped["language"]
    dumped_private: dict[str, Json] | None = response.__pydantic_private__
    hidden = PRIVATE.validate_python(dumped_private)["_hidden_params"]
    return {"text": response.text, "fields": fields, "hidden": hidden}


def probe_value(config: TranscriptionConfig, probe: Probe) -> Json:
    model = probe["model"]
    optional_params = probe.get("optional_params")
    if optional_params is None:
        optional_params = {}
    non_default_params = probe.get("non_default_params")
    if non_default_params is None:
        non_default_params = {}
    mapped = config.map_openai_params(
        non_default_params, optional_params, model, drop_params=False
    )
    caller = probe.get("headers")
    if caller is None:
        caller = {}
    headers = string_map(
        config.validate_environment(
            caller, model, NoMessages(), {}, {}, api_key=probe.get("api_key")
        )
    )
    record = config.transform_audio_transcription_request(
        model, (probe["filename"], probe["content"].encode()), mapped, {}
    )
    fixture = probe.get("reply")
    return {
        "supported": list(config.get_supported_openai_params(model)),
        "mapped": mapped,
        "headers": headers,
        "url": config.get_complete_url(
            probe.get("api_base"), probe.get("api_key"), model, {}, {}
        ),
        "request": record_view(record),
        "response": None if fixture is None else response_value(config, fixture),
    }


def run(input_value: Input) -> Json:
    config = FACTORY()
    values: list[Json] = []
    for probe in input_value["probes"]:
        install_environment(probe)
        values.append(probe_value(config, probe))
    return values
