import time
from collections.abc import Coroutine
from typing import Final, Literal, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.black_forest_labs.common_utils import BlackForestLabsError
from litellm.llms.black_forest_labs.image_generation.handler import (
    BlackForestLabsImageGeneration,
)
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
TEXT: Final = TypeAdapter(dict[str, str])
TIMEOUT: Final = TypeAdapter(dict[str, float | None])
TRANSPORT_HEADERS: Final = frozenset(
    {
        "accept",
        "accept-encoding",
        "connection",
        "content-length",
        "host",
        "user-agent",
    }
)
CLIENT_TIMEOUT_MILLIS: Final = 1234000
REAL_TIME: Final = time.time
REAL_SLEEP: Final = time.sleep


class ClockSettings(TypedDict):
    start_millis: int
    tick_millis: int
    sleep_advance_millis: int


class Configured(TypedDict):
    api_key: str | None
    api_base: str


class LiteLLMParams(TypedDict, total=False):
    api_key: str
    api_base: str


PARAMS: Final = TypeAdapter(LiteLLMParams)


class Reply(TypedDict):
    status: int
    body: str


class Job(TypedDict):
    model: str
    prompt: str
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    params_kind: Literal["dict", "typed"]
    extra_headers: dict[str, str]
    timeout_millis: int | None
    configured: Configured
    clock: ClockSettings
    submission: Reply
    polls: list[Reply]


class Input(TypedDict):
    jobs: list[Job]


class Recorder:
    def __init__(self) -> None:
        self.effects: list[JsonValue] = []

    def record(self, effect: dict[str, JsonValue]) -> None:
        self.effects.append(JSON.validate_python(effect))


class Clock:
    def __init__(self, settings: ClockSettings, recorder: Recorder) -> None:
        self.millis = settings["start_millis"]
        self.tick = settings["tick_millis"]
        self.advance = settings["sleep_advance_millis"]
        self.recorder = recorder

    def __call__(self) -> float:
        moment = self.millis
        self.millis += self.tick
        return moment / 1000

    def sleep(self, seconds: float) -> None:
        millis = round(seconds * 1000)
        self.recorder.record({"effect": "sleep", "millis": millis})
        self.millis += self.advance


class RecordingLogging(Logging):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    @override
    def pre_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        headers = TEXT.validate_python(arguments["headers"])
        names = sorted(name.lower() for name in headers)
        self.recorder.record(
            {
                "effect": "pre_call",
                "input": kwargs["input"],
                "api_base": arguments["api_base"],
                "body": arguments["complete_input_dict"],
                "header_names": JSON.validate_python(names),
            }
        )


class Transport:
    def __init__(self, job: Job, recorder: Recorder) -> None:
        self.job = job
        self.recorder = recorder
        self.poll_index = 0

    def __call__(self, request: httpx.Request) -> httpx.Response:
        reply = self.post_reply(request) if request.method == "POST" else self.get_reply(request)
        return httpx.Response(reply["status"], content=reply["body"].encode())

    def post_reply(self, request: httpx.Request) -> Reply:
        self.recorder.record(
            {
                "effect": "post",
                "url": str(request.url),
                "headers": JSON.validate_python(shown_headers(request)),
                "body": JSON.validate_json(request.content),
                "timeout_millis": read_timeout(request),
            }
        )
        return self.job["submission"]

    def get_reply(self, request: httpx.Request) -> Reply:
        self.recorder.record(
            {
                "effect": "get",
                "url": str(request.url),
                "headers": JSON.validate_python(shown_headers(request)),
            }
        )
        polls = self.job["polls"]
        index = min(self.poll_index, len(polls) - 1)
        self.poll_index += 1
        return polls[index] if polls else self.job["submission"]


def shown_headers(request: httpx.Request) -> dict[str, str]:
    return {
        name: value for name, value in request.headers.items() if name not in TRANSPORT_HEADERS
    }


def read_timeout(request: httpx.Request) -> int | None:
    extensions = OBJECT.validate_python(request.extensions)
    timeout = extensions.get("timeout")
    if timeout is None:
        return None
    read = TIMEOUT.validate_python(timeout).get("read")
    if read is None:
        return None
    millis = round(read * 1000)
    return None if millis == CLIENT_TIMEOUT_MILLIS else millis


def litellm_params(job: Job) -> GenericLiteLLMParams | dict[str, JsonValue]:
    if job["params_kind"] == "typed":
        typed = PARAMS.validate_python(job["litellm_params"])
        return GenericLiteLLMParams(api_key=typed.get("api_key"), api_base=typed.get("api_base"))
    return job["litellm_params"]


def call(
    handler: BlackForestLabsImageGeneration,
    job: Job,
    logging: RecordingLogging,
    client: HTTPHandler,
) -> ImageResponse:
    timeout = job["timeout_millis"]
    pending = handler.image_generation(
        model=job["model"],
        prompt=job["prompt"],
        model_response=ImageResponse(created=0, data=[]),
        optional_params=job["optional_params"],
        litellm_params=litellm_params(job),
        logging_obj=logging,
        timeout=None if timeout is None else timeout / 1000,
        extra_headers=job["extra_headers"],
        client=client,
    )
    match pending:
        case Coroutine():
            raise ValueError("the sync BFL entry point answered with a coroutine")
        case object():
            return pending


def outcome(job: Job, logging: RecordingLogging, client: HTTPHandler) -> JsonValue:
    try:
        response = call(BlackForestLabsImageGeneration(), job, logging, client)
    except BlackForestLabsError as error:
        return JSON.validate_python({"ok": False, "status": error.status_code})
    images = list(response.data) if response.data is not None else []
    urls: JsonValue = JSON.validate_python([image.url for image in images])
    return JSON.validate_python({"ok": True, "urls": urls})


def perform(job: Job) -> JsonValue:
    recorder = Recorder()
    clock = Clock(job["clock"], recorder)
    transport = Transport(job, recorder)
    logging = RecordingLogging(recorder)
    client = HTTPHandler(
        client=httpx.Client(
            transport=httpx.MockTransport(transport),
            timeout=httpx.Timeout(CLIENT_TIMEOUT_MILLIS / 1000),
        )
    )
    time.time = clock
    time.sleep = clock.sleep
    try:
        result = outcome(job, logging, client)
    finally:
        time.time = REAL_TIME
        time.sleep = REAL_SLEEP
    return JSON.validate_python({"effects": recorder.effects, "outcome": result})


def run(input_value: Input) -> JsonValue:
    jobs: list[JsonValue] = [perform(job) for job in input_value["jobs"]]
    return JSON.validate_python({"jobs": jobs})
