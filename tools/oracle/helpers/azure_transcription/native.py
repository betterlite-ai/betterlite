import base64
import os
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from typing import Final, Protocol

import httpx
import litellm
from litellm.llms.base_llm.audio_transcription.transformation import (
    BaseAudioTranscriptionConfig,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue as Json

from openai_transcription.exchange import PRIVATE

from .probe import Probe

litellm.suppress_debug_info = True

UNKNOWN_MODEL: Final = "speech/azure-stt"


class NoMessages(list[AllMessageValues]):
    pass


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
def environment(name: str, value: str | None) -> Iterator[None]:
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


def checked_headers(config: ConfigView, probe: Probe) -> Json:
    empty: dict[str, Json] = {}
    try:
        with environment("AZURE_SPEECH_API_KEY", probe.environment_api_key):
            validated = config.validate_environment(
                dict(probe.headers), probe.model, NoMessages(), empty, empty, probe.api_key
            )
    except BaseLLMException as failure:
        return {"ok": False, "status": failure.status_code}
    return {"ok": True, "value": validated}


def checked_urls(config: ConfigView, probe: Probe) -> list[Json]:
    empty: dict[str, Json] = {}
    urls: list[Json] = []
    for base in probe.bases:
        try:
            with environment("AZURE_SPEECH_API_BASE", probe.environment_api_base):
                url = config.get_complete_url(
                    api_base=base,
                    api_key=None,
                    model=probe.model,
                    optional_params=dict(probe.optional_params),
                    litellm_params=empty,
                )
        except BaseLLMException as failure:
            urls.append({"ok": False, "status": failure.status_code})
        else:
            urls.append({"ok": True, "url": url})
    return urls


def native_config(config: ConfigView, probe: Probe) -> Json:
    supported: list[Json] = list(config.get_supported_openai_params(model=probe.model))
    mapped = config.map_openai_params(
        non_default_params=dict(probe.non_default_params),
        optional_params=dict(probe.existing_params),
        model=probe.model,
        drop_params=False,
    )
    error = config.get_error_class("boom", 502, {"x-request-id": "req-1"})
    return {
        "supported": supported,
        "mapped": mapped,
        "headers": checked_headers(config, probe),
        "urls": checked_urls(config, probe),
        "error_status": error.status_code,
    }


def native_response(config: BaseAudioTranscriptionConfig, probe: Probe) -> Json:
    results: list[Json] = []
    for item in probe.bodies:
        raw = httpx.Response(item.status, headers=dict(probe.headers), content=item.body.encode())
        try:
            response = config.transform_audio_transcription_response(raw)
        except BaseLLMException as failure:
            results.append({"ok": False, "status": failure.status_code})
        else:
            hidden = PRIVATE.validate_python(response.__pydantic_private__)["_hidden_params"]
            results.append({"ok": True, "text": response.text, "hidden": hidden})
    return {"results": results}


def native_request(config: BaseAudioTranscriptionConfig, probe: Probe) -> Json:
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
    raise TypeError("the native request data is not bytes")
