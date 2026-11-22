import asyncio
import datetime as dt
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypedDict, TypeVar, cast

import httpx
import litellm
from litellm.constants import request_timeout
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.videos.main import VideoObject
from openai import APIConnectionError, APIStatusError
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

# the handler's own read timeout: a request that carries it named no timeout of its own
CLIENT_TIMEOUT_SECONDS: Final[float] = 123.0
SIGNATURE_TIMEOUT_SECONDS: Final[int] = 600

Operation = Literal["remix", "avideo_remix", "list", "avideo_list"]
Value = TypeVar("Value")

litellm.suppress_debug_info = True


class Input(TypedDict):
    operation: Operation
    litellm_call_id: str
    status: int
    body: str
    video_id: str
    prompt: str
    after: NotRequired[str | None]
    limit: NotRequired[int | None]
    order: NotRequired[str | None]
    custom_llm_provider: NotRequired[str | None]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    timeout: NotRequired[int | None]
    extra_headers: NotRequired[dict[str, str] | None]
    extra_query: NotRequired[dict[str, JsonValue] | None]
    extra_body: NotRequired[dict[str, JsonValue] | None]
    mock_response: NotRequired[JsonValue]
    user: NotRequired[str | None]


class Extensions(BaseModel):
    timeout: dict[str, float | None] | None = None


# httpx keeps the per-request timeout in a dict of Any, so a model reads the read phase out of it
class SeenRequest(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    extensions: Extensions


class CallDetails(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str | None = None
    call_type: str | None = None
    user: str | None = None
    custom_llm_provider: str | None = None
    litellm_call_id: str | None = None
    optional_params: dict[str, JsonValue] | None = None
    video_id: str | None = None
    prompt: str | None = None
    after: str | None = None
    limit: int | None = None
    order: str | None = None


class SeenLog(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    call_type: str
    model_call_details: CallDetails


class SeenError(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None
    error_type: str | None = Field(default=None, alias="type")
    param: str | None = None
    code: str | None = None


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
        if not self.seen:
            self.seen = True
            self.method = request.method
            self.url = str(request.url).split("?")[0]
            self.query = sorted(request.url.params.multi_items())
            self.headers = dict(request.headers)
            self.body_text = request.content.decode()
            extensions = SeenRequest.model_validate(request).extensions
            timeout = None if extensions.timeout is None else extensions.timeout.get("read")
            self.timeout = None if timeout == CLIENT_TIMEOUT_SECONDS else timeout
        return httpx.Response(self.status, content=self.body.encode())


def as_dict(value: Mapping[str, Value] | None) -> dict[str, Value] | None:
    if value is None:
        return None
    return dict(value)


def request_view(responder: Responder, wanted: tuple[str, ...]) -> JsonValue:
    if not responder.seen:
        return None
    headers: dict[str, JsonValue] = {
        name: value for name, value in responder.headers.items() if name in wanted
    }
    pairs: list[list[str]] = [[name, value] for name, value in responder.query]
    return {
        "method": responder.method,
        "url": responder.url,
        "query": JSON.validate_python(pairs),
        "headers": headers,
        "body": None if responder.body_text == "" else JSON.validate_json(responder.body_text),
        "timeout": responder.timeout,
    }


def logged(log: Logging) -> JsonValue:
    seen = SeenLog.model_validate(log)
    return {
        "call_type": seen.call_type,
        "model": seen.model_call_details.model,
        "detail_call_type": seen.model_call_details.call_type,
        "user": seen.model_call_details.user,
        "custom_llm_provider": seen.model_call_details.custom_llm_provider,
        "litellm_call_id": seen.model_call_details.litellm_call_id,
        "optional_params": seen.model_call_details.optional_params,
        "video_id": seen.model_call_details.video_id,
        "prompt": seen.model_call_details.prompt,
        "after": seen.model_call_details.after,
        "limit": seen.model_call_details.limit,
        "order": seen.model_call_details.order,
    }


def page_view(page: list[VideoObject] | dict[str, JsonValue]) -> JsonValue:
    match page:
        case list():
            return [JSON.validate_json(item.model_dump_json()) for item in page]
        case dict():
            return JSON.validate_python(page)


def timeout_seconds(input_value: Input) -> int:
    if "timeout" not in input_value:
        return SIGNATURE_TIMEOUT_SECONDS
    given = input_value.get("timeout")
    if given is None:
        return int(request_timeout)
    return given


def list_provider(input_value: Input) -> str | None:
    provider = input_value.get("custom_llm_provider")
    if input_value["operation"] == "list" or provider is not None:
        return provider
    _, inferred, _, _ = get_llm_provider(model="")
    return inferred


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    responder = Responder(input_value["status"], input_value["body"])
    transport = httpx.MockTransport(responder.handle)
    log = Logging(
        model="", messages=None, stream=False, call_type="",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id=input_value["litellm_call_id"], function_id="",
    )
    client: AsyncHTTPHandler | HTTPHandler = (
        HTTPHandler(client=httpx.Client(transport=transport, timeout=CLIENT_TIMEOUT_SECONDS))
        if operation == "remix"
        else AsyncHTTPHandler(transport=transport, timeout=CLIENT_TIMEOUT_SECONDS)
    )
    headers = cast("dict[str, object] | None", as_dict(input_value.get("extra_headers")))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L528
    query = cast("dict[str, object] | None", as_dict(input_value.get("extra_query")))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L529
    body = cast("dict[str, object] | None", as_dict(input_value.get("extra_body")))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L530
    mock = input_value.get("mock_response")
    response: JsonValue = None
    error: JsonValue = None
    try:
        if operation in ("remix", "avideo_remix"):
            if operation == "avideo_remix" and mock is None:
                remixed = litellm.video_remix(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L519-L553
                    input_value["video_id"],
                    input_value["prompt"],
                    timeout=timeout_seconds(input_value),
                    custom_llm_provider=input_value.get("custom_llm_provider"),
                    extra_headers=headers,
                    extra_query=query,
                    extra_body=body,
                    api_base=input_value.get("api_base"),
                    api_key=input_value.get("api_key"),
                    user=input_value.get("user"),
                    client=client,
                    litellm_logging_obj=log,
                    litellm_call_id=input_value["litellm_call_id"],
                    avideo_remix=True,
                    async_call=True,
                )
                response = JSON.validate_json(asyncio.run(remixed).model_dump_json())
            else:
                video = litellm.video_remix(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L519-L553
                    input_value["video_id"],
                    input_value["prompt"],
                    timeout=timeout_seconds(input_value),
                    custom_llm_provider=input_value.get("custom_llm_provider"),
                    extra_headers=headers,
                    extra_query=query,
                    extra_body=body,
                    api_base=input_value.get("api_base"),
                    api_key=input_value.get("api_key"),
                    user=input_value.get("user"),
                    mock_response=mock,
                    client=client,
                    litellm_logging_obj=log,
                    litellm_call_id=input_value["litellm_call_id"],
                )
                response = JSON.validate_json(video.model_dump_json())
        else:
            provider = list_provider(input_value)
            if operation == "list" or mock is not None:
                page = litellm.video_list(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L731-L767
                    after=input_value.get("after"),
                    limit=input_value.get("limit"),
                    order=input_value.get("order"),
                    timeout=timeout_seconds(input_value),
                    custom_llm_provider=provider,
                    extra_headers=headers,
                    extra_query=query,
                    api_base=input_value.get("api_base"),
                    api_key=input_value.get("api_key"),
                    user=input_value.get("user"),
                    mock_response=mock,
                    client=client,
                    litellm_logging_obj=log,
                    litellm_call_id=input_value["litellm_call_id"],
                )
                response = page_view(page)
            else:
                listed = litellm.video_list(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/videos/main.py#L731-L767
                    after=input_value.get("after"),
                    limit=input_value.get("limit"),
                    order=input_value.get("order"),
                    timeout=timeout_seconds(input_value),
                    custom_llm_provider=provider,
                    extra_headers=headers,
                    extra_query=query,
                    api_base=input_value.get("api_base"),
                    api_key=input_value.get("api_key"),
                    user=input_value.get("user"),
                    client=client,
                    litellm_logging_obj=log,
                    litellm_call_id=input_value["litellm_call_id"],
                    avideo_list=True,
                    async_call=True,
                )
                response = page_view(asyncio.run(listed))
    except (APIStatusError, APIConnectionError) as caught:
        seen = SeenError.model_validate(caught)
        error = {
            "status": seen.status_code,
            "type": seen.error_type,
            "param": seen.param,
            "code": seen.code,
        }
    extra = input_value.get("extra_headers")
    names = () if extra is None else tuple(name.lower() for name in extra)
    return {
        "request": request_view(responder, ("authorization", *names)),
        "response": response,
        "error": error,
        "logging": logged(log),
    }
