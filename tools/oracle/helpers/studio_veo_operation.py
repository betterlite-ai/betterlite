from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.constants import DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS
from litellm.llms.gemini.videos.transformation import GeminiVideoConfig
from litellm.types.llms.gemini import GeminiLongRunningOperationResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoObject
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)


@runtime_checkable
class VideoConfig(Protocol):
    transform_video_create_response: Callable[  # ast-grep-ignore: no-long-names
        [str, httpx.Response, None, str | None, JsonObject | None], VideoObject
    ]
    transform_video_status_retrieve_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, GenericLiteLLMParams, dict[str, str]], tuple[str, JsonObject]
    ]
    transform_video_status_retrieve_response: Callable[  # ast-grep-ignore: no-long-names
        [httpx.Response, None, str | None], VideoObject
    ]


# the config's __init__ is untyped, and the methods this file calls keep no state
CONFIG: Final[VideoConfig] = TypeAdapter[VideoConfig](
    VideoConfig, config=ARBITRARY_TYPES
).validate_python(GeminiVideoConfig.__new__(GeminiVideoConfig))


class CreateCall(TypedDict):
    function: Literal["create_response"]
    model: str
    provider: str | None
    request_data: JsonObject | None
    body: JsonValue


class StatusCall(TypedDict):
    function: Literal["status_response"]
    provider: str | None
    body: JsonValue


class StatusRequestCall(TypedDict):
    function: Literal["status_request"]
    video_id: str
    api_base: str


class ContentTargetCall(TypedDict):
    function: Literal["content_target"]
    body: JsonValue


Call: TypeAlias = CreateCall | StatusCall | StatusRequestCall | ContentTargetCall


class Input(TypedDict):
    default_duration_seconds: int
    calls: list[Call]


def check_duration(default_duration_seconds: int) -> None:
    # ast-grep-ignore: no-long-names
    if default_duration_seconds != DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS:
        raise ValueError("the case env must match default_duration_seconds")


def dump_video(video: VideoObject) -> JsonValue:
    dumped = video.model_dump_json()
    return JSON.validate_json(dumped)


def create_answer(call: CreateCall) -> JsonValue:
    response = httpx.Response(200, content=JSON.dump_json(call["body"]))
    video = CONFIG.transform_video_create_response(
        call["model"], response, None, call["provider"], call["request_data"]
    )
    return dump_video(video)


def status_answer(call: StatusCall) -> JsonValue:
    response = httpx.Response(200, content=JSON.dump_json(call["body"]))
    video = CONFIG.transform_video_status_retrieve_response(response, None, call["provider"])
    return dump_video(video)


def status_request_answer(call: StatusRequestCall) -> JsonValue:
    url, params = CONFIG.transform_video_status_retrieve_request(
        call["video_id"], call["api_base"], GenericLiteLLMParams(), {}
    )
    payload: JsonObject = {"url": url, "params": params}
    return JSON.validate_python(payload)


def content_target_answer(call: ContentTargetCall) -> JsonValue:
    operation = GeminiLongRunningOperationResponse.model_validate(call["body"])
    if not operation.done:
        raise ValueError("the operation is not done")
    response = operation.response
    if response is None:
        raise ValueError("the operation has no response")
    target = response.generateVideoResponse.generatedSamples[0].video.uri
    payload: JsonObject = {"target": target}
    return JSON.validate_python(payload)


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "create_response":
            return create_answer(call)
        case "status_response":
            return status_answer(call)
        case "status_request":
            return status_request_answer(call)
        case "content_target":
            return content_target_answer(call)


def run(input_value: Input) -> JsonValue:
    check_duration(input_value["default_duration_seconds"])
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    payload: JsonObject = {"calls": answers}
    return JSON.validate_python(payload)
