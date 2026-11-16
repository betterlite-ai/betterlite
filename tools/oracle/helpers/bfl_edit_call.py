import base64
import time
from collections.abc import Awaitable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.black_forest_labs.common_utils import BlackForestLabsError
from litellm.llms.black_forest_labs.image_edit.handler import BlackForestLabsImageEdit
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import FileTypes, ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
TEXT: Final = TypeAdapter(dict[str, str])
TIMEOUT: Final = TypeAdapter(dict[str, float | None])
HANDLER: Final[type[BlackForestLabsImageEdit]] = BlackForestLabsImageEdit
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


class ImageBytes(TypedDict):
    kind: Literal["bytes"]
    data: str


class ImageList(TypedDict):
    kind: Literal["list"]
    items: list[str]


ImageSpec: TypeAlias = ImageBytes | ImageList


class Job(TypedDict):
    model: str
    prompt: str | None
    image: ImageSpec
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


class Wire:
    def __init__(self, job: Job, effects: list[JsonValue]) -> None:
        self.job = job
        self.effects = effects
        self.millis = job["clock"]["start_millis"]
        self.poll_index = 0

    def record(self, effect: dict[str, JsonValue]) -> None:
        self.effects.append(effect)

    def now(self) -> float:
        moment = self.millis
        self.millis += self.job["clock"]["tick_millis"]
        return moment / 1000

    def sleep(self, seconds: float) -> None:
        self.record({"effect": "sleep", "millis": round(seconds * 1000)})
        self.millis += self.job["clock"]["sleep_advance_millis"]

    def __call__(self, request: httpx.Request) -> httpx.Response:
        reply = self.post_reply(request) if request.method == "POST" else self.get_reply(request)
        return httpx.Response(reply["status"], content=reply["body"].encode())

    def post_reply(self, request: httpx.Request) -> Reply:
        self.record(
            {
                "effect": "post",
                "url": str(request.url),
                "headers": JSON.validate_python(shown_headers(request)),
                "body": JSON.validate_json(request.content),
                "timeout_millis": request_timeout(request),
            }
        )
        return self.job["submission"]

    def get_reply(self, request: httpx.Request) -> Reply:
        self.record(
            {
                "effect": "get",
                "url": str(request.url),
                "headers": JSON.validate_python(shown_headers(request)),
            }
        )
        polls = self.job["polls"]
        reply = polls[min(self.poll_index, len(polls) - 1)] if polls else self.job["submission"]
        self.poll_index += 1
        return reply


class RecordingLogging(Logging):
    def __init__(self, effects: list[JsonValue]) -> None:
        self.effects = effects

    @override
    def pre_call(self, *_args: JsonValue, **kwargs: JsonValue) -> None:
        arguments = OBJECT.validate_python(kwargs["additional_args"])
        headers = TEXT.validate_python(arguments["headers"])
        names: list[str] = sorted(name.lower() for name in headers)
        self.effects.append(
            {
                "effect": "pre_call",
                "input": kwargs["input"],
                "api_base": arguments["api_base"],
                "body": arguments["complete_input_dict"],
                "header_names": JSON.validate_python(names),
            }
        )


def shown_headers(request: httpx.Request) -> dict[str, str]:
    kept = (
        (name, value)
        for name, value in request.headers.items()
        if name not in TRANSPORT_HEADERS
    )
    return dict(kept)


def request_timeout(request: httpx.Request) -> int | None:
    extensions: dict[str, JsonValue] = request.extensions
    timeout = extensions.get("timeout")
    if timeout is None:
        return None
    read = TIMEOUT.validate_python(timeout).get("read")
    millis = None if read is None else round(read * 1000)
    return None if millis == CLIENT_TIMEOUT_MILLIS else millis


def media(spec: ImageSpec) -> FileTypes | list[FileTypes]:
    if spec["kind"] == "list":
        images: list[FileTypes] = [base64.b64decode(item) for item in spec["items"]]
        return images
    return base64.b64decode(spec["data"])


def call_params(job: Job) -> GenericLiteLLMParams | dict[str, JsonValue]:
    if job["params_kind"] == "dict":
        return job["litellm_params"]
    typed = PARAMS.validate_python(job["litellm_params"])
    return GenericLiteLLMParams(api_key=typed.get("api_key"), api_base=typed.get("api_base"))


class ImageEditHandler(Protocol):
    def image_edit(
        self,
        _model: str,
        _image: FileTypes | list[FileTypes],
        _prompt: str | None,
        _optional_params: dict[str, JsonValue],
        _litellm_params: GenericLiteLLMParams | dict[str, JsonValue],
        _logging: Logging,
        _timeout: float | None,
        _extra_headers: dict[str, str],
        _client: HTTPHandler,
        /,
    ) -> ImageResponse | Awaitable[ImageResponse]: ...


def call(
    handler: ImageEditHandler,
    job: Job,
    logging: RecordingLogging,
    client: HTTPHandler,
) -> ImageResponse:
    timeout = job["timeout_millis"]
    pending = handler.image_edit(
        job["model"],
        media(job["image"]),
        job["prompt"],
        job["optional_params"],
        call_params(job),
        logging,
        None if timeout is None else timeout / 1000,
        job["extra_headers"],
        client,
    )
    match pending:
        case Awaitable():
            raise ValueError("the sync BFL edit entry point answered with an awaitable")
        case object():
            return pending


def projection(response: ImageResponse) -> dict[str, JsonValue]:
    images = list(response.data) if response.data is not None else []
    urls: list[str | None] = [image.url for image in images]
    return {"ok": True, "urls": JSON.validate_python(urls)}


def outcome(job: Job, logging: RecordingLogging, client: HTTPHandler) -> JsonValue:
    try:
        return projection(call(HANDLER(), job, logging, client))
    except ERROR as error:
        failure: dict[str, JsonValue] = {"ok": False, "status": error.status_code}
        return failure


def perform(job: Job) -> JsonValue:
    effects: list[JsonValue] = []
    wire = Wire(job, effects)
    logging = RecordingLogging(effects)
    client = HTTPHandler(
        client=httpx.Client(
            transport=httpx.MockTransport(wire),
            timeout=httpx.Timeout(CLIENT_TIMEOUT_MILLIS / 1000),
        )
    )
    time.time = wire.now
    time.sleep = wire.sleep
    try:
        result = outcome(job, logging, client)
    finally:
        time.time = REAL_TIME
        time.sleep = REAL_SLEEP
    record: dict[str, JsonValue] = {"effects": effects, "outcome": result}
    return record


def run(input_value: Input) -> JsonValue:
    jobs: list[JsonValue] = [perform(job) for job in input_value["jobs"]]
    payload: dict[str, JsonValue] = {"jobs": jobs}
    return payload
