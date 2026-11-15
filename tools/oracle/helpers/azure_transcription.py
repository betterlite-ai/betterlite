import asyncio
import base64
import os
from collections.abc import Coroutine, Iterator, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
import openai
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.azure.audio_transcriptions import AzureAudioTranscription
from litellm.llms.base_llm.audio_transcription.transformation import (
    AudioTranscriptionRequestData,
    BaseAudioTranscriptionConfig,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import FileTypes, LlmProviders, TranscriptionResponse
from litellm.utils import ProviderConfigManager
from openai_transcription.exchange import PRIVATE, Upstream, data_view, multipart_view
from openai_transcription.probe import Answer
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import NotRequired, override

litellm.suppress_debug_info = True

FILE: Final = ("a.wav", b"abc", "audio/wav")
UNKNOWN_MODEL: Final = "speech/azure-stt"


class NoMessages(list[AllMessageValues]):
    pass


class Config(Protocol):
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
        headers: dict[str, Json],
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        api_key: str | None = None,
        api_base: str | None = None,
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

    def transform_audio_transcription_request(
        self,
        model: str,
        audio_file: FileTypes,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> AudioTranscriptionRequestData: ...

    def transform_audio_transcription_response(
        self, raw_response: httpx.Response
    ) -> TranscriptionResponse: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException: ...


class Handler(Protocol):
    def audio_transcriptions(
        self,
        *,
        model: str,
        audio_file: FileTypes,
        optional_params: dict[str, Json],
        logging_obj: LiteLLMLoggingObj,
        model_response: TranscriptionResponse,
        timeout: float,
        max_retries: int,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None,
        azure_ad_token: str | None,
        atranscription: bool,
        litellm_params: dict[str, Json],
        client: openai.OpenAI | None = None,
    ) -> TranscriptionResponse | Coroutine[object, object, TranscriptionResponse]: ...


@dataclass
class Reply:
    status: int = 200
    headers: dict[str, str] = field(
        default_factory=lambda: {"content-type": "application/json"}
    )
    body: str = '{"text":"hi"}'


@dataclass
class Body:
    status: int = 200
    body: str = ""


@dataclass
class Probe:
    model: str
    optional_params: dict[str, Json] = field(default_factory=dict)
    non_default_params: dict[str, Json] = field(default_factory=dict)
    existing_params: dict[str, Json] = field(default_factory=dict)
    headers: dict[str, str] = field(default_factory=dict)
    api_key: str | None = None
    environment_api_key: str | None = None
    api_base: str | None = None
    environment_api_base: str | None = None
    bases: list[str | None] = field(default_factory=list)
    atranscription: bool = False
    api_version: str | None = None
    azure_ad_token: str | None = None
    max_retries: int | None = None
    client_kind: str | None = None
    reply: Reply = field(default_factory=Reply)
    bodies: list[Body] = field(default_factory=list)
    audio: str = ""


PROBE: Final = TypeAdapter(Probe)


class Input(TypedDict):
    function: Literal["exchange", "native_config", "native_response", "native_request"]
    probes: list[dict[str, Json]]


class Sent(TypedDict):
    complete_input_dict: dict[str, Json | tuple[str, bytes, str]]


class PreCall(Sent):
    api_base: str
    headers: dict[str, str]


class PostArgs(TypedDict):
    complete_input_dict: dict[str, Json | tuple[str, bytes, str]]
    headers: NotRequired[dict[str, str]]
    api_base: NotRequired[str]
    atranscription: NotRequired[bool]


class Recorder(LiteLLMLoggingObj):
    def __init__(self) -> None:
        self.pre: Json = None
        self.post: Json = None
        self.document: Json = None

    @override
    def pre_call(
        self,
        input: Json,
        api_key: str | None,
        model: str | None = None,
        additional_args: PreCall | None = None,
    ) -> None:
        if additional_args is not None:
            self.pre = {
                "input_names_file": "audio_file_" in str(input),
                "authorization": additional_args["headers"]["Authorization"],
                "api_base": str(additional_args["api_base"]),
                "data": data_view(additional_args),
            }

    @override
    def post_call(
        self,
        original_response: Json,
        input: Json = None,
        api_key: str | None = None,
        additional_args: PostArgs | None = None,
    ) -> None:
        match original_response:
            case dict() as fields:
                self.document = fields
                self.post = {
                    "input_names_file": "a.wav" in str(input),
                    "authorization": post_authorization(additional_args),
                    "api_base": post_api_base(additional_args),
                    "original_response": fields,
                    "data": data_view(additional_args),
                    "error_text": False,
                }
            case None | bool() | int() | float() | str() | list():
                self.document = None
                self.post = {
                    "input_names_file": "a.wav" in str(input),
                    "authorization": None,
                    "api_base": None,
                    "original_response": None,
                    "data": None,
                    "error_text": True,
                }


def post_authorization(additional_args: PostArgs | None) -> Json:
    if additional_args is None:
        return None
    headers = additional_args.get("headers")
    if headers is None:
        return None
    return headers.get("Authorization")


def post_api_base(additional_args: PostArgs | None) -> Json:
    if additional_args is None:
        return None
    api_base = additional_args.get("api_base")
    if api_base is None:
        return None
    return str(api_base)


def native_audio_config() -> Config:
    config: BaseAudioTranscriptionConfig | None = (
        ProviderConfigManager.get_provider_audio_transcription_config(
            model=UNKNOWN_MODEL, provider=LlmProviders.AZURE
        )
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


def native_config(config: Config, probe: Probe) -> Json:
    empty: dict[str, Json] = {}
    supported: list[Json] = list(config.get_supported_openai_params(model=probe.model))
    mapped = config.map_openai_params(
        non_default_params=dict(probe.non_default_params),
        optional_params=dict(probe.existing_params),
        model=probe.model,
        drop_params=False,
    )
    headers: Json
    try:
        with environment("AZURE_SPEECH_API_KEY", probe.environment_api_key):
            validated = config.validate_environment(
                dict(probe.headers), probe.model, NoMessages(), empty, empty, probe.api_key
            )
    except BaseLLMException as failure:
        headers = {"ok": False, "status": failure.status_code}
    else:
        headers = {"ok": True, "value": validated}
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
    failure = config.get_error_class("boom", 502, {"x-request-id": "req-1"})
    return {
        "supported": supported,
        "mapped": mapped,
        "headers": headers,
        "urls": urls,
        "error_status": failure.status_code,
    }


def native_response(config: Config, probe: Probe) -> Json:
    results: list[Json] = []
    for item in probe.bodies:
        raw = httpx.Response(
            item.status, headers=dict(probe.headers), content=item.body.encode()
        )
        try:
            response = config.transform_audio_transcription_response(raw)
        except BaseLLMException as failure:
            results.append({"ok": False, "status": failure.status_code})
        else:
            hidden = PRIVATE.validate_python(response.__pydantic_private__)["_hidden_params"]
            results.append({"ok": True, "text": response.text, "hidden": hidden})
    return {"results": results}


def native_request(config: Config, probe: Probe) -> Json:
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


def supplied_client(probe: Probe) -> openai.OpenAI | None:
    if probe.client_kind != "openai":
        return None
    return openai.OpenAI(api_key=probe.api_key, base_url=probe.api_base, max_retries=2)


def invoke(handler: Handler, probe: Probe, recorder: Recorder) -> TranscriptionResponse:
    empty: dict[str, Json] = {}
    outcome = handler.audio_transcriptions(
        model=probe.model,
        audio_file=FILE,
        optional_params=dict(probe.optional_params),
        logging_obj=recorder,
        model_response=TranscriptionResponse(),
        timeout=30.0,
        max_retries=2 if probe.max_retries is None else probe.max_retries,
        api_key=probe.api_key,
        api_base=probe.api_base,
        api_version=probe.api_version,
        azure_ad_token=probe.azure_ad_token,
        atranscription=probe.atranscription,
        litellm_params=empty,
        client=supplied_client(probe),
    )
    if isinstance(outcome, TranscriptionResponse):
        return outcome
    return asyncio.run(outcome)


def result_view(response: TranscriptionResponse, recorder: Recorder) -> Json:
    private: dict[str, Json] | None = response.__pydantic_private__
    hidden = PRIVATE.validate_python(private)["_hidden_params"]
    return {
        "model": hidden.get("model"),
        "provider": hidden.get("custom_llm_provider"),
        "duration_recorded": "audio_transcription_duration" in hidden,
        "duration": hidden.get("audio_transcription_duration"),
        "text": response.text,
        "additional_headers": hidden.get("additional_headers"),
        "original_response": recorder.document,
    }


def request_view(upstream: Upstream) -> Json:
    if not upstream.seen:
        return None
    sent = upstream.seen[0]
    return {
        "url": str(sent.url),
        "api_key": sent.headers.get("api-key"),
        "authorization": sent.headers.get("authorization"),
        "accept": sent.headers.get("accept"),
        "parts": multipart_view(sent),
    }


def outcome(probe: Probe, recorder: Recorder) -> tuple[Json, Json]:
    try:
        response = invoke(AzureAudioTranscription(), probe, recorder)
    except openai.APIStatusError as failure:
        status: Json = {
            "status": failure.status_code,
            "type": failure.type,
            "param": failure.param,
            "code": failure.code,
        }
        return None, status
    except (
        BaseLLMException,
        openai.OpenAIError,
        httpx.HTTPError,
        ValueError,
        RuntimeError,
        TypeError,
    ):
        return None, {"kind": "error"}
    return result_view(response, recorder), None


def exchange(probe: Probe) -> Json:
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    upstream = Upstream(
        Answer(
            status=probe.reply.status,
            headers=dict(probe.reply.headers),
            body=probe.reply.body,
        )
    )
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(upstream.handle))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(upstream.handle))
    recorder = Recorder()
    result, error = outcome(probe, recorder)
    return {
        "request": request_view(upstream),
        "result": result,
        "error": error,
        "pre_call": recorder.pre,
        "post_call": recorder.post,
    }


def run(input_value: Input) -> Json:
    values: list[Json] = []
    for raw in input_value["probes"]:
        probe = PROBE.validate_python(raw)
        match input_value["function"]:
            case "exchange":
                values.append(exchange(probe))
            case "native_config":
                values.append(native_config(native_audio_config(), probe))
            case "native_response":
                values.append(native_response(native_audio_config(), probe))
            case "native_request":
                values.append(native_request(native_audio_config(), probe))
    return values
