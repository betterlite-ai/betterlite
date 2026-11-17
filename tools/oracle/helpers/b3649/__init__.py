import asyncio
import json
import os
from collections.abc import Awaitable
from datetime import datetime
from typing import Final, Literal, NotRequired, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    tinyfish_passthrough_logging_handler as tinyfish,
)
from pydantic import JsonValue, TypeAdapter, ValidationError

from .recorder import LoggedResponse, Recorder, billing_view
from .script import API_BASE_VARIABLE, API_KEY_VARIABLE, Clock, Script, Served, install

litellm.suppress_debug_info = True

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# the route stamps a naive start_time and the poller a naive datetime.now()
START: Final = datetime.fromisoformat("2024-01-01T00:00:00")
END: Final = datetime.fromisoformat("2024-01-01T00:00:05")
CALL_ID: Final = "call-1"
ROUTE: Final = "https://agent.tinyfish.ai/v1/automation/run"
HANDLER: Final = tinyfish.TinyFishPassthroughLoggingHandler
NO_REQUESTS: Final[dict[str, JsonValue]] = {"count": 0, "paths": [], "api_keys": []}


class Run(TypedDict, total=False):
    run_id: str | None
    status: str | None
    num_of_steps: int | None
    result: JsonValue
    error: JsonValue
    type: str | None


class Reply(TypedDict):
    status: int
    body: JsonValue


class HandlerPayload(TypedDict):
    result: dict[str, JsonValue]


class Input(TypedDict):
    function: Literal["blocking", "async_run", "sse_run", "collected_chunks"]
    api_key: str
    run: NotRequired[Run | None]
    result: NotRequired[str | None]
    run_id: NotRequired[str | None]
    replies: NotRequired[list[Reply]]
    lines: NotRequired[list[str]]


def make_logging() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id=CALL_ID,
        function_id="function-1",
    )


def result_text(input_value: Input) -> str:
    explicit = input_value.get("result")
    if explicit is not None:
        return explicit
    submitted = input_value.get("run")
    if submitted is None:
        return ""
    return json.dumps(submitted)


def parsed(text: str) -> JsonValue:
    if not text:
        return None
    try:
        return JSON.validate_json(text)
    except ValidationError:
        return text


def configure(base: str, api_key: str) -> None:
    os.environ[API_BASE_VARIABLE] = base
    os.environ[API_KEY_VARIABLE] = api_key


def blocking_value(input_value: Input) -> dict[str, JsonValue]:
    logging_obj = make_logging()
    payload = cast(
        HandlerPayload,
        HANDLER.tinyfish_passthrough_handler(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/tinyfish_passthrough_logging_handler.py#L143-L174
            httpx_response=httpx.Response(200),
            response_body=input_value.get("run"),
            logging_obj=logging_obj,
            url_route=ROUTE,
            result=result_text(input_value),
            start_time=START,
            end_time=END,
            cache_hit=False,
            request_body={},
        ),
    )
    logged = LoggedResponse.model_validate(payload["result"])
    return billing_view(logging_obj, parsed(logged.response), NO_REQUESTS, cache_hit=None)


async def async_run_value(input_value: Input) -> dict[str, JsonValue]:
    logging_obj = make_logging()
    recorder = Recorder()
    logging_obj.add_dynamic_callback(recorder)
    script = Script(input_value.get("replies"))
    clock = Clock()
    install(clock)
    with Served(script) as base:
        configure(base, input_value["api_key"])
        HANDLER.start_async_run_billing(
            response_body=input_value.get("run"),
            logging_obj=logging_obj,
            result=result_text(input_value),
            start_time=START,
            cache_hit=False,
        )
        for task in clock.tasks:
            await task
    call = recorder.only_call()
    return billing_view(
        logging_obj,
        parsed(call["response"]),
        script.summary(base),
        cache_hit=call["cache_hit"],
    )


async def sse_run_value(input_value: Input) -> dict[str, JsonValue]:
    run_id = input_value.get("run_id")
    if run_id is None:
        raise ValueError("the sse_run function needs a run_id")
    logging_obj = make_logging()
    recorder = Recorder()
    logging_obj.add_dynamic_callback(recorder)
    script = Script(input_value.get("replies"))
    clock = Clock()
    install(clock)
    with Served(script) as base:
        configure(base, input_value["api_key"])
        HANDLER.start_sse_run_billing(
            run_id=run_id,
            litellm_logging_obj=logging_obj,
            start_time=START,
        )
        for task in clock.tasks:
            await task
    call = recorder.only_call()
    return billing_view(
        logging_obj,
        parsed(call["response"]),
        script.summary(base),
        cache_hit=call["cache_hit"],
    )


async def collected_value(input_value: Input) -> dict[str, JsonValue]:
    logging_obj = make_logging()
    script = Script(input_value.get("replies"))
    lines = input_value.get("lines")
    with Served(script) as base:
        configure(base, input_value["api_key"])
        payload = await cast(
            Awaitable[HandlerPayload],
            HANDLER.handle_logging_tinyfish_collected_chunks(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/tinyfish_passthrough_logging_handler.py#L333-L364
                litellm_logging_obj=logging_obj,
                url_route=ROUTE,
                start_time=START,
                all_chunks=[] if lines is None else lines,
                end_time=END,
            ),
        )
    logged = LoggedResponse.model_validate(payload["result"])
    return billing_view(
        logging_obj, parsed(logged.response), script.summary(base), cache_hit=None
    )


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "blocking":
            return blocking_value(input_value)
        case "async_run":
            return asyncio.run(async_run_value(input_value))
        case "sse_run":
            return asyncio.run(sse_run_value(input_value))
        case "collected_chunks":
            return asyncio.run(collected_value(input_value))
