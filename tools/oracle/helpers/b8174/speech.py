from typing import Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.minimax.text_to_speech.transformation import (
    MinimaxTextToSpeechConfig,
)
from pydantic import JsonValue, TypeAdapter

from .steps import Object, Recorder, Step, install_environment, object_or_empty


class SpeechRequestData(TypedDict, total=False):
    dict_body: Object


class BinaryReply(Protocol):
    response: httpx.Response

    @property
    def content(self) -> bytes: ...


class SpeechConfig(Protocol):
    def map_openai_params(
        self,
        _model: str,
        _optional_params: Object,
        _voice: str | dict[str, JsonValue] | None,
        /,
        *,
        drop_params: bool,
    ) -> tuple[str | None, Object]: ...

    # ast-grep-ignore: no-long-names
    def transform_text_to_speech_request(
        self,
        _model: str,
        _input: str,
        _voice: str | None,
        _optional_params: Object,
        _litellm_params: Object,
        _headers: dict[str, str],
        /,
    ) -> SpeechRequestData: ...

    def validate_environment(
        self,
        _headers: dict[str, str],
        _model: str,
        /,
        *,
        api_key: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self,
        _model: str,
        _api_base: str | None,
        _litellm_params: Object,
        /,
    ) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_text_to_speech_response(
        self,
        _model: str,
        _raw_response: httpx.Response,
        _logging_obj: LiteLLMLoggingObj,
        /,
    ) -> BinaryReply: ...


class SpeechFactory(Protocol):
    def __call__(self) -> SpeechConfig: ...


SPEECH: SpeechFactory = MinimaxTextToSpeechConfig
NO_DROP = False


def speech_params(step: Step) -> JsonValue:
    mapped_voice, mapped = SPEECH().map_openai_params(
        "",
        object_or_empty(step.get("params")),
        step.get("voice"),
        drop_params=NO_DROP,
    )
    answer: Object = {"voice": mapped_voice, "params": mapped}
    return TypeAdapter(Object).validate_python(answer)


def speech_body(step: Step) -> JsonValue:
    model = step.get("model")
    text = step.get("input")
    if model is None or text is None:
        raise ValueError("the speech body step needs a model and an input")
    mapped_voice, mapped = SPEECH().map_openai_params(
        model,
        object_or_empty(step.get("params")),
        step.get("voice"),
        drop_params=NO_DROP,
    )
    request = SPEECH().transform_text_to_speech_request(
        model,
        text,
        mapped_voice,
        mapped,
        {},
        {},
    )
    body = request.get("dict_body")
    if body is None:
        raise ValueError("the MiniMax request carries no body")
    return body


def speech_url(step: Step) -> JsonValue:
    install_environment({"MINIMAX_API_BASE": step.get("env_base")})
    return SPEECH().get_complete_url("", step.get("api_base"), {})


def speech_headers(step: Step) -> JsonValue:
    install_environment({"MINIMAX_API_KEY": step.get("env_key")})
    litellm.api_key = step.get("global_key")
    headers = SPEECH().validate_environment({}, "", api_key=step.get("api_key"))
    return TypeAdapter(Object).validate_python(headers)


def speech_reply(step: Step) -> JsonValue:
    status = step.get("status")
    text = step.get("body")
    if status is None or text is None:
        raise ValueError("the speech reply step needs a status and a body")
    response = httpx.Response(
        status,
        content=text.encode(),
        headers=step.get("headers"),
        request=httpx.Request("POST", "https://host"),
    )
    result = SPEECH().transform_text_to_speech_response("", response, Recorder())
    reply_headers: Object = TypeAdapter(Object).validate_python(result.response.headers)
    answer: Object = {
        "status": result.response.status_code,
        "content_type": reply_headers.get("content-type"),
        "headers": reply_headers,
        "body": result.content.hex(),
    }
    return TypeAdapter(Object).validate_python(answer)
