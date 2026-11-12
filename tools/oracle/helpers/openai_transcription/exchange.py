import asyncio
import base64
from collections.abc import Coroutine
from typing import Final, Protocol, TypedDict, Unpack, cast

import httpx
import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.base_llm.audio_transcription.transformation import (
    BaseAudioTranscriptionConfig,
)
from litellm.llms.openai.transcriptions.handler import OpenAIAudioTranscription
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue as Json

from .probe import PROVIDERS, Answer, Probe

litellm.suppress_debug_info = True

FILE: Final = ("a.wav", b"abc", "audio/wav")
FILE_VIEW: Final[Json] = {"filename": "a.wav", "content_type": "audio/wav", "content": "YWJj"}


class AdditionalArgs(TypedDict):
    complete_input_dict: dict[str, Json]


class PreCallArgs(AdditionalArgs):
    api_base: str
    atranscription: bool


class AsyncArgs(TypedDict):
    audio_file: tuple[str, bytes, str]
    data: dict[str, Json]
    model_response: TranscriptionResponse
    timeout: float
    logging_obj: LiteLLMLoggingObj
    api_key: str | None
    api_base: str | None
    max_retries: int


class SyncArgs(TypedDict):
    model: str
    audio_file: tuple[str, bytes, str]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    model_response: TranscriptionResponse
    timeout: float
    max_retries: int
    logging_obj: LiteLLMLoggingObj
    api_key: str | None
    api_base: str | None
    atranscription: bool
    provider_config: BaseAudioTranscriptionConfig | None


class HandlerView(Protocol):
    def async_audio_transcriptions(
        self, **kwargs: Unpack[AsyncArgs]
    ) -> Coroutine[None, None, TranscriptionResponse]: ...

    def audio_transcriptions(
        self, **kwargs: Unpack[SyncArgs]
    ) -> TranscriptionResponse | Coroutine[None, None, TranscriptionResponse]: ...


class Upstream:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        return httpx.Response(
            self.answer.status, headers=self.answer.headers, content=self.answer.body.encode()
        )


class Recorder:
    def __init__(self) -> None:
        self.model_call_details: dict[str, Json] = {}
        self.pre: Json = None
        self.post: Json = None

    def pre_call(self, additional_args: PreCallArgs, **_ignored: Json) -> None:
        self.pre = {
            "api_base": str(additional_args["api_base"]),
            "data": data_view(additional_args),
        }

    def post_call(
        self,
        original_response: Json,
        additional_args: AdditionalArgs | None = None,
        **kwargs: Json,
    ) -> None:
        match original_response:
            case dict() as fields:
                self.post = {
                    "input_names_file": FILE[0] in str(kwargs["input"]),
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


def data_view(additional_args: AdditionalArgs | None) -> Json:
    if additional_args is None:
        return None
    fields = additional_args["complete_input_dict"]
    return {name: FILE_VIEW if name == "file" else value for name, value in fields.items()}


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


def invoke(probe: Probe, recorder: Recorder) -> TranscriptionResponse:
    handler = cast(HandlerView, OpenAIAudioTranscription())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/transcriptions/handler.py#L56-L171
    logger = cast(LiteLLMLoggingObj, recorder)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/transcriptions/handler.py#L81-L89
    if probe.data_model is not None or probe.data_model_null:
        data: dict[str, Json] = {"model": None if probe.data_model_null else probe.data_model}
        data["file"] = cast(Json, FILE)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/transcriptions/handler.py#L107
        data.update(probe.optional_params)
        return asyncio.run(
            handler.async_audio_transcriptions(
                audio_file=FILE,
                data=data,
                model_response=TranscriptionResponse(),
                timeout=30.0,
                logging_obj=logger,
                api_key=probe.api_key,
                api_base=probe.api_base,
                max_retries=2,
            )
        )
    outcome = handler.audio_transcriptions(
        model=probe.model,
        audio_file=FILE,
        optional_params=dict(probe.optional_params),
        litellm_params={},
        model_response=TranscriptionResponse(),
        timeout=30.0,
        max_retries=2,
        logging_obj=logger,
        api_key=probe.api_key,
        api_base=probe.api_base,
        atranscription=probe.atranscription,
        provider_config=None if probe.provider is None else PROVIDERS[probe.provider],
    )
    if probe.atranscription:
        return asyncio.run(cast("Coroutine[None, None, TranscriptionResponse]", outcome))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/transcriptions/handler.py#L82-L96
    return cast(TranscriptionResponse, outcome)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/transcriptions/handler.py#L82-L96


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
    dumped = cast("dict[str, Json]", response.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2748
    hidden = cast("dict[str, Json]", response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2748
    return {
        "model": hidden.get("model"),
        "provider": hidden.get("custom_llm_provider"),
        "duration_recorded": "audio_transcription_duration" in hidden,
        "duration": hidden.get("audio_transcription_duration"),
        "response": response_document(dumped),
        "headers": hidden.get("headers"),
        "additional_headers": hidden.get("additional_headers"),
    }


def response_document(dumped: dict[str, Json]) -> Json:
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
        return result_view(invoke(probe, recorder)), None
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
    litellm.in_memory_llm_clients_cache.flush_cache()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L260
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
