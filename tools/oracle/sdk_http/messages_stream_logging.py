import asyncio
import json
import logging
from collections.abc import AsyncIterator, Awaitable, Callable, Coroutine
from datetime import UTC, datetime
from typing import Annotated, Literal, Protocol, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from litellm.llms.anthropic.pass_through.messages import streaming_iterator
from litellm.proxy.pass_through_endpoints import streaming_handler
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json
from typing_extensions import override

Work = Coroutine[object, object, None]
Hook = Callable[[Exception], Awaitable[None]]

# route logging receives the raw chunks, direct delegation receives the live response
RouteCall = TypedDict("RouteCall", {  # noqa: UP013  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L278-L374
    "litellm_logging_obj": Logging, "passthrough_success_handler_obj": PassThroughEndpointLogging,
    "url_route": str, "request_body": dict[str, Json], "endpoint_type": EndpointType,
    "start_time": datetime, "raw_bytes": list[bytes], "end_time": datetime,
})

ProcessorCall = TypedDict("ProcessorCall", {  # noqa: UP013  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L139-L276
    "litellm_logging_obj": Logging, "passthrough_success_handler_obj": PassThroughEndpointLogging,
    "url_route": str, "request_body": dict[str, Json] | None, "endpoint_type": EndpointType,
    "start_time": datetime, "response": httpx.Response,
})

class DeferredCallback(Protocol):
    def __call__(self, *parked: Work) -> Work: ...

class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.routed: list[RouteCall] = []
        self.processed: list[ProcessorCall] = []
        self.warnings: list[Json] = []
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
    async def chunk_processor(self, **call: Unpack[ProcessorCall]) -> AsyncIterator[bytes]:
        self.processed.append(call)
        async for chunk in call["response"].aiter_bytes():
            yield chunk

# the iterator looks the handler class up by name on each call
def install(recorder: Recorder) -> None:
    handler = streaming_handler.PassThroughStreamingHandler
    handler.chunk_processor = recorder.chunk_processor  # type: ignore[method-assign, assignment]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L139-L276
    handler._route_streaming_logging_to_handler = recorder.route  # type: ignore[method-assign, assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L278-L374

class RecordedLogging(Logging):
    def __init__(self) -> None:
        details: dict[str, Json] = {}
        self.completion_start_time: datetime | None = None
        self.model_call_details = details
        self.details = details
        self._on_deferred_stream_complete: DeferredCallback | None = None
        self._deferred_stream_complete_args: tuple[Work, ...] | None = None
        self._on_detached_stream_failure: Hook | None = None
    def arm_deferred(self, callback: DeferredCallback) -> None:
        self._on_deferred_stream_complete = callback
    def park(self, parked: tuple[Work, ...]) -> None:
        self._deferred_stream_complete_args = parked
    def is_armed(self) -> bool:
        return self._on_deferred_stream_complete is not None
    def has_parked(self) -> bool:
        return self._deferred_stream_complete_args is not None
    def set_failure_hook(self, hook: Hook | None) -> None:
        self._on_detached_stream_failure = hook

class FailureHook:
    def __init__(self, calls: list[Json], *, raises: bool) -> None:
        self.calls = calls
        self.raises = raises
    async def __call__(self, exc: Exception) -> None:
        self.calls.append(f"{type(exc).__name__}({exc})")
        if self.raises:
            raise ValueError("hook")

class PlainStream(AsyncIterator[bytes]):
    @override
    async def __anext__(self) -> bytes:
        raise StopAsyncIteration

class ClosingStream(PlainStream):
    def __init__(self, *, fails: bool) -> None:
        self.fails = fails
    async def aclose(self) -> None:
        if self.fails:
            raise ValueError("closed")

class Session(streaming_iterator.BaseAnthropicMessagesStreamingIterator):
    def __init__(self, body: dict[str, Json], api_base: str, client: httpx.AsyncClient) -> None:
        self.logging = RecordedLogging()
        self.litellm_logging_obj = self.logging
        self.request_body: dict[str, Json] = body
        self.start_time = datetime.now(UTC).astimezone().replace(tzinfo=None)
        self.completion_start_time: datetime | None = None
        self.url = f"{api_base}/messages"
        self.client = client
        self.supplied: RecordedLogging | None = None
        self.recorder = Recorder()
        self.frames: list[bytes] = []
        self.hook_calls: list[Json] = []
        self.callback_args: list[Json] = []
        self.text: str | None = None
    async def collect_frames(self) -> list[bytes]:
        response = await self.client.post(self.url, json=self.request_body)
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
        return self._convert_chunk_to_sse_format(chunk)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L453-L466
    def relay(self, response: httpx.Response, supplied: RecordedLogging) -> AsyncIterator[bytes]:
        body: dict[str, Json] = {"model": "supplied"}
        return self.get_async_streaming_response_iterator(response, body, supplied)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/streaming_iterator.py#L431-L451
    async def run_step(self, step: "Step") -> Json:
        self.text = None
        await step.run(self)
        await GLOBAL_LOGGING_WORKER.flush()
        return self.observe()
    def seen(self, call: RouteCall | ProcessorCall) -> dict[str, Json]:
        handler = streaming_iterator.GLOBAL_PASS_THROUGH_SUCCESS_HANDLER_OBJ
        return {
            "request_body": call["request_body"],
            "endpoint_anthropic": call["endpoint_type"] == EndpointType.ANTHROPIC,
            "url_route": call["url_route"],
            "shared_success_handler": call["passthrough_success_handler_obj"] is handler,
            "start_is_construction": call["start_time"] == self.start_time,
        }
    def routed_entry(self, call: RouteCall, first: datetime | None) -> Json:
        within = None if first is None else call["start_time"] <= first <= call["end_time"]
        return {
            **self.seen(call),
            "raw_frames": [frame.decode(errors="replace") for frame in call["raw_bytes"]],
            "iterator_logging_obj": call["litellm_logging_obj"] is self.logging,
            "first_within_run": within,
        }
    def processed_entry(self, call: ProcessorCall) -> Json:
        return {
            **self.seen(call),
            "response_status": call["response"].status_code,
            "supplied_logging_obj": call["litellm_logging_obj"] is self.supplied,
        }
    def observe(self) -> Json:
        first = self.logging.completion_start_time
        recorded = first is not None and "completion_start_time" in self.logging.details
        outcome: Json = {
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
            session.completion_start_time = datetime.now(UTC).astimezone().replace(tzinfo=None)

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
        streams: dict[str, PlainStream] = {
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
                session.text = session.convert(text.encode()).decode(errors="replace")
            case dict(mapping):
                session.text = session.convert(mapping).decode(errors="replace")

class DelegateStep(StepModel):
    op: Literal["delegate"]
    async def run(self, session: Session) -> None:
        supplied = RecordedLogging()
        session.supplied = supplied
        async with session.client.stream(
            "POST", session.url, json=session.request_body
        ) as response:
            relayed = session.relay(response, supplied)
            relayed_bytes = b"".join([chunk async for chunk in relayed])
        session.text = relayed_bytes.decode(errors="replace")

Step = Annotated[
    CollectStep | BillStep | ArmDeferredStep | ParkArgsStep | DispatchStep
    | AbortUpstreamStep | DetachedFailureStep | ConvertStep | DelegateStep,
    Field(discriminator="op"),
]
class MessagesStreamLoggingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    body: dict[str, Json]
    steps: list[Step]
async def run_steps(call: MessagesStreamLoggingCall, api_base: str) -> list[Json]:
    async with httpx.AsyncClient(trust_env=False, timeout=120.0) as client:
        session = Session(call.body, api_base, client)
        logger = logging.getLogger("LiteLLM Proxy")
        logger.addHandler(session.recorder)
        logger.setLevel(logging.WARNING)
        install(session.recorder)
        outcomes = [await session.run_step(step) for step in call.steps]
        logger.removeHandler(session.recorder)
        await GLOBAL_LOGGING_WORKER.stop()
        return outcomes

def run_messages_stream_logging(call: MessagesStreamLoggingCall, api_base: str) -> str:
    steps = asyncio.run(run_steps(call, api_base))
    outcome: Json = {"messages_stream_logging": {"value": {"steps": steps}}}
    return json.dumps(outcome)
