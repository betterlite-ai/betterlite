import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Final

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.anthropic.chat.handler import ModelResponseIterator
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .wire import (
    COMPLETION,
    HANDLER,
    AsyncGuard,
    CallArguments,
    Client,
    ErrorView,
    Failure,
    Input,
    Opened,
    OptionsView,
    Raiser,
    SyncUpstream,
    Upstream,
    WrapperView,
)

TIMEOUT: Final[float] = 1.5
OPENED_STATUS: Final[int] = 200
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

def _text(body: bytes) -> str:
    try:
        return body.decode("utf-8")
    except UnicodeDecodeError:
        return f"{len(body)} bytes that are not text"

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

def _picked(headers: httpx.Headers, names: list[str]) -> JsonValue:
    picked: dict[str, str | None] = {name: headers.get(name) for name in names}
    return JSON.validate_python(picked)


def _request_value(request: httpx.Request, names: list[str]) -> dict[str, JsonValue]:
    return {
        "url": str(request.url),
        "body": _text(request.content),
        "request_headers": _picked(request.headers, names),
    }

def _options(iterator: ModelResponseIterator) -> dict[str, JsonValue]:
    view = OptionsView.model_validate(iterator)
    return {
        "json_mode": view.json_mode,
        "speed": view.speed,
        "tool_name_reverse_map": JSON.validate_python(view.tool_name_reverse_map),
    }

def _opened_value(
    input_value: Input, request: httpx.Request, opened: Opened
) -> dict[str, JsonValue]:
    picked = _picked(opened[1], input_value["response_header_names"])
    post = _request_value(request, input_value["request_header_names"])
    return {"post": post, "stream_options": _options(opened[0]), "response_headers": picked}

def _arguments(input_value: Input, client: Client) -> CallArguments:
    return CallArguments(
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

async def _async_call(input_value: Input, client: Client) -> Opened:
    arguments = _arguments(input_value, client)
    return await HANDLER.make_call(**arguments)

def _sync_call(input_value: Input, client: Client) -> Opened:
    arguments = _arguments(input_value, client)
    return HANDLER.make_sync_call(**arguments)

async def _threaded_call(input_value: Input, client: Client) -> Opened:
    loop = asyncio.get_running_loop()
    return await loop.run_in_executor(None, _sync_call, input_value, client)

async def _failed(call: Awaitable[Opened]) -> BaseException:
    task = asyncio.ensure_future(call)
    await asyncio.wait({task})
    failure = task.exception()
    if failure is None:
        raise ValueError("the call opened a stream")
    return failure

def _error(input_value: Input, failure: BaseException, *, status: bool) -> dict[str, JsonValue]:
    view = ErrorView.model_validate(failure)
    name = type(failure).__name__
    message = _text(view.message)
    if not status:
        if name == "Timeout":
            return {"class": name, "status": view.status_code}
        return {"class": name, "status": view.status_code, "message": message}
    headers = httpx.Headers(view.headers)
    return {
        "status": view.status_code,
        "message": message,
        "headers": _picked(headers, input_value["response_header_names"]),
    }

def _status_call(input_value: Input, upstream: Upstream, *, synchronous: bool) -> Awaitable[Opened]:
    if synchronous:
        return _threaded_call(input_value, upstream)
    return _async_call(input_value, AsyncHTTPHandler(transport=httpx.MockTransport(upstream)))

def _failure_call(input_value: Input, failure: Failure, *, synchronous: bool) -> Awaitable[Opened]:
    transport = httpx.MockTransport(Raiser(failure, input_value["failure_message"]))
    if synchronous:
        return _threaded_call(input_value, HTTPHandler(client=httpx.Client(transport=transport)))
    return _async_call(input_value, AsyncHTTPHandler(transport=transport))

def _status_error_call(
    input_value: Input, upstream: Upstream, *, synchronous: bool
) -> dict[str, JsonValue]:
    call = _status_call(input_value, upstream, synchronous=synchronous)
    return _error(input_value, asyncio.run(_failed(call)), status=True)

def _failure(input_value: Input, failure: Failure, *, synchronous: bool) -> dict[str, JsonValue]:
    call = _failure_call(input_value, failure, synchronous=synchronous)
    return _error(input_value, asyncio.run(_failed(call)), status=False)

def _upstream_status(input_value: Input, fallback: int) -> int:
    requested = input_value["upstream_status"]
    return fallback if requested is None else requested

async def _completion(
    input_value: Input, client: AsyncHTTPHandler, data: dict[str, JsonValue]
) -> CustomStreamWrapper:
    return await COMPLETION().acompletion_stream_function(
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
        headers=input_value["headers"],
    )

def _stream_post(input_value: Input) -> dict[str, JsonValue]:
    asynchronous = Upstream(input_value, OPENED_STATUS)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(asynchronous))
    opened = asyncio.run(_async_call(input_value, client))
    synchronous = Upstream(input_value, OPENED_STATUS)
    sync_client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(synchronous)))
    sync_opened = asyncio.run(_threaded_call(input_value, sync_client))
    return {
        "async": _opened_value(input_value, asynchronous.requests[0], opened),
        "sync": _opened_value(input_value, synchronous.requests[0], sync_opened),
    }

def _handshake_error(input_value: Input) -> dict[str, JsonValue]:
    status = _upstream_status(input_value, 400)
    return {
        "async": _status_error_call(input_value, Upstream(input_value, status), synchronous=False),
        "sync": _status_error_call(
            input_value, SyncUpstream(input_value, status), synchronous=True
        ),
    }

def _non_http_error(input_value: Input) -> dict[str, JsonValue]:
    return {
        "async_unknown": _failure(input_value, "unknown", synchronous=False),
        "async_known": _failure(input_value, "timeout", synchronous=False),
        "sync_unknown": _failure(input_value, "unknown", synchronous=True),
        "sync_known": _failure(input_value, "timeout", synchronous=True),
    }

def _sync_status_guard(input_value: Input) -> dict[str, JsonValue]:
    status = _upstream_status(input_value, 529)
    guard = AsyncGuard(input_value, status)
    iterator, _headers = asyncio.run(_async_call(input_value, guard))
    rejected = _status_error_call(input_value, SyncUpstream(input_value, status), synchronous=True)
    return {
        "async": {
            "opened": True,
            "response_read": guard.responses[0].is_stream_consumed,
            "stream_options": _options(iterator),
        },
        "sync": rejected,
    }

def _stream_flag(input_value: Input) -> dict[str, JsonValue]:
    upstream = Upstream(input_value, OPENED_STATUS)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(upstream))
    data: dict[str, JsonValue] = dict(input_value["request_data"])
    wrapper = asyncio.run(_completion(input_value, client, data))
    view = WrapperView.model_validate(wrapper)
    return {
        "wrapper": {"model": view.model, "custom_llm_provider": view.custom_llm_provider},
        "request": _request_value(upstream.requests[0], input_value["request_header_names"]),
        "stream_options": _options(view.completion_stream),
    }

def _response_headers(input_value: Input) -> dict[str, JsonValue]:
    upstream = Upstream(input_value, OPENED_STATUS)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(upstream))
    _iterator, headers = asyncio.run(_async_call(input_value, client))
    return {
        "post": _request_value(upstream.requests[0], input_value["request_header_names"]),
        "raw": _picked(headers, input_value["response_header_names"]),
        "projected": JSON.validate_python(HANDLER.process_anthropic_headers(headers)),
    }

SCENARIOS: Final[dict[str, Callable[[Input], dict[str, JsonValue]]]] = {
    "stream_post": _stream_post,
    "handshake_error": _handshake_error,
    "non_http_error_500": _non_http_error,
    "sync_status_guard": _sync_status_guard,
    "stream_flag_and_context": _stream_flag,
    "response_headers": _response_headers,
}

def run(input_value: Input) -> JsonValue:
    scenario = SCENARIOS[input_value["scenario"]]
    return JSON.validate_python(scenario(input_value))
