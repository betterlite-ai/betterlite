import base64
import os
from typing import Final, Protocol, TypedDict

import httpx
from litellm.llms.mistral.audio_transcription.transformation import (
    MistralAudioTranscriptionConfig as Config,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

Object = dict[str, Json]


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Probe(TypedDict):
    model: str
    filename: str
    content: str
    optional_params: dict[str, Json]
    non_default_params: dict[str, Json]
    headers: dict[str, str]
    api_key: str | None
    environment_api_key: str | None
    api_base: str | None
    reply: Reply | None


class Input(TypedDict):
    probes: list[Probe]


class RequestRecord(Protocol):
    data: Object | bytes
    files: dict[str, tuple[str, bytes, str]] | None


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


class Factory(Protocol):
    def __call__(self) -> Config: ...


class NoMessages(list[AllMessageValues]):
    pass


class Private(TypedDict):
    _hidden_params: Object


MAKE: Factory = Config
EMPTY: Final[Object] = {}
FIELDS: Final = TypeAdapter(dict[str, Json])
PRIVATE: Final = TypeAdapter(Private)


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
            raise ValueError("the transcription form is not a mapping")


def request_value(record: RequestRecord) -> Json:
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


def response_value(config: Config, fixture: Reply) -> Json:
    reply = httpx.Response(
        fixture["status"],
        headers=fixture["headers"],
        content=fixture["body"].encode(),
    )
    response: TranscriptionResponse = Config.transform_audio_transcription_response(
        config, reply
    )
    dumped = FIELDS.validate_json(response.model_dump_json())
    fields: Object = {}
    for name in ("segments", "language"):
        if name in dumped:
            fields[name] = dumped[name]
    private: dict[str, Json] | None = response.__pydantic_private__
    hidden = PRIVATE.validate_python(private)["_hidden_params"]
    return {"text": response.text, "fields": fields, "hidden": hidden}


def probe_value(view: TranscriptionConfig, probe: Probe) -> Json:
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
            {},
            {},
            api_key=probe["api_key"],
        )
    )
    record = Config.transform_audio_transcription_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/audio_transcription/transformation.py#L96-L104
        MAKE(),
        probe["model"],
        (probe["filename"], probe["content"].encode()),
        mapped,
        EMPTY,
    )
    reply = probe["reply"]
    return {
        "mapped": mapped,
        "headers": headers,
        "url": view.get_complete_url(
            probe["api_base"], probe["api_key"], probe["model"], EMPTY, EMPTY
        ),
        "request": request_value(record),
        "response": None if reply is None else response_value(MAKE(), reply),
    }


def install_environment(probe: Probe) -> None:
    os.environ.clear()
    key = probe["environment_api_key"]
    if key is not None:
        os.environ["MISTRAL_API_KEY"] = key


def run(input_value: Input) -> Json:
    config = MAKE()
    values: list[Json] = []
    for probe in input_value["probes"]:
        install_environment(probe)
        values.append(probe_value(config, probe))
    return values
