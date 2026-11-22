import asyncio
import datetime as dt
from typing import Final, Literal, NotRequired, TypedDict

import httpx
import litellm
from litellm.exceptions import APIConnectionError as LitellmAPIConnectionError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.videos.main import VideoObject
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

# the client's own read timeout: a request carrying it is one the caller named no timeout for
CLIENT_TIMEOUT_SECONDS: Final[float] = 123.0

litellm.suppress_debug_info = True


class Input(TypedDict):
    operation: Literal["remix", "avideo_remix", "list", "avideo_list"]
    litellm_call_id: str
    status: int
    body: str
    request_timeout_seconds: NotRequired[float]
    video_id: NotRequired[str]
    prompt: NotRequired[str]
    after: NotRequired[str | None]
    limit: NotRequired[int | None]
    order: NotRequired[str | None]
    custom_llm_provider: NotRequired[str | None]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    timeout: NotRequired[int | None]
    extra_headers: NotRequired[dict[str, str] | None]
    extra_body: NotRequired[dict[str, JsonValue] | None]
    extra_query: NotRequired[dict[str, JsonValue] | None]
    mock_response: NotRequired[JsonValue]
    user: NotRequired[str | None]


class Responder:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.seen = False
        self.method = ""
        self.url = ""
        self.query: list[tuple[str, str]] = []
        self.headers: dict[str, str] = {}
        self.body_text = ""
        self.timeout: float | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen = True
        self.method = request.method
        self.url = str(request.url).split("?")[0]
        self.query = sorted(request.url.params.multi_items())
        self.headers = dict(request.headers)
        self.body_text = request.content.decode()
        timeouts: dict[str, float | None] | None = request.extensions.get("timeout")
        read = None if timeouts is None else timeouts.get("read")
        self.timeout = None if read == CLIENT_TIMEOUT_SECONDS else read
        return httpx.Response(self.status, content=self.body.encode())


def request_view(responder: Responder, wanted: tuple[str, ...]) -> JsonValue:
    if not responder.seen:
        return None
    headers: dict[str, JsonValue] = {
        name: value for name, value in responder.headers.items() if name in wanted
    }
    return {
        "method": responder.method,
        "url": responder.url,
        "query": JSON.validate_python([[name, value] for name, value in responder.query]),
        "headers": headers,
        "body": None if responder.body_text == "" else JSON.validate_json(responder.body_text),
        "timeout": responder.timeout,
    }


def logged(log: Logging) -> JsonValue:
    details: dict[str, JsonValue] = log.model_call_details
    return {
        "call_type": str(log.call_type),
        "model": details.get("model"),
        "detail_call_type": details.get("call_type"),
        "user": details.get("user"),
        "custom_llm_provider": details.get("custom_llm_provider"),
        "litellm_call_id": details.get("litellm_call_id"),
        "optional_params": details.get("optional_params"),
        "video_id": details.get("video_id"),
        "prompt": details.get("prompt"),
        "after": details.get("after"),
        "limit": details.get("limit"),
        "order": details.get("order"),
    }


def page_view(page: list[VideoObject] | dict[str, JsonValue]) -> JsonValue:
    match page:
        case list():
            return [JSON.validate_json(item.model_dump_json()) for item in page]
        case dict():
            return JSON.validate_python(page)
        case object():
            raise ValueError("the video list answered with neither a page nor a list")


def wanted_headers(input_value: Input) -> tuple[str, ...]:
    extra = input_value.get("extra_headers")
    names = () if extra is None else tuple(name.lower() for name in extra)
    return ("authorization", *names)


def run(input_value: Input) -> JsonValue:
    responder = Responder(input_value["status"], input_value["body"])
    transport = httpx.MockTransport(responder.handle)
    caller = input_value["litellm_call_id"]
    log = Logging(
        model="",
        messages=None,
        stream=False,
        call_type="",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id=caller,
        function_id="",
    )
    operation = input_value["operation"]
    if operation == "remix":
        client: HTTPHandler | AsyncHTTPHandler = HTTPHandler(
            client=httpx.Client(transport=transport)
        )
    else:
        client = AsyncHTTPHandler(transport=transport, timeout=CLIENT_TIMEOUT_SECONDS)
    options: dict[str, JsonValue] = {
        "custom_llm_provider": JSON.validate_python(input_value.get("custom_llm_provider")),
        "api_base": JSON.validate_python(input_value.get("api_base")),
        "api_key": JSON.validate_python(input_value.get("api_key")),
        "user": JSON.validate_python(input_value.get("user")),
        "extra_headers": JSON.validate_python(input_value.get("extra_headers")),
    }
    if "extra_body" in input_value:
        options["extra_body"] = JSON.validate_python(input_value["extra_body"])
    if "extra_query" in input_value:
        options["extra_query"] = JSON.validate_python(input_value["extra_query"])
    if "mock_response" in input_value:
        options["mock_response"] = JSON.validate_python(input_value["mock_response"])
    if "timeout" in input_value:
        options["timeout"] = JSON.validate_python(input_value["timeout"])
    outcome: dict[str, JsonValue] = {"request": None, "response": None, "error": None}
    try:
        match operation:
            case "remix":
                video: VideoObject = litellm.video_remix(
                    input_value["video_id"],
                    input_value["prompt"],
                    client=client,
                    litellm_logging_obj=log,
                    litellm_call_id=caller,
                    **options,
                )
                outcome["response"] = JSON.validate_json(video.model_dump_json())
            case "avideo_remix":
                remixed: VideoObject = asyncio.run(
                    litellm.avideo_remix(
                        input_value["video_id"],
                        input_value["prompt"],
                        client=client,
                        litellm_logging_obj=log,
                        litellm_call_id=caller,
                        **options,
                    )
                )
                outcome["response"] = JSON.validate_json(remixed.model_dump_json())
            case "list":
                options["after"] = JSON.validate_python(input_value.get("after"))
                options["limit"] = JSON.validate_python(input_value.get("limit"))
                options["order"] = JSON.validate_python(input_value.get("order"))
                page: list[VideoObject] | dict[str, JsonValue] = litellm.video_list(
                    client=client,
                    litellm_logging_obj=log,
                    litellm_call_id=caller,
                    **options,
                )
                outcome["response"] = page_view(page)
            case "avideo_list":
                options["after"] = JSON.validate_python(input_value.get("after"))
                options["limit"] = JSON.validate_python(input_value.get("limit"))
                options["order"] = JSON.validate_python(input_value.get("order"))
                listed: list[VideoObject] | dict[str, JsonValue] = asyncio.run(
                    litellm.avideo_list(
                        client=client,
                        litellm_logging_obj=log,
                        litellm_call_id=caller,
                        **options,
                    )
                )
                outcome["response"] = page_view(listed)
    except APIStatusError as error:
        outcome["error"] = {
            "status": error.status_code,
            "type": error.type,
            "param": error.param,
            "code": error.code,
        }
    except LitellmAPIConnectionError as error:
        outcome["error"] = {
            "status": error.status_code,
            "type": error.type,
            "param": error.param,
            "code": error.code,
        }
    outcome["request"] = request_view(responder, wanted_headers(input_value))
    outcome["logging"] = logged(log)
    return outcome
