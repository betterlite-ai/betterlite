import os
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import httpx
import litellm
from litellm.llms.soniox.audio_transcription.handler import (
    SonioxAudioTranscriptionHandler as Handler,
)
from litellm.llms.soniox.audio_transcription.transformation import (
    SonioxAudioTranscriptionConfig as Config,
)
from litellm.llms.soniox.common_utils import SonioxException
from pydantic import JsonValue, TypeAdapter

from xai_files import guarded

Object: TypeAlias = dict[str, JsonValue]
EMPTY: Final[Object] = {}
EMPTY_HEADERS: Final[dict[str, str]] = {}
ENVIRONMENT_NAMES: Final[tuple[str, str]] = ("SONIOX_API_KEY", "SONIOX_API_BASE")

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
CONFIG: Final = Config.__new__(Config)
HANDLER: Final = Handler()


class HandlerOptions(TypedDict):
    poll_interval: float
    max_attempts: int
    cleanup: list[str]
    filename_override: str | None
    audio_url: str | None
    file_id: str | None
    response_format: str | None


class Recorder:
    failing: bool
    recorded: JsonValue

    def __init__(self, *, failing: bool) -> None:
        self.failing = failing
        self.recorded = None

    def pre_call(self, **kwargs: JsonValue) -> None:
        if self.failing:
            raise RuntimeError("callback boom")
        self.recorded = {
            "input": kwargs["input"],
            "additional_args": kwargs["additional_args"],
        }

    def post_call(self, **kwargs: JsonValue) -> None:
        if self.failing:
            raise RuntimeError("callback boom")
        self.recorded = {
            "input": kwargs["input"],
            "additional_args": kwargs["additional_args"],
            "original_response": kwargs["original_response"],
        }


class PrepareKwargs(TypedDict):
    audio_file: None
    optional_params: Object
    litellm_params: Object
    api_key: str | None
    api_base: str | None
    provider_config: Config
    headers: dict[str, str]


class PrepareFn(Protocol):
    def __call__(
        self, **kwargs: Unpack[PrepareKwargs]
    ) -> tuple[dict[str, str], str, Object, HandlerOptions]: ...


class BuildBodyFn(Protocol):
    def __call__(
        self, model: str, optional_params: Object, handler_opts: HandlerOptions, file_id: str | None
    ) -> Object: ...


class RedactFn(Protocol):
    def __call__(self, body: Object) -> Object: ...


class PreLogFn(Protocol):
    def __call__(
        self, logging_obj: Recorder, api_key: str | None, api_base: str, body: Object
    ) -> None: ...


class PostLogFn(Protocol):
    def __call__(
        self,
        logging_obj: Recorder,
        audio_file: str | None,
        api_key: str | None,
        body: Object,
        original_response: Object,
    ) -> None: ...


class RaiseFn(Protocol):
    def __call__(self, response: httpx.Response, provider_config: Config, action: str) -> None: ...


PREPARE, BUILD_BODY, REDACT, PRE_CALL, POST_CALL, RAISE = cast(PrepareFn, HANDLER._prepare), cast(BuildBodyFn, HANDLER._build_create_body), cast(RedactFn, HANDLER._redact_body_for_logging), cast(PreLogFn, HANDLER._safe_log_pre_call), cast(PostLogFn, HANDLER._safe_log_post_call), cast(RaiseFn, HANDLER._raise_for_response)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/soniox/audio_transcription/handler.py#L173-L352


class PrepareArgs(TypedDict):
    optional_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]


class PrepareCall(PrepareArgs):
    function: Literal["prepare"]


class CreateBodyCall(TypedDict):
    function: Literal["create_body"]
    prepare: PrepareArgs
    model: str
    file_id: NotRequired[str | None]


class RedactCall(TypedDict):
    function: Literal["redact"]
    body: Object


class RaiseCall(TypedDict):
    function: Literal["raise"]
    status: int
    action: str
    body: str
    headers: NotRequired[dict[str, str]]
    probes: NotRequired[list[str]]


class PreCallCall(TypedDict):
    function: Literal["pre_call"]
    api_base: str
    body: Object
    failing: bool


class PostCallCall(TypedDict):
    function: Literal["post_call"]
    audio: NotRequired[str | None]
    original_response: Object
    body: Object
    failing: bool


Call: TypeAlias = (
    PrepareCall | CreateBodyCall | RedactCall | RaiseCall | PreCallCall | PostCallCall
)


class Input(TypedDict):
    calls: list[Call]


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def applied(args: PrepareArgs) -> tuple[dict[str, str], str, Object, HandlerOptions]:
    supplied = args.get("optional_params")
    optional_params = EMPTY if supplied is None else supplied
    given_headers = args.get("headers")
    return PREPARE(
        audio_file=None,
        optional_params=optional_params,
        litellm_params=EMPTY,
        api_key=args.get("api_key"),
        api_base=args.get("api_base"),
        provider_config=CONFIG,
        headers=EMPTY_HEADERS if given_headers is None else given_headers,
    )


def prepare_view(call: PrepareCall) -> JsonValue:
    supplied = call.get("optional_params")
    optional_params = EMPTY if supplied is None else supplied
    auth_headers, api_base, params, handler_opts = applied(call)
    return {
        "ok": True,
        "auth_headers": JSON.validate_python(auth_headers),
        "api_base": api_base,
        "params": params,
        "options": JSON.validate_python(
            {
                "poll_interval": handler_opts["poll_interval"],
                "max_attempts": handler_opts["max_attempts"],
                "cleanup": handler_opts["cleanup"],
                "filename_override": handler_opts["filename_override"],
                "audio_url": handler_opts["audio_url"],
                "file_id": handler_opts["file_id"],
                "response_format": handler_opts["response_format"],
            }
        ),
        "input": optional_params,
    }


def prepare(call: PrepareCall) -> JsonValue:
    environment = call.get("environment")
    apply_environment({} if environment is None else environment)
    return guarded(prepare_view, call)


def create_body_view(call: CreateBodyCall) -> JsonValue:
    _auth_headers, _api_base, params, handler_opts = applied(call["prepare"])
    body = BUILD_BODY(call["model"], params, handler_opts, call.get("file_id"))
    return {"ok": True, "body": body}


def create_body(call: CreateBodyCall) -> JsonValue:
    environment = call["prepare"].get("environment")
    apply_environment({} if environment is None else environment)
    return guarded(create_body_view, call)


def raise_view(call: RaiseCall) -> JsonValue:
    given = call.get("headers")
    headers = EMPTY_HEADERS if given is None else given
    given_probes = call.get("probes")
    probes = [] if given_probes is None else given_probes
    response = httpx.Response(call["status"], content=call["body"].encode(), headers=headers)
    try:
        RAISE(response, CONFIG, call["action"])
    except SonioxException as error:
        return {
            "raised": error.status_code,
            "headers": JSON.validate_python(
                {name.lower(): response.headers.get(name) for name in headers}
            ),
            "probes": JSON.validate_python(
                {probe: probe in error.message for probe in probes}
            ),
        }
    return {"ok": True}


def pre_call(call: PreCallCall) -> JsonValue:
    recorder = Recorder(failing=call["failing"])
    PRE_CALL(recorder, None, call["api_base"], call["body"])
    return {"recorded": recorder.recorded, "body": call["body"]}


def post_call(call: PostCallCall) -> JsonValue:
    recorder = Recorder(failing=call["failing"])
    POST_CALL(recorder, call.get("audio"), None, call["body"], call["original_response"])
    return {"recorded": recorder.recorded, "body": call["body"]}


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "prepare":
            return prepare(call)
        case "create_body":
            return create_body(call)
        case "redact":
            return {"redacted": REDACT(call["body"]), "body": call["body"]}
        case "raise":
            return raise_view(call)
        case "pre_call":
            return pre_call(call)
        case "post_call":
            return post_call(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
