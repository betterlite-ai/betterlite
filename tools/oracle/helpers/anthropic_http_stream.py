import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.anthropic.chat import handler as anthropic_handler
from litellm.llms.anthropic.chat.handler import AnthropicChatCompletion, ModelResponseIterator
from litellm.llms.anthropic.common_utils import AnthropicError
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

TIMEOUT: Final[float] = 1.5
OPENED_STATUS: Final[int] = 200
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Failure = Literal["unknown", "timeout"]


class Input(TypedDict):
    scenario: str
    api_base: str
    messages: list[dict[str, JsonValue]]
    model: str
    json_mode: bool
    headers: dict[str, str]
    request_header_names: list[str]
    response_header_names: list[str]
    data: str
    request_data: dict[str, JsonValue]
    speed: str | None
    reverse_map: dict[str, str]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    upstream_status: int | None
    upstream_headers: dict[str, str]
    upstream_lines: list[str]
    upstream_body: str | None
    failure_message: str | None


@runtime_checkable
class Handlers(Protocol):
    make_call: Callable[..., Awaitable[tuple[ModelResponseIterator, httpx.Headers]]]
    make_sync_call: Callable[..., tuple[ModelResponseIterator, httpx.Headers]]
    process_anthropic_headers: Callable[..., dict[str, str]]


@runtime_checkable
class CompletionHandlers(Protocol):
    acompletion_stream_function: Callable[..., Awaitable[CustomStreamWrapper]]


CALLS: TypeAdapter[Handlers] = TypeAdapter(Handlers, config=ConfigDict(arbitrary_types_allowed=True))
HANDLER: Handlers = CALLS.validate_python(anthropic_handler)
COMPLETION: Callable[[], CompletionHandlers] = TypeAdapter(Callable[[], CompletionHandlers]).validate_python(
    AnthropicChatCompletion
)


class ErrorView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    status_code: int
    message: bytes
    headers: httpx.Headers | None


class WrapperView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    model: str
    custom_llm_provider: str | None
    completion_stream: ModelResponseIterator


class Upstream:
    def __init__(self, input_value: Input, status: int) -> None:
        self.status = status
        self.headers = input_value["upstream_headers"]
        body = input_value["upstream_body"]
        self.body = "\n".join(input_value["upstream_lines"]).encode() if body is None else body.encode()
        self.requests: list[httpx.Request] = []
        self.responses: list[httpx.Response] = []

    def reply(self) -> httpx.Response:
        response = httpx.Response(self.status, headers=self.headers, stream=httpx.ByteStream(self.body))
        self.responses.append(response)
        return response

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return self.reply()


class SyncUpstream(Upstream):
    def post(self, url: str, **options: JsonValue) -> httpx.Response:
        del url, options
        return self.reply()


class AsyncGuard(Upstream):
    async def post(self, url: str, **options: JsonValue) -> httpx.Response:
        del url, options
        return self.reply()


class Raiser:
    def __init__(self, failure: Failure, message: str | None) -> None:
        self.failure = failure
        self.message = "the transport exploded" if message is None else message

    def __call__(self, request: httpx.Request) -> httpx.Response:
        del request
        if self.failure == "timeout":
            raise httpx.ReadTimeout(self.message)
        raise RuntimeError(self.message)


def _logging(messages: list[dict[str, JsonValue]]) -> Logging:
    return Logging(
        model="claude-3-5-sonnet",
        messages=messages,
        stream=True,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def _text(body: bytes) -> str:
    try:
        return body.decode("utf-8")
    except UnicodeDecodeError:
        return f"{len(body)} bytes that are not text"


def _request_value(request: httpx.Request, names: list[str]) -> dict[str, JsonValue]:
    return {
        "url": str(request.url),
        "body": _text(request.content),
        "request_headers": JSON.validate_python({name: request.headers.get(name) for name in names}),
    }


def _options(iterator: ModelResponseIterator) -> dict[str, JsonValue]:
    return {
        "json_mode": iterator.json_mode,
        "speed": iterator.speed,
        "tool_name_reverse_map": JSON.validate_python(iterator.tool_name_reverse_map),
    }


def _opened(
    input_value: Input,
    request: httpx.Request,
    iterator: ModelResponseIterator,
    headers: httpx.Headers,
) -> dict[str, JsonValue]:
    return {
        "post": _request_value(request, input_value["request_header_names"]),
        "stream_options": _options(iterator),
        "response_headers": JSON.validate_python(
            {name: headers.get(name) for name in input_value["response_header_names"]}
        ),
    }


async def _call_async(
    input_value: Input,
    client: AsyncHTTPHandler | AsyncGuard,
) -> tuple[ModelResponseIterator, httpx.Headers]:
    return await HANDLER.make_call(
        client=client,
        api_base=input_value["api_base"],
        headers=input_value["headers"],
        data=input_value["data"],
        model=input_value["model"],
        messages=input_value["messages"],
        logging_obj=_logging(input_value["messages"]),
        timeout=TIMEOUT,
        json_mode=input_value["json_mode"],
        speed=input_value["speed"],
        tool_name_reverse_map=input_value["reverse_map"],
    )


def _call_sync(
    input_value: Input,
    client: HTTPHandler | Upstream,
) -> tuple[ModelResponseIterator, httpx.Headers]:
    return HANDLER.make_sync_call(
        client=client,
        api_base=input_value["api_base"],
        headers=input_value["headers"],
        data=input_value["data"],
        model=input_value["model"],
        messages=input_value["messages"],
        logging_obj=_logging(input_value["messages"]),
        timeout=TIMEOUT,
        json_mode=input_value["json_mode"],
        speed=input_value["speed"],
        tool_name_reverse_map=input_value["reverse_map"],
    )


def _status_error_call(input_value: Input, upstream: Upstream, *, synchronous: bool) -> dict[str, JsonValue]:
    try:
        if synchronous:
            _call_sync(input_value, upstream)
        else:
            asyncio.run(_call_async(input_value, AsyncHTTPHandler(transport=httpx.MockTransport(upstream))))
    except AnthropicError as error:
        view = ErrorView.model_validate(error)
        headers = httpx.Headers() if view.headers is None else view.headers
        return {
            "status": view.status_code,
            "message": _text(view.message),
            "headers": JSON.validate_python(
                {name: headers.get(name) for name in input_value["response_header_names"]}
            ),
        }
    raise ValueError("the call opened a stream")


def _failure(input_value: Input, failure: Failure, *, synchronous: bool) -> dict[str, JsonValue]:
    transport = httpx.MockTransport(Raiser(failure, input_value["failure_message"]))
    try:
        if synchronous:
            _call_sync(input_value, HTTPHandler(client=httpx.Client(transport=transport)))
        else:
            asyncio.run(_call_async(input_value, AsyncHTTPHandler(transport=transport)))
    except litellm.Timeout as timeout:
        return {"class": "Timeout", "status": timeout.status_code}
    except AnthropicError as error:
        return {"class": "AnthropicError", "status": error.status_code, "message": error.message}
    raise ValueError("the call opened a stream")


def _stream_post(input_value: Input) -> JsonValue:
    asynchronous = Upstream(input_value, OPENED_STATUS)
    iterator, headers = asyncio.run(
        _call_async(input_value, AsyncHTTPHandler(transport=httpx.MockTransport(asynchronous)))
    )
    synchronous = Upstream(input_value, OPENED_STATUS)
    sync_client = httpx.Client(transport=httpx.MockTransport(synchronous))
    sync_iterator, sync_headers = _call_sync(input_value, HTTPHandler(client=sync_client))
    answer: dict[str, JsonValue] = {
        "async": _opened(input_value, asynchronous.requests[0], iterator, headers),
        "sync": _opened(input_value, synchronous.requests[0], sync_iterator, sync_headers),
    }
    return JSON.validate_python(answer)


def _handshake_error(input_value: Input) -> JsonValue:
    requested_status = input_value["upstream_status"]
    status = 400 if requested_status is None else requested_status
    answer: dict[str, JsonValue] = {
        "async": _status_error_call(input_value, Upstream(input_value, status), synchronous=False),
        "sync": _status_error_call(input_value, SyncUpstream(input_value, status), synchronous=True),
    }
    return JSON.validate_python(answer)


def _non_http_error(input_value: Input) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "async_unknown": _failure(input_value, "unknown", synchronous=False),
        "async_known": _failure(input_value, "timeout", synchronous=False),
        "sync_unknown": _failure(input_value, "unknown", synchronous=True),
        "sync_known": _failure(input_value, "timeout", synchronous=True),
    }
    return JSON.validate_python(answer)


def _sync_status_guard(input_value: Input) -> JsonValue:
    requested_status = input_value["upstream_status"]
    guard_status = 529 if requested_status is None else requested_status
    guard = AsyncGuard(input_value, guard_status)
    iterator, _ = asyncio.run(_call_async(input_value, guard))
    sync_upstream = SyncUpstream(input_value, guard_status)
    answer: dict[str, JsonValue] = {
        "async": {
            "opened": True,
            "response_read": guard.responses[0].is_stream_consumed,
            "stream_options": _options(iterator),
        },
        "sync": _status_error_call(input_value, sync_upstream, synchronous=True),
    }
    return JSON.validate_python(answer)


def _stream_flag(input_value: Input) -> JsonValue:
    upstream = Upstream(input_value, OPENED_STATUS)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(upstream))
    data: dict[str, JsonValue] = dict(input_value["request_data"])
    wrapper = asyncio.run(
        COMPLETION().acompletion_stream_function(
            model=input_value["model"],
            messages=input_value["messages"],
            api_base=input_value["api_base"],
            custom_prompt_dict={},
            model_response=ModelResponse(),
            print_verbose=None,
            timeout=TIMEOUT,
            client=client,
            encoding=None,
            api_key="",
            logging_obj=_logging(input_value["messages"]),
            stream=True,
            _is_function_call=False,
            data=data,
            json_mode=input_value["json_mode"],
            optional_params=input_value["optional_params"],
            litellm_params=input_value["litellm_params"],
            logger_fn=None,
            headers=input_value["headers"],
        )
    )
    view = WrapperView.model_validate(wrapper)
    answer: dict[str, JsonValue] = {
        "wrapper": {"model": view.model, "custom_llm_provider": view.custom_llm_provider},
        "request": _request_value(upstream.requests[0], input_value["request_header_names"]),
        "stream_options": _options(view.completion_stream),
    }
    return JSON.validate_python(answer)


def _response_headers(input_value: Input) -> JsonValue:
    upstream = Upstream(input_value, OPENED_STATUS)
    _, headers = asyncio.run(_call_async(input_value, AsyncHTTPHandler(transport=httpx.MockTransport(upstream))))
    answer: dict[str, JsonValue] = {
        "post": _request_value(upstream.requests[0], input_value["request_header_names"]),
        "raw": JSON.validate_python(
            {name: headers.get(name) for name in input_value["response_header_names"]}
        ),
        "projected": JSON.validate_python(HANDLER.process_anthropic_headers(headers)),
    }
    return JSON.validate_python(answer)


SCENARIOS: Final[dict[str, Callable[[Input], JsonValue]]] = {
    "stream_post": _stream_post,
    "handshake_error": _handshake_error,
    "non_http_error_500": _non_http_error,
    "sync_status_guard": _sync_status_guard,
    "stream_flag_and_context": _stream_flag,
    "response_headers": _response_headers,
}


def run(input_value: Input) -> JsonValue:
    scenario = SCENARIOS[input_value["scenario"]]
    return scenario(input_value)
