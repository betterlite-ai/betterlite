import asyncio
import json
import logging
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable, Callable
from datetime import UTC, datetime
from types import CoroutineType
from typing import Annotated, Literal, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from litellm.llms.anthropic.pass_through.messages.streaming_iterator import (
    GLOBAL_PASS_THROUGH_SUCCESS_HANDLER_OBJ,
    BaseAnthropicMessagesStreamingIterator,
)
from litellm.proxy.pass_through_endpoints import streaming_handler
from litellm.proxy.pass_through_endpoints.streaming_handler import PassThroughStreamingHandler
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json
from typing_extensions import override

TIMEOUT_SECONDS = 120.0
LOGGER_NAME = "LiteLLM Proxy"

Work = CoroutineType[None, None, None]


class SeamCall(TypedDict):
    litellm_logging_obj: Logging
    passthrough_success_handler_obj: PassThroughEndpointLogging
    url_route: str
    request_body: dict[str, Json]
    endpoint_type: EndpointType
    start_time: datetime


class RouteCall(SeamCall):
    raw_bytes: list[bytes]
    end_time: datetime


class ProcessorCall(SeamCall):
    response: httpx.Response


class Seen(TypedDict):
    request_body: dict[str, Json]
    endpoint_anthropic: bool
    url_route: str
    shared_success_handler: bool
    start_is_construction: bool


class Routed(Seen):
    raw_frames: list[str]
    iterator_logging_obj: bool
    first_within_run: bool | None


class Processed(Seen):
    response_status: int
    supplied_logging_obj: bool


class StepOutcome(TypedDict):
    routed: list[Routed]
    processed: list[Processed]
    ran: int
    warnings: list[str]
    callback_args: list[int]
    hook_calls: list[str]
    completion_start_recorded: bool
    callback_armed: bool
    args_present: bool
    text: str | None


class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.routed: list[RouteCall] = []
        self.processed: list[ProcessorCall] = []
        self.warnings: list[str] = []
        self.route_failure: str | None = None
        self.ran = 0

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.warnings.append(record.getMessage().strip())

    async def counted(self) -> None:
        self.ran += 1

    def route(self, **call: Unpack[RouteCall]) -> Work:
        if self.route_failure is not None:
            raise ValueError(self.route_failure)
        self.routed.append(call)
        return self.counted()

    async def chunk_processor(self, **call: Unpack[ProcessorCall]) -> AsyncGenerator[bytes, None]:
        self.processed.append(call)
        async for chunk in call["response"].aiter_bytes():
            yield chunk


# The iterator looks the handler class up by name on each call, so a subclass takes its place
class Reroute(PassThroughStreamingHandler):
    @classmethod
    def install(cls, recorder: Recorder) -> None:
        route: Callable[..., Work] = recorder.route
        processor: Callable[..., AsyncGenerator[bytes, None]] = recorder.chunk_processor
        cls._route_streaming_logging_to_handler = route
        cls.chunk_processor = processor
        streaming_handler.PassThroughStreamingHandler = cls


class RecordedLogging(Logging):
    def __init__(self) -> None:
        super().__init__(
            model="claude",
            messages=None,
            stream=None,
            call_type="anthropic_messages",
            start_time=datetime.now(UTC),
            litellm_call_id="call-1",
            function_id="fn-1",
            litellm_trace_id="trace-1",
        )
        self._on_deferred_stream_complete: Callable[..., Work] | None = None
        self._deferred_stream_complete_args: tuple[Work, ...] | None = None

    def arm_deferred(self, callback: Callable[..., Work]) -> None:
        self._on_deferred_stream_complete = callback

    def park(self, parked: tuple[Work, ...]) -> None:
        self._deferred_stream_complete_args = parked

    def is_armed(self) -> bool:
        return self._on_deferred_stream_complete is not None

    def has_parked(self) -> bool:
        return self._deferred_stream_complete_args is not None

    def set_failure_hook(self, hook: Callable[[Exception], Awaitable[None]] | None) -> None:
        self._on_detached_stream_failure = hook


class FailureHook:
    def __init__(self, calls: list[str], *, raises: bool) -> None:
        self.calls = calls
        self.raises = raises

    async def __call__(self, exc: Exception) -> None:
        self.calls.append(f"{type(exc).__name__}({exc})")
        if self.raises:
            raise ValueError("hook")


# An upstream stream without aclose
class PlainStream(AsyncIterator[bytes]):
    @override
    async def __anext__(self) -> bytes:
        raise StopAsyncIteration


class ClosingStream(PlainStream):
    def __init__(self, *, fails: bool) -> None:
        super().__init__()
        self.fails = fails

    async def aclose(self) -> None:
        if self.fails:
            raise ValueError("closed")


class Session(BaseAnthropicMessagesStreamingIterator):
    def __init__(self, body: dict[str, Json], api_base: str, client: httpx.AsyncClient) -> None:
        self.logging = RecordedLogging()
        super().__init__(litellm_logging_obj=self.logging, request_body=body)
        self.body = body
        self.url = f"{api_base}/messages"
        self.client = client
        self.supplied: RecordedLogging | None = None
        self.recorder = Recorder()
        self.frames: list[bytes] = []
        self.hook_calls: list[str] = []
        self.callback_args: list[int] = []
        self.text: str | None = None

    async def collect_frames(self) -> list[bytes]:
        response = await self.client.post(self.url, json=self.body)
        parts = response.content.split(b"\n\n")
        frames = [part + b"\n\n" for part in parts[:-1]]
        if parts[-1]:
            frames.append(parts[-1])
        return frames

    def on_deferred(self, *parked: Work) -> Work:
        self.callback_args.append(len(parked))
        for coroutine in parked:
            coroutine.close()
        return self.recorder.counted()

    async def bill(self, *, teardown: bool) -> None:
        await self._bill_collected_chunks(self.frames, stream_teardown=teardown)

    def dispatch(self) -> None:
        self._dispatch_pending_deferred_logging()

    async def abort(self, stream: PlainStream) -> None:
        await self._abort_upstream(stream)

    async def fail(self, exc: Exception) -> None:
        await self._fire_detached_failure_hook(exc)

    def convert(self, chunk: dict[str, Json] | bytes) -> bytes:
        return self._convert_chunk_to_sse_format(chunk)

    def relay(self, response: httpx.Response, supplied: RecordedLogging) -> AsyncIterator[bytes]:
        return self.get_async_streaming_response_iterator(response, {"model": "supplied"}, supplied)

    async def run_step(self, step: "Step") -> StepOutcome:
        self.text = None
        await step.run(self)
        await GLOBAL_LOGGING_WORKER.flush()
        return self.observe()

    def seen(self, call: SeamCall) -> Seen:
        return {
            "request_body": call["request_body"],
            "endpoint_anthropic": call["endpoint_type"] == EndpointType.ANTHROPIC,
            "url_route": call["url_route"],
            "shared_success_handler": call["passthrough_success_handler_obj"]
            is GLOBAL_PASS_THROUGH_SUCCESS_HANDLER_OBJ,
            "start_is_construction": call["start_time"] == self.start_time,
        }

    def routed_entry(self, call: RouteCall, first: datetime | None) -> Routed:
        within = None if first is None else call["start_time"] <= first <= call["end_time"]
        return {
            **self.seen(call),
            "raw_frames": [frame.decode(errors="replace") for frame in call["raw_bytes"]],
            "iterator_logging_obj": call["litellm_logging_obj"] is self.logging,
            "first_within_run": within,
        }

    def processed_entry(self, call: ProcessorCall) -> Processed:
        return {
            **self.seen(call),
            "response_status": call["response"].status_code,
            "supplied_logging_obj": call["litellm_logging_obj"] is self.supplied,
        }

    def observe(self) -> StepOutcome:
        first = self.logging.completion_start_time
        recorded = first is not None and "completion_start_time" in self.logging.model_call_details
        outcome: StepOutcome = {
            "routed": [self.routed_entry(call, first) for call in self.recorder.routed],
            "processed": [self.processed_entry(call) for call in self.recorder.processed],
            "ran": self.recorder.ran,
            "warnings": self.recorder.warnings,
            "callback_args": self.callback_args,
            "hook_calls": self.hook_calls,
            "completion_start_recorded": recorded,
            "callback_armed": self.logging.is_armed(),
            "args_present": self.logging.has_parked(),
            "text": self.text,
        }
        self.recorder.routed = []
        self.recorder.processed = []
        self.recorder.warnings = []
        self.recorder.ran = 0
        self.callback_args = []
        self.hook_calls = []
        return outcome


class StepModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


class CollectStep(StepModel):
    op: Literal["collect"]
    first_chunk: bool

    async def run(self, session: Session) -> None:
        session.frames = await session.collect_frames()
        if self.first_chunk:
            now = datetime.now(UTC).astimezone()
            session.completion_start_time = now.replace(tzinfo=None)


class BillStep(StepModel):
    op: Literal["bill"]
    teardown: bool
    route_failure: str | None = None

    async def run(self, session: Session) -> None:
        session.recorder.route_failure = self.route_failure
        await session.bill(teardown=self.teardown)


class ArmDeferredStep(StepModel):
    op: Literal["arm_deferred"]

    async def run(self, session: Session) -> None:
        session.logging.arm_deferred(session.on_deferred)


class ParkArgsStep(StepModel):
    op: Literal["park_args"]
    count: int

    async def run(self, session: Session) -> None:
        session.logging.park(tuple(session.recorder.counted() for _ in range(self.count)))


class DispatchStep(StepModel):
    op: Literal["dispatch"]

    async def run(self, session: Session) -> None:
        session.dispatch()


class AbortUpstreamStep(StepModel):
    op: Literal["abort_upstream"]
    close: Literal["unsupported", "closes", "fails"]

    async def run(self, session: Session) -> None:
        streams = {
            "unsupported": PlainStream(),
            "closes": ClosingStream(fails=False),
            "fails": ClosingStream(fails=True),
        }
        await session.abort(streams[self.close])


class DetachedFailureStep(StepModel):
    op: Literal["detached_failure"]
    hook: Literal["missing", "records", "raises"]

    async def run(self, session: Session) -> None:
        hook: FailureHook | None = None
        if self.hook != "missing":
            hook = FailureHook(session.hook_calls, raises=self.hook == "raises")
        session.logging.set_failure_hook(hook)
        await session.fail(ValueError("original"))


class ConvertStep(StepModel):
    op: Literal["convert"]
    chunk: dict[str, Json] | str

    async def run(self, session: Session) -> None:
        match self.chunk:
            case str(text):
                frame = session.convert(text.encode())
            case dict(mapping):
                frame = session.convert(mapping)
        session.text = frame.decode(errors="replace")


class DelegateStep(StepModel):
    op: Literal["delegate"]

    async def run(self, session: Session) -> None:
        supplied = RecordedLogging()
        session.supplied = supplied
        async with session.client.stream("POST", session.url, json=session.body) as response:
            relayed = session.relay(response, supplied)
            relayed_bytes = b"".join([chunk async for chunk in relayed])
        session.text = relayed_bytes.decode(errors="replace")


Step = Annotated[
    CollectStep
    | BillStep
    | ArmDeferredStep
    | ParkArgsStep
    | DispatchStep
    | AbortUpstreamStep
    | DetachedFailureStep
    | ConvertStep
    | DelegateStep,
    Field(discriminator="op"),
]


class MessagesStreamLoggingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    body: dict[str, Json]
    steps: list[Step]


async def run_steps(call: MessagesStreamLoggingCall, api_base: str) -> list[StepOutcome]:
    async with httpx.AsyncClient(trust_env=False, timeout=TIMEOUT_SECONDS) as client:
        session = Session(call.body, api_base, client)
        logger = logging.getLogger(LOGGER_NAME)
        logger.addHandler(session.recorder)
        logger.setLevel(logging.WARNING)
        Reroute.install(session.recorder)
        outcomes = [await session.run_step(step) for step in call.steps]
        logger.removeHandler(session.recorder)
        await GLOBAL_LOGGING_WORKER.stop()
        return outcomes


def run_messages_stream_logging(call: MessagesStreamLoggingCall, api_base: str) -> str:
    steps = asyncio.run(run_steps(call, api_base))
    outcome = {"messages_stream_logging": {"value": {"steps": steps}}}
    return json.dumps(outcome)
