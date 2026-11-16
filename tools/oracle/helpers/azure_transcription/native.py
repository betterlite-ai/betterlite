import base64
import os
from collections.abc import Callable, Generator, Sequence
from contextlib import contextmanager
from functools import partial
from typing import Final, Protocol

import httpx
import litellm
from litellm.llms.base_llm.audio_transcription.transformation import (
    BaseAudioTranscriptionConfig,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import FileTypes, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel
from pydantic import JsonValue as Json

from openai_transcription.exchange import PRIVATE

from .probe import PROVIDER_ERROR, Probe

litellm.suppress_debug_info = True

UNKNOWN_MODEL: Final = "speech/azure-stt"


class NoMessages(list[AllMessageValues]):
    pass


class ErrorStatus(BaseModel):
    status_code: int


class RequestView(Protocol):
    @property
    def data(self) -> dict[str, Json] | bytes: ...

    @property
    def content_type(self) -> str | None: ...


class ConfigView(Protocol):
    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...

    def map_openai_params(
        self,
        *,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def validate_environment(
        self,
        _headers: dict[str, Json],
        _model: str,
        _messages: list[AllMessageValues],
        _optional_params: dict[str, Json],
        _litellm_params: dict[str, Json],
        _api_key: str | None,
        /,
    ) -> dict[str, Json]: ...

    def get_complete_url(
        self,
        *,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_audio_transcription_request(
        self,
        _model: str,
        _audio_file: FileTypes,
        _optional_params: dict[str, Json],
        _litellm_params: dict[str, Json],
        /,
    ) -> RequestView: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException: ...


def native_audio_config() -> BaseAudioTranscriptionConfig:
    config = ProviderConfigManager.get_provider_audio_transcription_config(
        model=UNKNOWN_MODEL, provider=LlmProviders.AZURE
    )
    if config is None:
        raise LookupError(UNKNOWN_MODEL)
    return config


@contextmanager
def environment(name: str, value: str | None) -> Generator[None]:
    previous = os.environ.get(name)
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = previous


# the native config raises BaseLLMException, and its status is what the case compares
def captured(call: Callable[[], Json]) -> Json:
    try:
        return call()
    except PROVIDER_ERROR as failure:
        status = ErrorStatus.model_validate(failure, from_attributes=True).status_code
        return {"ok": False, "status": status}


def headers_view(config: ConfigView, probe: Probe) -> Json:
    empty: dict[str, Json] = {}
    with environment("AZURE_SPEECH_API_KEY", probe.environment_api_key):
        validated = config.validate_environment(
            dict(probe.headers), probe.model, NoMessages(), empty, empty, probe.api_key
        )
    return {"ok": True, "value": validated}


def url_view(config: ConfigView, probe: Probe, base: str | None) -> Json:
    empty: dict[str, Json] = {}
    with environment("AZURE_SPEECH_API_BASE", probe.environment_api_base):
        url = config.get_complete_url(
            api_base=base,
            api_key=None,
            model=probe.model,
            optional_params=dict(probe.optional_params),
            litellm_params=empty,
        )
    return {"ok": True, "url": url}


def response_view(config: BaseAudioTranscriptionConfig, raw: httpx.Response) -> Json:
    response = config.transform_audio_transcription_response(raw)
    private: dict[str, Json] | None = response.__pydantic_private__
    hidden = PRIVATE.validate_python(private)["_hidden_params"]
    return {"ok": True, "text": response.text, "hidden": hidden}


def native_config(config: ConfigView, probe: Probe) -> Json:
    supported: list[Json] = list(config.get_supported_openai_params(model=probe.model))
    mapped = config.map_openai_params(
        non_default_params=dict(probe.non_default_params),
        optional_params=dict(probe.existing_params),
        model=probe.model,
        drop_params=False,
    )
    error = config.get_error_class("boom", 502, {"x-request-id": "req-1"})
    urls: list[Json] = [captured(partial(url_view, config, probe, base)) for base in probe.bases]
    return {
        "supported": supported,
        "mapped": mapped,
        "headers": captured(partial(headers_view, config, probe)),
        "urls": urls,
        "error_status": error.status_code,
    }


def native_response(config: BaseAudioTranscriptionConfig, probe: Probe) -> Json:
    results: list[Json] = [
        captured(
            partial(
                response_view,
                config,
                httpx.Response(
                    item.status, headers=dict(probe.headers), content=item.body.encode()
                ),
            )
        )
        for item in probe.bodies
    ]
    return {"results": results}


def native_request(config: ConfigView, probe: Probe) -> Json:
    empty: dict[str, Json] = {}
    request = config.transform_audio_transcription_request(
        probe.model, (probe.audio, probe.audio.encode(), "audio/wav"), empty, empty
    )
    match request.data:
        case bytes() as content:
            return {
                "kind": "bytes",
                "body": base64.b64encode(content).decode(),
                "content_type": request.content_type,
            }
        case dict():
            pass
    raise TypeError("the native request data is not bytes")
