import time
from collections.abc import Awaitable
from typing import Final, Literal, Protocol, SupportsIndex, TypedDict

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
HANDLER: Final[type[BlackForestLabsImageGeneration]] = BlackForestLabsImageGeneration
ERROR: Final[type[BlackForestLabsError]] = BlackForestLabsError
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


class Clock:
    def __init__(self, settings: ClockSettings, effects: list[JsonValue]) -> None:
        self.millis = settings["start_millis"]
        self.tick = settings["tick_millis"]
        self.advance = settings["sleep_advance_millis"]
        self.effects = effects

    def __call__(self) -> float:
        moment = self.millis
        self.millis += self.tick
        return moment / 1000

    def sleep(self, seconds: float | SupportsIndex) -> None:
        effect: dict[str, JsonValue] = {
            "effect": "sleep",
            "millis": round(float(seconds) * 1000),
        }
        self.effects.append(effect)
        self.millis += self.advance


class RecordingLogging(Logging):
    def __init__(self, effects: list[JsonValue]) -> None:
        self.effects = effects

    @override
    def pre_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        headers = TEXT.validate_python(arguments["headers"])
        names: list[str] = sorted(name.lower() for name in headers)
        effect: dict[str, JsonValue] = {
            "effect": "pre_call",
            "input": kwargs["input"],
            "api_base": arguments["api_base"],
            "body": arguments["complete_input_dict"],
            "header_names": JSON.validate_python(names),
        }
        self.effects.append(effect)


class Transport:
    def __init__(self, job: Job, effects: list[JsonValue]) -> None:
        self.job = job
        self.effects = effects
        self.poll_index = 0

    def __call__(self, request: httpx.Request) -> httpx.Response:
        reply = self.post_reply(request) if request.method == "POST" else self.get_reply(request)
        return httpx.Response(reply["status"], content=reply["body"].encode())

    def post_reply(self, request: httpx.Request) -> Reply:
        effect: dict[str, JsonValue] = {
            "effect": "post",
            "url": str(request.url),
            "headers": JSON.validate_python(shown_headers(request)),
            "body": JSON.validate_json(request.content),
            "timeout_millis": read_timeout(request),
        }
        self.effects.append(effect)
        return self.job["submission"]

    def get_reply(self, request: httpx.Request) -> Reply:
        effect: dict[str, JsonValue] = {
            "effect": "get",
            "url": str(request.url),
            "headers": JSON.validate_python(shown_headers(request)),
        }
        self.effects.append(effect)
        polls = self.job["polls"]
        index = min(self.poll_index, len(polls) - 1)
        self.poll_index += 1
        return polls[index] if polls else self.job["submission"]


def shown_headers(request: httpx.Request) -> dict[str, str]:
    return {
        name: value for name, value in request.headers.items() if name not in TRANSPORT_HEADERS
    }


def read_timeout(request: httpx.Request) -> int | None:
    extensions: dict[str, JsonValue] = request.extensions
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


class ImageGenerationHandler(Protocol):
    def image_generation(
        self,
        _model: str,
        _prompt: str,
        _model_response: ImageResponse,
        _optional_params: dict[str, JsonValue],
        _litellm_params: GenericLiteLLMParams | dict[str, JsonValue],
        _logging_obj: Logging,
        _timeout: float | None,
        _extra_headers: dict[str, str],
        _client: HTTPHandler,
        /,
    ) -> ImageResponse | Awaitable[ImageResponse]: ...


def call(
    handler: ImageGenerationHandler,
    job: Job,
    logging: RecordingLogging,
    client: HTTPHandler,
) -> ImageResponse:
    timeout = job["timeout_millis"]
    pending = handler.image_generation(
        job["model"],
        job["prompt"],
        ImageResponse(created=0, data=[]),
        job["optional_params"],
        litellm_params(job),
        logging,
        None if timeout is None else timeout / 1000,
        job["extra_headers"],
        client,
    )
    match pending:
        case Awaitable():
            raise ValueError("the sync BFL entry point answered with an awaitable")
        case object():
            return pending


def outcome(job: Job, logging: RecordingLogging, client: HTTPHandler) -> JsonValue:
    try:
        response = call(HANDLER(), job, logging, client)
    except ERROR as error:
        failed: dict[str, JsonValue] = {"ok": False, "status": error.status_code}
        return failed
    images = list(response.data) if response.data is not None else []
    urls: list[str | None] = [image.url for image in images]
    succeeded: dict[str, JsonValue] = {"ok": True, "urls": JSON.validate_python(urls)}
    return succeeded


def perform(job: Job) -> JsonValue:
    effects: list[JsonValue] = []
    clock = Clock(job["clock"], effects)
    transport = Transport(job, effects)
    logging = RecordingLogging(effects)
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
    recorded: dict[str, JsonValue] = {"effects": effects, "outcome": result}
    return recorded


def run(input_value: Input) -> JsonValue:
    jobs: list[JsonValue] = [perform(job) for job in input_value["jobs"]]
    payload: dict[str, JsonValue] = {"jobs": jobs}
    return payload
