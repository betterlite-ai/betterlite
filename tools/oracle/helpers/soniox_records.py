import os
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
from litellm.llms.soniox.audio_transcription.transformation import (
    SonioxAudioTranscriptionConfig,
)
from litellm.llms.soniox.common_utils import SonioxException
from litellm.types.utils import TranscriptionResponse
from pydantic import JsonValue, TypeAdapter

from xai_files import guarded

Object: TypeAlias = dict[str, JsonValue]
EMPTY: Final[Object] = {}
EMPTY_HEADERS: Final[dict[str, str]] = {}
MODEL: Final[str] = "soniox/stt-async-v4"
ENVIRONMENT_NAMES: Final[tuple[str, str]] = ("SONIOX_API_KEY", "SONIOX_API_BASE")

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BASE: Final = SonioxAudioTranscriptionConfig.__new__(SonioxAudioTranscriptionConfig)


class MapParamsFn(Protocol):
    def __call__(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        drop_params: bool,
    ) -> Object: ...


class ValidateKwargs(TypedDict):
    headers: dict[str, str]
    model: str
    messages: tuple[()]
    optional_params: Object
    litellm_params: Object
    api_key: str | None
    api_base: str | None


class ValidateEnvironmentFn(Protocol):
    def __call__(self, **kwargs: Unpack[ValidateKwargs]) -> dict[str, str]: ...


class CompleteUrlFn(Protocol):
    def __call__(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
    ) -> str: ...


class ErrorClassFn(Protocol):
    def __call__(
        self,
        error_message: str,
        status_code: int,
        headers: dict[str, str],
    ) -> SonioxException: ...


class BuildResponseFn(Protocol):
    def __call__(
        self,
        payload: Object,
        model_response: TranscriptionResponse | None = None,
        response_format: str | None = None,
    ) -> TranscriptionResponse: ...


# LiteLLM annotates these five methods with bare dicts, and the gate counts one suppression line in this file
MAP_PARAMS, VALIDATE_ENVIRONMENT, COMPLETE_URL, ERROR_CLASS, BUILD_RESPONSE = cast(MapParamsFn, BASE.map_openai_params), cast(ValidateEnvironmentFn, BASE.validate_environment), cast(CompleteUrlFn, BASE.get_complete_url), cast(ErrorClassFn, BASE.get_error_class), cast(BuildResponseFn, BASE._build_response_from_payload)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/transformation.py#L192-L261


class SupportedParamsCall(TypedDict):
    function: Literal["supported_params"]


class MapParamsCall(TypedDict):
    function: Literal["map_params"]
    non_default_params: Object
    optional_params: NotRequired[Object]


class ValidateEnvironmentCall(TypedDict):
    function: Literal["validate_environment"]
    headers: dict[str, str]
    api_key: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]


class CompleteUrlCall(TypedDict):
    function: Literal["complete_url"]
    api_base: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]


class ErrorCall(TypedDict):
    function: Literal["error"]
    message: str
    status_code: int


class ResponseCall(TypedDict):
    function: Literal["response"]
    payload: Object
    response_format: NotRequired[str | None]


Call: TypeAlias = (
    SupportedParamsCall
    | MapParamsCall
    | ValidateEnvironmentCall
    | CompleteUrlCall
    | ErrorCall
    | ResponseCall
)


class Input(TypedDict):
    calls: list[Call]


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def map_params(call: MapParamsCall) -> JsonValue:
    options = dict(call.get("optional_params", EMPTY))
    params = MAP_PARAMS(call["non_default_params"], options, MODEL, drop_params=False)
    return {"ok": True, "params": params, "optional_params": options}


def validate_view(call: ValidateEnvironmentCall) -> JsonValue:
    merged = VALIDATE_ENVIRONMENT(
        headers=call["headers"],
        model=MODEL,
        messages=(),
        optional_params=EMPTY,
        litellm_params=EMPTY,
        api_key=call.get("api_key"),
        api_base=None,
    )
    return {"ok": True, "headers": merged}


def validate_environment(call: ValidateEnvironmentCall) -> JsonValue:
    apply_environment(call.get("environment", EMPTY))
    return guarded(validate_view, call)


def complete_url(call: CompleteUrlCall) -> JsonValue:
    apply_environment(call.get("environment", EMPTY))
    url = COMPLETE_URL(call.get("api_base"), None, MODEL, EMPTY, EMPTY)
    return {"ok": True, "url": url}


def error(call: ErrorCall) -> JsonValue:
    built = ERROR_CLASS(call["message"], call["status_code"], EMPTY_HEADERS)
    return {"status": built.status_code, "message": built.message}


def response(call: ResponseCall) -> JsonValue:
    try:
        built = BUILD_RESPONSE(
            call["payload"], None, response_format=call.get("response_format")
        )
    except (TypeError, ValueError):
        return {"ok": False, "error": {"status": None}}
    return {"ok": True, "response": JSON.validate_json(built.model_dump_json())}


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "supported_params":
            return {"ok": True, "params": BASE.get_supported_openai_params(MODEL)}
        case "map_params":
            return map_params(call)
        case "validate_environment":
            return validate_environment(call)
        case "complete_url":
            return complete_url(call)
        case "error":
            return error(call)
        case "response":
            return response(call)


def run(input_value: Input) -> JsonValue:
    return {"calls": [answer(call) for call in input_value["calls"]]}
