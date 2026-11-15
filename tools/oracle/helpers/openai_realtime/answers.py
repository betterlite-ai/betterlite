import asyncio
import sys
from datetime import UTC, datetime

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.realtime.handler import OpenAIRealtime
from litellm.llms.openai.realtime.http_transformation import OpenAIRealtimeHTTPConfig
from litellm.types.realtime import RealtimeQueryParams
from pydantic import BaseModel, ConfigDict, JsonValue

from .probe import REAL_WEBSOCKETS, FakeWebsockets, ProbeServer
from .types import (
    HEADERS,
    JSON,
    QUERY,
    CloseFrame,
    FailMode,
    FailureSetup,
    HttpSetup,
    Input,
    QuerySetup,
    RealtimeCall,
    RealtimeHttpConfig,
    RealtimeRunner,
)

CONFIG: RealtimeHttpConfig = OpenAIRealtimeHTTPConfig()
RUNNER: RealtimeRunner = OpenAIRealtime()


class PreCallDetails(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    additional_args: dict[str, JsonValue]


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: PreCallDetails


def header_pairs(headers: dict[str, str] | None) -> list[JsonValue]:
    if headers is None:
        return []
    pairs: list[JsonValue] = [[name, value] for name, value in sorted(headers.items())]
    return pairs


def frame_json(frame: CloseFrame | None) -> JsonValue:
    if frame is None:
        return None
    payload: dict[str, JsonValue] = {"code": frame["code"], "reason": frame["reason"]}
    return JSON.validate_python(payload)


def query_argument(value: dict[str, str | None] | None) -> RealtimeQueryParams | None:
    if value is None:
        return None
    return QUERY.validate_python(value)


def pre_call_view(logging_obj: Logging) -> JsonValue:
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    additional = details.additional_args
    headers = HEADERS.validate_python(additional["headers"])
    body = HEADERS.validate_python(additional["complete_input_dict"])
    names = sorted(headers)
    payload: dict[str, JsonValue] = {
        "api_base": additional["api_base"],
        "header_names": JSON.validate_python(names),
        "openai_beta": headers.get("OpenAI-Beta"),
        "query_params": body["query_params"],
    }
    return JSON.validate_python(payload)


async def probe_call(
    input_value: Input,
    server: ProbeServer,
    api_base: str | None,
    query_params: RealtimeQueryParams | None,
) -> Logging:
    logging_obj = Logging(
        model=input_value["model"],
        messages=None,
        stream=False,
        call_type="realtime",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )
    sys.modules["websockets"] = FakeWebsockets(server)
    try:
        call: RealtimeCall = {
            "model": input_value["model"],
            "websocket": server.client,
            "logging_obj": logging_obj,
            "api_base": api_base,
            "api_key": input_value["api_key"],
            "timeout": None,
            "query_params": query_params,
            "litellm_metadata": None,
        }
        await RUNNER.async_realtime(**call)
    finally:
        sys.modules["websockets"] = REAL_WEBSOCKETS
    return logging_obj


async def connect_one(input_value: Input, setup: QuerySetup) -> JsonValue:
    server = ProbeServer(input_value["scope_headers"], None, "error")
    logging_obj = await probe_call(
        input_value, server, setup["api_base"], query_argument(setup["query_params"])
    )
    payload: dict[str, JsonValue] = {
        "url": server.url,
        "headers": header_pairs(server.headers),
        "max_size": server.max_size,
        "ssl": server.ssl_kind,
        "pre_call": pre_call_view(logging_obj),
    }
    return JSON.validate_python(payload)


async def failure_probe(input_value: Input, failure: FailureSetup) -> JsonValue:
    mode: FailMode = "invalid_status" if input_value["kind"] == "handshake" else "error"
    server = ProbeServer(input_value["scope_headers"], failure["close_error"], mode)
    server.failure = failure
    await probe_call(input_value, server, None, None)
    payload: dict[str, JsonValue] = {
        "sent": [JSON.validate_json(text) for text in server.client.sent],
        "closed": frame_json(server.client.closed),
        "close_raised": server.client.close_raised,
    }
    return JSON.validate_python(payload)


def connect_answer(input_value: Input) -> JsonValue:
    connects = [
        asyncio.run(connect_one(input_value, setup)) for setup in input_value["queries"]
    ]
    return {"connects": connects}


def failure_answer(input_value: Input) -> JsonValue:
    failures = [
        asyncio.run(failure_probe(input_value, failure))
        for failure in input_value["failures"]
    ]
    return {"failures": failures}


def http_view(model: str, setup: HttpSetup) -> JsonValue:
    base = CONFIG.get_api_base(setup["api_base"])
    key = CONFIG.get_api_key(setup["api_key"])
    headers = HEADERS.validate_python(
        CONFIG.validate_environment(dict(setup["headers"]), model, key)
    )
    payload: dict[str, JsonValue] = {
        "api_base": base,
        "api_key": key,
        "complete_url": CONFIG.get_complete_url(base, model, setup["api_version"]),
        "calls_url": CONFIG.get_realtime_calls_url(base, model, setup["api_version"]),
        "transcription_url": CONFIG.get_transcription_session_url(
            base, model, setup["api_version"]
        ),
        "headers": headers,
    }
    return JSON.validate_python(payload)


def http_setup(model: str, setup: HttpSetup) -> JsonValue:
    saved = (litellm.api_base, litellm.api_key, litellm.openai_key)
    litellm.api_base, litellm.api_key, litellm.openai_key = (
        setup["litellm_api_base"],
        setup["litellm_api_key"],
        setup["litellm_openai_key"],
    )
    try:
        return http_view(model, setup)
    finally:
        litellm.api_base, litellm.api_key, litellm.openai_key = saved


def http_answer(input_value: Input) -> JsonValue:
    http = [http_setup(input_value["model"], setup) for setup in input_value["http_setups"]]
    return {"http": http}


def run(input_value: Input) -> JsonValue:
    kind = input_value["kind"]
    if kind == "http":
        return http_answer(input_value)
    if kind == "connect":
        return connect_answer(input_value)
    return failure_answer(input_value)
