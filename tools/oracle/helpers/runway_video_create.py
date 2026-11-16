import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.runwayml.videos.transformation import RunwayMLVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoObject
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
EMPTY: Final[JsonObject] = {}
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False
RUNWAY_VARIABLES: Final = (
    "RUNWAYML_API_SECRET",
    "RUNWAYML_API_KEY",
    "RUNWAYML_DEFAULT_API_VERSION",
    "RUNWAYML_API_BASE",
)


VideoRequestTransform = Callable[
    [str, str, str, JsonObject, GenericLiteLLMParams, JsonObject],
    tuple[JsonValue, JsonValue, str],
]


VideoReplyTransform = Callable[
    [str, httpx.Response, Logging, str | None, JsonObject | None],
    VideoObject,
]


@runtime_checkable
class VideoConfig(Protocol):
    def map_openai_params(
        self, video_create_optional_request_params: JsonObject, model: str, *, drop_params: bool
    ) -> JsonObject: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: JsonObject
    ) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None,
        litellm_params: GenericLiteLLMParams,
    ) -> JsonObject: ...

    def use_multipart_form_data(self) -> bool: ...

    # ruff counts a def's parameters (PLR0913), so the six-argument call types are property values
    @property
    def transform_video_create_request(  # ast-grep-ignore: no-long-names
        self,
    ) -> VideoRequestTransform: ...

    @property
    def transform_video_create_response(  # ast-grep-ignore: no-long-names
        self,
    ) -> VideoReplyTransform: ...


# litellm's config class carries untyped parameters, so the protocol is the declared shape
CONFIG: Final[VideoConfig] = TypeAdapter[VideoConfig](
    VideoConfig, config=ARBITRARY_TYPES
).validate_python(RunwayMLVideoConfig())


class CreateCall(TypedDict):
    api_base: str | None
    api_key: str | None
    litellm_params_api_key: str | None
    global_api_key: str | None
    environment: dict[str, str]
    params: JsonObject


class CreateParams(TypedDict):
    model: str
    prompt: str
    calls: list[CreateCall]


class CreateInput(TypedDict):
    operation: Literal["create"]
    params: CreateParams


class ReplyCall(TypedDict):
    provider: str | None
    request_data: JsonObject
    body: str


class ReplyParams(TypedDict):
    model: str
    calls: list[ReplyCall]


class ReplyInput(TypedDict):
    operation: Literal["reply"]
    params: ReplyParams


Input = CreateInput | ReplyInput


def set_environment(environment: dict[str, str]) -> None:
    for name in RUNWAY_VARIABLES:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def logging_object(model: str) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=False,
        call_type="video_generation",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="runway-video-call",
        function_id="runway-video-function",
    )


def create_value(model: str, prompt: str, call: CreateCall) -> JsonValue:
    set_environment(call["environment"])
    litellm.api_key = call["global_api_key"]
    litellm_params = GenericLiteLLMParams(api_key=call["litellm_params_api_key"])
    try:
        headers = CONFIG.validate_environment({}, model, call["api_key"], litellm_params)
    except ValueError:
        return JSON.validate_python({"credentials_error": True})
    complete = CONFIG.get_complete_url(model, call["api_base"], EMPTY)
    mapped = CONFIG.map_openai_params(call["params"], model, drop_params=DROP_PARAMS)
    body, files, url = CONFIG.transform_video_create_request(
        model, prompt, complete, mapped, litellm_params, headers
    )
    answer: JsonObject = {
        "mapped": mapped,
        "url": url,
        "body": body,
        "files": files,
        "multipart": CONFIG.use_multipart_form_data(),
        "headers": headers,
    }
    return JSON.validate_python(answer)


def reply_value(model: str, call: ReplyCall) -> JsonValue:
    request = httpx.Request("POST", "https://api.runwayml.com/v1/text_to_video")
    raw_response = httpx.Response(
        200, content=call["body"].encode(), request=request
    )
    video = CONFIG.transform_video_create_response(
        model,
        raw_response,
        logging_object(model),
        call["provider"],
        call["request_data"],
    )
    return JSON.validate_json(video.model_dump_json())


def create_results(input_value: CreateInput) -> JsonValue:
    params = input_value["params"]
    results: list[JsonValue] = [
        create_value(params["model"], params["prompt"], call) for call in params["calls"]
    ]
    return JSON.validate_python({"results": results})


def reply_results(input_value: ReplyInput) -> JsonValue:
    params = input_value["params"]
    videos: list[JsonValue] = [reply_value(params["model"], call) for call in params["calls"]]
    return JSON.validate_python({"videos": videos})


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "create":
        return create_results(input_value)
    return reply_results(input_value)
