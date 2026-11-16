import os
import time
from collections.abc import Callable
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.constants import DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS
from litellm.llms.vertex_ai.videos.transformation import VertexAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoObject
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)


@runtime_checkable
class VeoOperationConfig(Protocol):
    transform_video_create_response: Callable[  # ast-grep-ignore: no-long-names
        [str, httpx.Response, None, str | None, JsonObject | None], VideoObject
    ]
    transform_video_status_retrieve_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, GenericLiteLLMParams, dict[str, str]], tuple[str, JsonObject]
    ]
    transform_video_status_retrieve_response: Callable[  # ast-grep-ignore: no-long-names
        [httpx.Response, None, str | None], VideoObject
    ]
    transform_video_content_response: Callable[  # ast-grep-ignore: no-long-names
        [httpx.Response, None], bytes
    ]
    transform_video_edit_response: Callable[
        [httpx.Response, None, str | None, JsonObject | None], VideoObject
    ]


# LiteLLM's __init__ is untyped, and every method this file calls is stateless
HANDLER: Final = VertexAIVideoConfig.__new__(VertexAIVideoConfig)
# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[VeoOperationConfig] = TypeAdapter[VeoOperationConfig](
    VeoOperationConfig, config=ARBITRARY_TYPES
).validate_python(HANDLER)


class CreateCall(TypedDict):
    function: Literal["create_response"]
    model: str
    provider: str | None
    request_data: JsonObject | None
    response: JsonValue


class StatusCall(TypedDict):
    function: Literal["status_response"]
    provider: str | None
    response: JsonValue
    now: int
    time_zone: str


class ContentCall(TypedDict):
    function: Literal["content_response"]
    response: JsonValue


class EditCall(TypedDict):
    function: Literal["edit_response"]
    provider: str | None
    request_data: JsonObject | None
    response: JsonValue


class FetchCall(TypedDict):
    function: Literal["fetch_request"]
    video_id: str
    api_base: str


Call: TypeAlias = CreateCall | StatusCall | ContentCall | EditCall | FetchCall


class Input(TypedDict):
    default_duration_seconds: int
    calls: list[Call]


def check_duration(default_duration_seconds: int) -> None:
    # ast-grep-ignore: no-long-names
    if DEFAULT_GOOGLE_VIDEO_DURATION_SECONDS != default_duration_seconds:
        raise ValueError("the case env must match default_duration_seconds")


def create_answer(call: CreateCall) -> JsonValue:
    response = httpx.Response(200, content=JSON.dump_json(call["response"]))
    video = CONFIG.transform_video_create_response(
        call["model"], response, None, call["provider"], call["request_data"]
    )
    return JSON.validate_json(video.model_dump_json())


def status_answer(call: StatusCall) -> JsonValue:
    os.environ["TZ"] = call["time_zone"]
    time.tzset()
    saved = time.time
    time.time = partial(float, call["now"])
    try:
        response = httpx.Response(200, content=JSON.dump_json(call["response"]))
        video = CONFIG.transform_video_status_retrieve_response(response, None, call["provider"])
    finally:
        time.time = saved
    return JSON.validate_json(video.model_dump_json())


def content_answer(call: ContentCall) -> JsonValue:
    response = httpx.Response(200, content=JSON.dump_json(call["response"]))
    content = CONFIG.transform_video_content_response(response, None)
    values: list[JsonValue] = []
    values.extend(content)
    payload: JsonObject = {"bytes": values}
    return JSON.validate_python(payload)


def edit_answer(call: EditCall) -> JsonValue:
    response = httpx.Response(200, content=JSON.dump_json(call["response"]))
    video = CONFIG.transform_video_edit_response(
        response, None, call["provider"], call["request_data"]
    )
    return JSON.validate_json(video.model_dump_json())


def fetch_answer(call: FetchCall) -> JsonValue:
    url, params = CONFIG.transform_video_status_retrieve_request(
        call["video_id"], call["api_base"], GenericLiteLLMParams(), {}
    )
    payload: JsonObject = {"url": url, "params": params}
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    check_duration(input_value["default_duration_seconds"])
    answers: list[JsonValue] = []
    for call in input_value["calls"]:
        match call["function"]:
            case "create_response":
                answers.append(create_answer(call))
            case "status_response":
                answers.append(status_answer(call))
            case "content_response":
                answers.append(content_answer(call))
            case "edit_response":
                answers.append(edit_answer(call))
            case "fetch_request":
                answers.append(fetch_answer(call))
    payload: JsonObject = {"calls": answers}
    return JSON.validate_python(payload)
