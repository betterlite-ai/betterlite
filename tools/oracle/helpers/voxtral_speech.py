import os
from typing import Final, Protocol, TypedDict

import litellm
from litellm.llms.mistral.audio_speech.transformation import (
    MistralTextToSpeechConfig as Speech,
)
from litellm.llms.mistral.audio_transcription.transformation import (
    MistralAudioTranscriptionConfig as Transcription,
)
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue as Json

Object = dict[str, Json]
EMPTY: Final[Object] = {}

litellm.suppress_debug_info = True


class SpeechProbe(TypedDict, total=False):
    voice: str | dict[str, Json]
    optional_params: Object
    kwargs: Object


class SpeechInput(TypedDict):
    probes: list[SpeechProbe]


class TranscriptionProbe(TypedDict):
    model: str
    optional_params: Object
    non_default_params: Object
    headers: dict[str, str]
    api_key: str | None
    environment_api_key: str | None


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


class SpeechFactory(Protocol):
    def __call__(self) -> Speech: ...


class TranscriptionFactory(Protocol):
    def __call__(self) -> Transcription: ...


class NoMessages(list[AllMessageValues]):
    pass


SPEECH: Final[SpeechFactory] = Speech
TRANSCRIPTION: Final[TranscriptionFactory] = Transcription


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


def speech_probe(view: SpeechConfig, probe: SpeechProbe) -> Json:
    voice, params = view.map_openai_params(
        "voxtral",
        or_empty(probe.get("optional_params")),
        probe.get("voice"),
        drop_params=False,
        kwargs=or_empty(probe.get("kwargs")),
    )
    return {"voice": voice, "params": params}


def run(input_value: SpeechInput) -> Json:
    config = SPEECH()
    return [speech_probe(config, probe) for probe in input_value["probes"]]


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
    return {"headers": headers, "mapped": mapped}


def transcription_run(input_value: TranscriptionInput) -> Json:
    config = TRANSCRIPTION()
    values: list[Json] = []
    for probe in input_value["probes"]:
        install_environment(probe["environment_api_key"])
        values.append(transcription_probe(config, probe))
    return values
