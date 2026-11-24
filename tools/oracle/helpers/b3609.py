import asyncio
import datetime as dt
from collections.abc import AsyncGenerator, AsyncIterator, Coroutine, Sequence
from typing import Final, Literal, NamedTuple, TypedDict, cast

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import PassThroughEndpointLoggingResultValues as LoggingResult
from litellm.proxy.common_request_processing import ProxyBaseLLMRequestProcessing
from litellm.proxy.pass_through_endpoints import streaming_handler
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    tinyfish_passthrough_logging_handler as tinyfish,
)
from litellm.proxy.pass_through_endpoints.streaming_handler import PassThroughStreamingHandler
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from litellm.types.utils import ModelResponse, Usage
from pydantic import JsonValue, TypeAdapter

FIXED_START: Final = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)
FIXED_STAMP: Final = dt.datetime(2024, 1, 2, tzinfo=dt.UTC)
PROMPT_TOKENS: Final = 10
COMPLETION_TOKENS: Final = 5
TOTAL_TOKENS: Final = 15
RESPONSE_COST: Final = 0.25
SCRIPTED_CLASS: Final = "ScriptedReadError"

EndpointName = Literal[
    "vertex-ai", "gemini", "anthropic", "openai", "tinyfish", "generic", "decisions"
]
ENDPOINTS: Final[dict[EndpointName, EndpointType]] = {
    "vertex-ai": EndpointType.VERTEX_AI,
    "gemini": EndpointType.GEMINI,
    "anthropic": EndpointType.ANTHROPIC,
    "openai": EndpointType.OPENAI,
    "tinyfish": EndpointType.TINYFISH,
    "generic": EndpointType.GENERIC,
    "decisions": EndpointType.DECISIONS,
}


class FailureSpec(TypedDict):
    message: str
    traceback: str


class Input(TypedDict):
    endpoint_type: EndpointName
    chunks: list[list[int]]
    failure: FailureSpec | None
    disconnect_after: int | None
    status_code: int
    cost_injection: bool
    request_body: dict[str, JsonValue] | None
    url_route: str
    deferred: bool
    tinyfish_poller: bool
    run_id: str | None


class Reservation(TypedDict, total=False):
    id: str
    callback_bound: bool


class LoggingParams(TypedDict):
    metadata: dict[str, Reservation]


class UsageAndCost(NamedTuple):
    usage: Usage
    cost: float


class FailureRecord(NamedTuple):
    message: str
    traceback_tail: str


class Driven(NamedTuple):
    yielded: list[list[int]]
    queued_jobs: int
    deferred_jobs: int


class ScriptedReadError(Exception):
    def __init__(self, message: str) -> None:
        super().__init__(message)


class DeferredMarker:
    def __call__(self) -> None: ...


class ScriptedResponse:
    def __init__(self, chunks: list[bytes], failure: ScriptedReadError | None, status: int) -> None:
        self.status_code = status
        self.chunks = chunks
        self.failure = failure

    async def aiter_bytes(self) -> AsyncIterator[bytes]:
        for chunk in self.chunks:
            yield chunk
        if self.failure is not None:
            raise self.failure


class PartialUsageSink:
    def __call__(self, usage: Usage, response_cost: float) -> None:
        RECORDER.partial = UsageAndCost(usage, response_cost)


class StubLogging:
    def __init__(self, params: dict[str, JsonValue]) -> None:
        self.litellm_params: dict[str, JsonValue] = params
        self.model_call_details: dict[str, JsonValue] = {}
        self.completion_start_time: dt.datetime | None = None
        self._on_deferred_stream_complete: DeferredMarker | None = None
        self._deferred_stream_complete_args: tuple[Coroutine[None, None, None], ...] | None = None
        self.record_partial_usage_for_failure = PartialUsageSink()

    def _update_completion_start_time(self, completion_start_time: dt.datetime) -> None:
        self.completion_start_time = FIXED_STAMP
        self.model_call_details["completion_start_time"] = completion_start_time.isoformat()

    async def dispatch_failure_handlers(
        self, exception: Exception, traceback_exception: str, prefer_async_handlers: bool = False
    ) -> None:
        _ = prefer_async_handlers
        lines = traceback_exception.rstrip().splitlines()
        tail = "" if not lines else lines[-1]
        qualified = f"{__name__}.{SCRIPTED_CLASS}"
        record = FailureRecord(str(exception), tail.replace(qualified, SCRIPTED_CLASS))
        RECORDER.failures.append(record)


class Recorder:
    def __init__(self) -> None:
        self.poller = False
        self.run_id: str | None = None
        self.jobs: list[Coroutine[None, None, None]] = []
        self.routes: list[list[bytes]] = []
        self.started: list[tuple[str, dt.datetime]] = []
        self.scans = 0
        self.failures: list[FailureRecord] = []
        self.partial: UsageAndCost | None = None
        self.error: ScriptedReadError | None = None
        self.disconnected = False
        self.params: dict[str, JsonValue] = {}

    def configure(self, input_value: Input) -> None:
        self.poller = input_value["tinyfish_poller"]
        self.run_id = input_value["run_id"]
        self.jobs = []
        self.routes = []
        self.started = []
        self.scans = 0
        self.failures = []
        self.partial = None
        self.error = None
        self.disconnected = False
        self.params = {"metadata": {"user_api_key_budget_reservation": {"id": "reservation-1"}}}

    def __call__(self, async_coroutine: Coroutine[None, None, None]) -> None:
        self.jobs.append(async_coroutine)

    async def route(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L41-L53
        self,
        *,
        litellm_logging_obj: LiteLLMLoggingObj,
        passthrough_success_handler_obj: PassThroughEndpointLogging,
        url_route: str,
        request_body: dict[str, JsonValue],
        endpoint_type: EndpointType,
        start_time: dt.datetime,
        raw_bytes: list[bytes],
        end_time: dt.datetime,
    ) -> None:
        _ = (
            litellm_logging_obj,
            passthrough_success_handler_obj,
            url_route,
            request_body,
            endpoint_type,
            start_time,
            end_time,
        )
        self.routes.append(list(raw_bytes))


RECORDER: Final = Recorder()
PARAMS: Final = TypeAdapter(LoggingParams)
JSON: Final = TypeAdapter(JsonValue)


def start_billing(
    run_id: str,
    litellm_logging_obj: LiteLLMLoggingObj,
    start_time: dt.datetime,
    client: AsyncHTTPHandler | None = None,
) -> None:
    _ = client
    RECORDER.started.append((run_id, start_time))
    if RECORDER.poller:
        tinyfish.mark_sse_poller_spawned(litellm_logging_obj)


def run_id_from_frames(frames: bytes) -> str | None:
    _ = frames
    RECORDER.scans += 1
    return RECORDER.run_id


def process_chunk(
    chunk: bytes, model_name: str, litellm_logging_obj: LiteLLMLoggingObj | None = None
) -> bytes:
    _ = (model_name, litellm_logging_obj)
    return b"injected:" + chunk


def build_result(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L377-L387
    litellm_logging_obj: LiteLLMLoggingObj,
    passthrough_success_handler_obj: PassThroughEndpointLogging,
    url_route: str,
    request_body: dict[str, JsonValue],
    endpoint_type: EndpointType,
    start_time: dt.datetime,
    raw_bytes: Sequence[bytes],
    end_time: dt.datetime,
    model: str | None,
) -> tuple[LoggingResult, dict[str, float]]:
    _ = (
        litellm_logging_obj,
        passthrough_success_handler_obj,
        url_route,
        request_body,
        endpoint_type,
        start_time,
        raw_bytes,
        end_time,
        model,
    )
    usage = Usage(
        prompt_tokens=PROMPT_TOKENS,
        completion_tokens=COMPLETION_TOKENS,
        total_tokens=TOTAL_TOKENS,
    )
    return ModelResponse(usage=usage), {"response_cost": RESPONSE_COST}


def patch() -> None:
    GLOBAL_LOGGING_WORKER.ensure_initialized_and_enqueue = cast("object", RECORDER)  # type: ignore[assignment, method-assign]  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/logging_worker.py#L448-L453
    ProxyBaseLLMRequestProcessing._process_chunk_with_cost_injection = cast("object", process_chunk)  # type: ignore[assignment, method-assign]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L4068-L4071
    tinyfish.TinyFishPassthroughLoggingHandler.start_sse_run_billing = start_billing  # type: ignore[method-assign]  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/tinyfish_passthrough_logging_handler.py#L203-L223
    PassThroughStreamingHandler._build_passthrough_logging_result = build_result  # type: ignore[method-assign]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L377-L387
    streaming_handler.run_id_from_sse_frames = run_id_from_frames


async def drive(
    stream: AsyncGenerator[bytes, None], disconnect_after: int | None
) -> list[list[int]]:
    yielded: list[bytes] = []
    while disconnect_after is None or len(yielded) < disconnect_after:
        try:
            chunk = await anext(stream)
        except StopAsyncIteration:
            break
        except ScriptedReadError as raised:
            RECORDER.error = raised
            break
        yielded.append(chunk)
    if disconnect_after is not None and len(yielded) == disconnect_after:
        RECORDER.disconnected = True
        await stream.aclose()
    return [list(chunk) for chunk in yielded]


def outcome_value() -> str:
    if RECORDER.error is not None:
        return "failed"
    if RECORDER.disconnected:
        return "disconnected"
    return "completed"


def error_value() -> str | None:
    error = RECORDER.error
    return None if error is None else f"{SCRIPTED_CLASS}: {error}"


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
    failures = [
        {
            "class": SCRIPTED_CLASS,
            "message": record.message,
            "traceback_tail": record.traceback_tail,
        }
        for record in RECORDER.failures
    ]
    routes = RECORDER.routes
    return JSON.validate_python(
        {
            "yielded": driven.yielded,
            "outcome": outcome_value(),
            "error": error_value(),
            "completion_start_time": completion_value(logging_obj),
            "route_calls": len(routes),
            "route_raw_bytes": None if not routes else [list(chunk) for chunk in routes[0]],
            "queued_jobs": driven.queued_jobs,
            "deferred_jobs": driven.deferred_jobs,
            "failures": failures,
            "partial_usage": partial_value(),
            "tinyfish_started": [[run_id, at.isoformat()] for run_id, at in RECORDER.started],
            "tinyfish_scans": RECORDER.scans,
            "callback_bound": bound_value(),
        }
    )


async def execute(input_value: Input) -> JsonValue:
    litellm.include_cost_in_streaming_usage = input_value["cost_injection"]
    logging_obj = StubLogging(RECORDER.params)
    if input_value["deferred"]:
        logging_obj._on_deferred_stream_complete = DeferredMarker()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L235-L242
    patch()
    failure = input_value["failure"]
    scripted = None if failure is None else ScriptedReadError(failure["message"])
    response = ScriptedResponse(
        [bytes(chunk) for chunk in input_value["chunks"]], scripted, input_value["status_code"]
    )
    stream = cast(
        "AsyncGenerator[bytes, None]",
        PassThroughStreamingHandler.chunk_processor(  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L138-L148
            response,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L139-L148
            input_value["request_body"],
            cast("LiteLLMLoggingObj", logging_obj),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L142
            ENDPOINTS[input_value["endpoint_type"]],
            FIXED_START,
            PassThroughEndpointLogging(),
            input_value["url_route"],
            route_streaming_logging=RECORDER.route,  # pyright: ignore[reportArgumentType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L147
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
    RECORDER.configure(input_value)
    return asyncio.run(execute(input_value))
