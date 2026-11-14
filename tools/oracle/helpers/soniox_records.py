import os
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.soniox.audio_transcription.transformation import (
    SonioxAudioTranscriptionConfig,
)
from litellm.types.llms.openai import (
    AllMessageValues,
    OpenAIAudioTranscriptionOptionalParams as TranscriptionParam,
)
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue, TypeAdapter

Object = dict[str, JsonValue]
EMPTY: Final[Object] = {}
EMPTY_HEADERS: Final[dict[str, str]] = {}
MODEL: Final[str] = "soniox/stt-async-v4"
ENVIRONMENT_NAMES: Final[tuple[str, str]] = ("SONIOX_API_KEY", "SONIOX_API_BASE")

litellm.suppress_debug_info = True

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Config(Protocol):
    def get_supported_openai_params(self, model: str) -> list[TranscriptionParam]: ...

    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        /,
        *,
        drop_params: bool,
    ) -> Object: ...

    def get_error_class(
        self,
        error_message: str,
        status_code: int,
        headers: dict[str, str],
    ) -> BaseLLMException: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: Object,
        litellm_params: Object,
        /,
        *,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
        /,
        *,
        stream: bool | None = None,
    ) -> str: ...

    def _build_response_from_payload(
        self,
        payload: Object,
        model_response: TranscriptionResponse | None = None,
        response_format: str | None = None,
    ) -> TranscriptionResponse: ...


# LiteLLM leaves the mapped params and the payload bare; this typed view reaches its config
CONFIG: Config = SonioxAudioTranscriptionConfig()


class Call(TypedDict, total=False):
    function: Literal[
        "supported_params",
        "map_params",
        "validate_environment",
        "complete_url",
        "error",
        "response",
    ]
    non_default_params: Object
    optional_params: Object
    headers: dict[str, str]
    api_key: str | None
    environment: dict[str, str]
    message: str
    status_code: int
    payload: Object
    response_format: str | None


class Input(TypedDict):
    calls: list[Call]


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def supported_params() -> JsonValue:
    params: list[JsonValue] = []
    for parameter in CONFIG.get_supported_openai_params(MODEL):
        params.append(parameter)
    return {"ok": True, "params": params}


def map_params(call: Call) -> JsonValue:
    params = CONFIG.map_openai_params(
        call["non_default_params"],
        call["optional_params"],
        MODEL,
        drop_params=False,
    )
    return {"ok": True, "params": params, "optional_params": params}


def validate_environment(call: Call) -> JsonValue:
    apply_environment(call["environment"])
    try:
        merged = CONFIG.validate_environment(
            call["headers"],
            MODEL,
            [],
            EMPTY,
            EMPTY,
            api_key=call["api_key"],
            api_base=None,
        )
    except BaseLLMException as error:
        return {
            "ok": False,
            "error": {"status": error.status_code, "message": error.message},
        }
    return {"ok": True, "headers": JSON.validate_python(merged)}


def complete_url(call: Call) -> JsonValue:
    apply_environment(call["environment"])
    url = CONFIG.get_complete_url(call["api_base"], None, MODEL, EMPTY, EMPTY)
    return {"ok": True, "url": url}


def error(call: Call) -> JsonValue:
    built = CONFIG.get_error_class(call["message"], call["status_code"], EMPTY_HEADERS)
    return {"status": built.status_code, "message": built.message}


def response(call: Call) -> JsonValue:
    try:
        built = CONFIG._build_response_from_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/transformation.py#L194
            call["payload"],
            None,
            response_format=call.get("response_format"),
        )
    except (TypeError, ValueError):
        return {"ok": False, "error": {"status": None}}
    return {"ok": True, "response": JSON.validate_json(built.model_dump_json())}


def answer(call: Call) -> JsonValue:
    function = call["function"]
    if function == "map_params":
        return map_params(call)
    if function == "validate_environment":
        return validate_environment(call)
    if function == "complete_url":
        return complete_url(call)
    if function == "error":
        return error(call)
    if function == "response":
        return response(call)
    return supported_params()


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
