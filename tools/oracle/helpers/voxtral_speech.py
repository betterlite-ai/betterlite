import base64
import datetime as dt
import json
import os
from typing import Final, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.mistral.audio_speech.transformation import (
    MistralTextToSpeechConfig,
    MistralTextToSpeechException,
)
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import JsonValue as Json

Object = dict[str, Json]
MESSAGES: Final[list[dict[str, Json]]] = []
LOGGING: Final = LiteLLMLoggingObj(
    model="voxtral",
    messages=MESSAGES,
    stream=False,
    call_type="text_to_speech",
    start_time=dt.datetime(2026, 1, 1, tzinfo=dt.UTC),
    litellm_call_id="voxtral-speech",
    function_id="voxtral_speech",
)


class Required(TypedDict):
    model: str
    input: str


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Probe(Required, total=False):
    voice: str | dict[str, Json]
    optional_params: Object
    kwargs: Object
    headers: dict[str, str]
    api_key: str
    environment_api_key: str
    api_base: str
    reply: Reply


class Input(TypedDict):
    probes: list[Probe]


class SpeechBody(TypedDict, total=False):
    dict_body: Object
    headers: dict[str, str]


class SpeechConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

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
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: Object
    ) -> str: ...

    def transform_text_to_speech_request(
        self,
        _model: str,
        _input: str,
        _voice: str | None,
        _optional_params: Object,
        _litellm_params: Object,
        _headers: dict[str, str],
        /,
    ) -> SpeechBody: ...

    def transform_text_to_speech_response(
        self, model: str, raw_response: httpx.Response, logging_obj: LiteLLMLoggingObj
    ) -> HttpxBinaryResponseContent: ...


class SpeechFactory(Protocol):
    def __call__(self) -> SpeechConfig: ...


FACTORY: SpeechFactory = MistralTextToSpeechConfig


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


def reply_request(body: Object) -> httpx.Request:
    return httpx.Request(
        "POST", "https://mistral.test/audio/speech", content=json.dumps(body)
    )


def reply_response(fixture: Reply, request: httpx.Request) -> httpx.Response:
    return httpx.Response(
        fixture["status"],
        headers=fixture["headers"],
        content=fixture["body"].encode(),
        request=request,
    )


def request_value(request: SpeechBody) -> Json:
    body = request.get("dict_body")
    headers = request.get("headers")
    return {
        "dict_body": body,
        "headers": None if headers is None else string_map(headers),
    }


def probe_value(config: SpeechConfig, probe: Probe) -> Json:
    model = probe["model"]
    optional_params = probe.get("optional_params")
    if optional_params is None:
        optional_params = {}
    kwargs = probe.get("kwargs")
    if kwargs is None:
        kwargs = {}
    voice, params = config.map_openai_params(
        model, optional_params, probe.get("voice"), drop_params=False, kwargs=kwargs
    )
    caller = probe.get("headers")
    if caller is None:
        caller = {}
    headers = string_map(
        config.validate_environment(
            headers=caller,
            model=model,
            api_key=probe.get("api_key"),
        )
    )
    request = config.transform_text_to_speech_request(
        model, probe["input"], voice, params, {}, {}
    )
    fixture = probe.get("reply")
    response: Json = None
    if fixture is not None:
        body = request.get("dict_body")
        if body is None:
            raise ValueError("the speech request carries no body")
        reply = reply_response(fixture, reply_request(body))
        binary = config.transform_text_to_speech_response(
            model=model, raw_response=reply, logging_obj=LOGGING
        )
        response = {
            "status": binary.response.status_code,
            "headers": string_map(dict(binary.response.headers)),
            "body": base64.b64encode(binary.content).decode(),
        }
    return {
        "supported": list(config.get_supported_openai_params(model)),
        "mapping": {"voice": voice, "params": params},
        "headers": headers,
        "url": config.get_complete_url(
            model=model, api_base=probe.get("api_base"), litellm_params={}
        ),
        "request": request_value(request),
        "response": response,
    }


def raised_value(config: SpeechConfig, probe: Probe) -> Json:
    try:
        return probe_value(config, probe)
    except MistralTextToSpeechException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L63
        return {"raised": {"status": error.status_code}}


def run(input_value: Input) -> Json:
    config = FACTORY()
    values: list[Json] = []
    for probe in input_value["probes"]:
        install_environment(probe)
        values.append(raised_value(config, probe))
    return values
