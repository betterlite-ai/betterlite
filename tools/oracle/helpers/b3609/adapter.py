import asyncio
from collections.abc import AsyncGenerator
from typing import cast

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.proxy.pass_through_endpoints.streaming_handler import PassThroughStreamingHandler
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from pydantic import JsonValue

from .fakes import DeferredMarker, ScriptedResponse, StubLogging
from .model import (
    ENDPOINTS,
    FIXED_START,
    JSON,
    PARAMS,
    Driven,
    Input,
    ScriptedReadError,
)
from .recorder import RECORDER
from .seams import install_fakes


async def drive(
    stream: AsyncGenerator[bytes, None], disconnect_after: int | None
) -> list[list[int]]:
    yielded: list[bytes] = []
    while disconnect_after is None or len(yielded) < disconnect_after:
        try:
            chunk = await anext(stream)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L196-L209
        except StopAsyncIteration:
            break
        except ScriptedReadError as raised:
            RECORDER.error = raised
            break
        yielded.append(chunk)
    if disconnect_after is not None and len(yielded) == disconnect_after:
        RECORDER.disconnected = True
        await stream.aclose()  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L261-L263
    return [list(chunk) for chunk in yielded]


def outcome_value() -> str:
    if RECORDER.error is not None:
        return "failed"
    if RECORDER.disconnected:
        return "disconnected"
    return "completed"


def completion_value(logging_obj: StubLogging) -> str | None:
    moment = logging_obj.completion_start_time
    return None if moment is None else moment.isoformat()


def partial_value() -> dict[str, JsonValue] | None:
    partial = RECORDER.partial
    if partial is None:
        return None
    usage = partial.usage
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "response_cost": str(partial.cost),
    }


def bound_value() -> bool | None:
    params = PARAMS.validate_python(RECORDER.params)
    reservation = params["metadata"].get("user_api_key_budget_reservation")
    return None if reservation is None else reservation.get("callback_bound")


def answer(logging_obj: StubLogging, driven: Driven) -> JsonValue:
    routes = RECORDER.routes
    route_bytes: list[list[int]] | None = (
        None if not routes else [list(chunk) for chunk in routes[0]]
    )
    started: list[list[str]] = [[run_id, at.isoformat()] for run_id, at in RECORDER.started]
    payload: dict[str, JsonValue] = {
        "yielded": JSON.validate_python(driven.yielded),
        "outcome": outcome_value(),
        "completion_start_time": completion_value(logging_obj),
        "route_calls": len(routes),
        "route_raw_bytes": JSON.validate_python(route_bytes),
        "queued_jobs": driven.queued_jobs,
        "deferred_jobs": driven.deferred_jobs,
        "failure_dispatches": len(RECORDER.failures),
        "partial_usage": partial_value(),
        "tinyfish_started": JSON.validate_python(started),
        "tinyfish_scans": RECORDER.scans,
        "callback_bound": bound_value(),
    }
    return payload


async def execute(input_value: Input) -> JsonValue:
    litellm.include_cost_in_streaming_usage = input_value["cost_injection"]
    logging_obj = StubLogging(RECORDER.params)
    if input_value["tinyfish_poller"]:
        logging_obj.model_call_details["tinyfish_sse_poller_spawned"] = True
    if input_value["deferred"]:
        logging_obj._on_deferred_stream_complete = DeferredMarker()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L235-L242
    install_fakes()
    failure = input_value["failure"]
    scripted = None if failure is None else ScriptedReadError(failure["message"])
    response = ScriptedResponse(
        [bytes(chunk) for chunk in input_value["chunks"]], scripted, input_value["status_code"]
    )
    stream = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L138-L148
        "AsyncGenerator[bytes, None]",
        PassThroughStreamingHandler.chunk_processor(  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L138-L148
            response,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L139-L148
            input_value["request_body"],
            cast("LiteLLMLoggingObj", logging_obj),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L142
            ENDPOINTS[input_value["endpoint_type"]],
            FIXED_START,
            PassThroughEndpointLogging(),
            input_value["url_route"],
            route_streaming_logging=RECORDER.route,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L147
        ),
    )
    yielded = await drive(stream, input_value["disconnect_after"])
    parked = logging_obj._deferred_stream_complete_args  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L235-L242
    queued_jobs = len(RECORDER.jobs)
    deferred_jobs = 0 if parked is None else len(parked)
    for job in RECORDER.jobs:
        await job
    if parked is not None:
        for job in parked:
            await job
    return answer(logging_obj, Driven(yielded, queued_jobs, deferred_jobs))


def run(input_value: Input) -> JsonValue:
    RECORDER.reset(poller=input_value["tinyfish_poller"], run_id=input_value["run_id"])
    return asyncio.run(execute(input_value))
