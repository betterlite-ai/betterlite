import asyncio
import base64
from collections.abc import Coroutine
from typing import Final, Protocol, TypedDict

import httpx
import litellm
import openai
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.base_llm.audio_transcription.transformation import (
    BaseAudioTranscriptionConfig,
)
from litellm.llms.openai.transcriptions.handler import OpenAIAudioTranscription
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override

from .probe import PROVIDERS, Answer, Probe

litellm.suppress_debug_info = True

FILE: Final = ("a.wav", b"abc", "audio/wav")
FILE_VIEW: Final[Json] = {"filename": "a.wav", "content_type": "audio/wav", "content": "YWJj"}
DUMP: Final = TypeAdapter(dict[str, Json])


class Private(TypedDict):
    _hidden_params: dict[str, Json]


PRIVATE: Final = TypeAdapter(Private)


class Sent(TypedDict):
    complete_input_dict: dict[str, Json | tuple[str, bytes, str]]


class PreCall(Sent):
    api_base: str


class Upstream:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        return httpx.Response(
            self.answer.status, headers=self.answer.headers, content=self.answer.body.encode()
        )


class Recorder(LiteLLMLoggingObj):
    def __init__(self) -> None:
        self.model_call_details: dict[str, Json] = {}
        self.pre: Json = None
        self.post: Json = None

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
                "api_base": str(additional_args["api_base"]),
                "data": data_view(additional_args),
            }

    @override
    def post_call(
        self,
        original_response: Json,
        input: Json = None,
        api_key: str | None = None,
        additional_args: Sent | None = None,
    ) -> None:
        match original_response:
            case dict() as fields:
                self.post = {
                    "input_names_file": FILE[0] in str(input),
                    "data": data_view(additional_args),
                    "original_response": {
                        name: value for name, value in fields.items() if value is not None
                    },
                    "error_text": False,
                }
            case None | bool() | int() | float() | str() | list():
                self.post = {
                    "input_names_file": None,
                    "data": None,
                    "original_response": None,
                    "error_text": True,
                }


def data_view(additional_args: Sent | None) -> Json:
    if additional_args is None:
        return None
    view: dict[str, Json] = {}
    for name, value in additional_args["complete_input_dict"].items():
        match value:
            case tuple():
                view[name] = FILE_VIEW
            case None | bool() | int() | float() | str() | list() | dict():
                view[name] = value
    return view


def multipart_view(sent: httpx.Request) -> list[Json]:
    boundary = sent.headers["content-type"].partition("boundary=")[2]
    parts: list[Json] = []
    for section in sent.content.split(b"--" + boundary.encode())[1:-1]:
        head, _separator, payload = section.removeprefix(b"\r\n").partition(b"\r\n\r\n")
        headers: dict[str, str] = {}
        for line in head.decode().split("\r\n"):
            name, _colon, value = line.partition(": ")
            headers[name.lower()] = value
        attributes: dict[str, str] = {}
        for item in headers["content-disposition"].split("; ")[1:]:
            key, _equals, value = item.partition("=")
            attributes[key] = value.strip('"')
        parts.append(
            {
                "name": attributes["name"],
                "filename": attributes.get("filename"),
                "content_type": headers.get("content-type"),
                "body": base64.b64encode(payload.removesuffix(b"\r\n")).decode(),
            }
        )
    return parts


class Handler(Protocol):
    def async_audio_transcriptions(
        self,
        _audio_file: FileTypes,
        _data: dict[str, Json | tuple[str, bytes, str]],
        _model_response: TranscriptionResponse,
        _timeout: float,
        _logging_obj: LiteLLMLoggingObj,
        _api_key: str | None,
        _api_base: str | None,
        /,
        *,
        max_retries: int,
    ) -> Coroutine[None, None, TranscriptionResponse]: ...

    def audio_transcriptions(
        self,
        _model: str,
        _audio_file: FileTypes,
        _optional_params: dict[str, Json],
        _litellm_params: dict[str, Json],
        _model_response: TranscriptionResponse,
        _timeout: float,
        _max_retries: int,
        _logging_obj: LiteLLMLoggingObj,
        _api_key: str | None,
        _api_base: str | None,
        /,
        *,
        atranscription: bool,
        provider_config: BaseAudioTranscriptionConfig | None,
    ) -> TranscriptionResponse | Coroutine[None, None, TranscriptionResponse]: ...


def invoke(handler: Handler, probe: Probe, recorder: Recorder) -> TranscriptionResponse:
    if probe.data_model is not None or probe.data_model_null:
        data: dict[str, Json | tuple[str, bytes, str]] = {
            "model": None if probe.data_model_null else probe.data_model,
            "file": FILE,
        }
        data.update(probe.optional_params)
        return asyncio.run(
            handler.async_audio_transcriptions(
                FILE,
                data,
                TranscriptionResponse(),
                30.0,
                recorder,
                probe.api_key,
                probe.api_base,
                max_retries=2,
            )
        )
    options = dict(probe.optional_params)
    no_litellm_params: dict[str, Json] = {}
    outcome = handler.audio_transcriptions(
        probe.model,
        FILE,
        options,
        no_litellm_params,
        TranscriptionResponse(),
        30.0,
        2,
        recorder,
        probe.api_key,
        probe.api_base,
        atranscription=probe.atranscription,
        provider_config=None if probe.provider is None else PROVIDERS[probe.provider],
    )
    if asyncio.iscoroutine(outcome):
        return asyncio.run(outcome)
    return outcome


def client_view(probe: Probe) -> Json:
    asynchronous = probe.atranscription or probe.data_model is not None or probe.data_model_null
    client = (
        openai.AsyncOpenAI(api_key=probe.api_key, base_url=probe.api_base, max_retries=2)
        if asynchronous
        else openai.OpenAI(api_key=probe.api_key, base_url=probe.api_base, max_retries=2)
    )
    return {
        "kind": "async" if asynchronous else "sync",
        "base_url": str(client.base_url),
        "max_retries": client.max_retries,
    }


def result_view(response: TranscriptionResponse) -> Json:
    private: dict[str, Json] | None = response.__pydantic_private__
    hidden = PRIVATE.validate_python(private)["_hidden_params"]
    return {
        "model": hidden.get("model"),
        "provider": hidden.get("custom_llm_provider"),
        "duration_recorded": "audio_transcription_duration" in hidden,
        "duration": hidden.get("audio_transcription_duration"),
        "response": response_document(response),
        "headers": hidden.get("headers"),
        "additional_headers": hidden.get("additional_headers"),
    }


def response_document(response: TranscriptionResponse) -> Json:
    dumped = DUMP.validate_json(response.model_dump_json())
    fields: dict[str, Json] = {
        name: value
        for name, value in dumped.items()
        if name not in ("text", "usage") and value is not None
    }
    return {"text": dumped["text"], "usage": dumped["usage"], "fields": fields}


def request_view(upstream: Upstream) -> Json:
    if not upstream.seen:
        return None
    sent = upstream.seen[0]
    present = {
        name: sent.headers[name] for name in ("authorization", "accept") if name in sent.headers
    }
    return {
        "url": str(sent.url),
        "authorization": present.get("authorization"),
        "accept": present.get("accept"),
        "parts": multipart_view(sent),
    }


def outcome(probe: Probe, recorder: Recorder) -> tuple[Json, Json]:
    try:
        return result_view(invoke(OpenAIAudioTranscription(), probe, recorder)), None
    except openai.APIStatusError as failure:
        status: Json = {
            "status": failure.status_code,
            "type": failure.type,
            "param": failure.param,
            "code": failure.code,
        }
        return None, status
    except (openai.OpenAIError, httpx.HTTPError, ValueError, RuntimeError, TypeError):
        return None, {"kind": "error"}


def exchange(probe: Probe) -> Json:
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    upstream = Upstream(probe.reply)
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(upstream.handle))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(upstream.handle))
    recorder = Recorder()
    result, error = outcome(probe, recorder)
    return {
        "client": client_view(probe),
        "request": request_view(upstream),
        "result": result,
        "error": error,
        "pre_call": recorder.pre,
        "post_call": recorder.post,
        "response_headers": recorder.model_call_details.get("response_headers"),
    }
