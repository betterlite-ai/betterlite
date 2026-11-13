import base64
import json
import os
from typing import Final, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.mistral.audio_speech.transformation import (
    MistralTextToSpeechConfig as Config,
)
from pydantic import JsonValue as Json

Object = dict[str, Json]


class LoggingStub(Logging):
    def __init__(self) -> None: ...


LOGGING: Final = LoggingStub()


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Probe(TypedDict, total=False):
    voice: str | dict[str, Json]
    optional_params: Object
    kwargs: Object
    headers: dict[str, str]
    api_key: str
    environment_api_key: str
    reply: Reply


class Input(TypedDict):
    probes: list[Probe]


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

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...


class Factory(Protocol):
    def __call__(self) -> Config: ...


MAKE: Factory = Config


def object_field(probe: Probe) -> tuple[Object, Object]:
    optional = probe.get("optional_params")
    if optional is None:
        optional = {}
    kwargs = probe.get("kwargs")
    if kwargs is None:
        kwargs = {}
    return optional, kwargs


def string_map(pairs: dict[str, str]) -> Object:
    fields: Object = {}
    for name, value in pairs.items():
        fields[name] = value
    return fields


def speech_response(probe: Probe, params: Object) -> Json:
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
    binary = Config.transform_text_to_speech_response(MAKE(), "voxtral", reply, LOGGING)
    return {
        "status": binary.response.status_code,
        "headers": string_map(dict(binary.response.headers)),
        "body": base64.b64encode(binary.content).decode(),
    }


def probe_value(view: SpeechConfig, probe: Probe) -> Json:
    optional, kwargs = object_field(probe)
    voice, params = view.map_openai_params(
        "voxtral", optional, probe.get("voice"), drop_params=False, kwargs=kwargs
    )
    caller = probe.get("headers")
    if caller is None:
        caller = {}
    return {
        "mapping": {"voice": voice, "params": params},
        "headers": string_map(
            view.validate_environment(caller, "voxtral", probe.get("api_key"))
        ),
        "response": speech_response(probe, params),
    }


def run(input_value: Input) -> Json:
    config = MAKE()
    values: list[Json] = []
    for probe in input_value["probes"]:
        os.environ.clear()
        key = probe.get("environment_api_key")
        if key is not None:
            os.environ["MISTRAL_API_KEY"] = key
        values.append(probe_value(config, probe))
    return values
