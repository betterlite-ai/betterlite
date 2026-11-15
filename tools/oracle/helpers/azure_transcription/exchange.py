import asyncio
from collections.abc import Coroutine
from typing import NotRequired, Protocol, TypedDict

import httpx
import litellm
import openai
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.azure.audio_transcriptions import AzureAudioTranscription
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import FileTypes, TranscriptionResponse
from pydantic import JsonValue as Json
from typing_extensions import override

from openai_transcription.exchange import PRIVATE, Upstream, data_view, multipart_view

from .probe import FILE, Probe

litellm.suppress_debug_info = True


class Handler(Protocol):
    def audio_transcriptions(
        self,
        _model: str,
        _audio_file: FileTypes,
        _optional_params: dict[str, Json],
        _logging_obj: LiteLLMLoggingObj,
        _model_response: TranscriptionResponse,
        _timeout: float,
        _max_retries: int,
        _api_key: str | None,
        _api_base: str | None,
        _api_version: str | None,
        _client: openai.OpenAI | None,
        _azure_ad_token: str | None,
        /,
        *,
        atranscription: bool,
        litellm_params: dict[str, Json],
    ) -> TranscriptionResponse | Coroutine[None, None, TranscriptionResponse]: ...


class Sent(TypedDict):
    complete_input_dict: dict[str, Json | tuple[str, bytes, str]]


class PreCall(Sent):
    api_base: str
    headers: dict[str, str]


class PostArgs(Sent):
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
                    "input_names_file": FILE[0] in str(input),
                    "authorization": post_authorization(additional_args),
                    "api_base": post_api_base(additional_args),
                    "original_response": fields,
                    "data": data_view(additional_args),
                    "error_text": False,
                }
            case None | bool() | int() | float() | str() | list():
                self.document = None
                self.post = {
                    "input_names_file": FILE[0] in str(input),
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


def supplied_client(probe: Probe) -> openai.OpenAI | None:
    if probe.client_kind != "openai":
        return None
    return openai.OpenAI(api_key=probe.api_key, base_url=probe.api_base, max_retries=2)


def invoke(handler: Handler, probe: Probe, recorder: Recorder) -> TranscriptionResponse:
    no_litellm_params: dict[str, Json] = {}
    outcome = handler.audio_transcriptions(
        probe.model,
        FILE,
        dict(probe.optional_params),
        recorder,
        TranscriptionResponse(),
        30.0,
        2 if probe.max_retries is None else probe.max_retries,
        probe.api_key,
        probe.api_base,
        probe.api_version,
        supplied_client(probe),
        probe.azure_ad_token,
        atranscription=probe.atranscription,
        litellm_params=no_litellm_params,
    )
    if asyncio.iscoroutine(outcome):
        return asyncio.run(outcome)
    return outcome


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
    upstream = Upstream(probe.reply)
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
