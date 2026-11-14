import asyncio
import json
import logging
from collections.abc import AsyncIterator
from typing import Literal, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from litellm.llms.anthropic.pass_through.messages import streaming_iterator
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from pydantic import BaseModel, ConfigDict, JsonValue as Json
from typing_extensions import override

from messages_stream_logging import RecordedLogging, Recorder, RouteCall, Work, install

TIMEOUT_SECONDS = 120.0


class RateLimitError(Exception):
    pass


class FailureSeamCall(TypedDict):
    litellm_logging_obj: RecordedLogging
    endpoint_type: EndpointType
    request_body: dict[str, Json]
    raw_bytes: list[bytes]
    exception: Exception


class Gate:
    def __init__(self) -> None:
        self.permits = 0
        self.finished = False
        self.failure: RateLimitError | None = None
        self.waiting = 0
        self.ready = asyncio.Event()
        self.idle = asyncio.Event()

    def release(self, count: int) -> None:
        self.permits += count
        self.ready.set()

    def finish(self) -> None:
        self.finished = True
        self.ready.set()

    def fail(self) -> None:
        self.failure = RateLimitError("busy")
        self.ready.set()

    async def quiesce(self) -> None:
        while not self.finished and (self.permits > 0 or self.waiting == 0):
            self.idle.clear()
            await self.idle.wait()


class GatedSource(AsyncIterator[bytes]):
    def __init__(self, gate: Gate) -> None:
        self.gate = gate
        self.frames: list[bytes] = []
        self.index = 0
        self.closed = False

    @override
    async def __anext__(self) -> bytes:
        while True:
            if self.gate.failure is not None:
                raise self.gate.failure
            if self.gate.permits > 0:
                self.gate.permits -= 1
                if self.index >= len(self.frames):
                    raise StopAsyncIteration
                frame = self.frames[self.index]
                self.index += 1
                return frame
            if self.gate.finished:
                raise StopAsyncIteration
            self.gate.waiting += 1
            self.gate.idle.set()
            self.gate.ready.clear()
            await self.gate.ready.wait()
            self.gate.waiting -= 1

    async def aclose(self) -> None:
        self.closed = True


class RelayRecorder(Recorder):
    def __init__(self) -> None:
        super().__init__()
        self.failures: list[Json] = []
        self.recorded = asyncio.Event()


class Dispatcher(Recorder):
    def __init__(self) -> None:
        super().__init__()
        self.by_logging: dict[int, RelayRecorder] = {}

    def bind(self, logging_obj: RecordedLogging, recorder: RelayRecorder) -> None:
        self.by_logging[id(logging_obj)] = recorder

    def _route_streaming_logging_to_handler(  # ast-grep-ignore: no-long-names
        self, **call: Unpack[RouteCall]
    ) -> Work:
        recorder = self.by_logging[id(call["litellm_logging_obj"])]
        recorder.routed.append(call)
        recorder.recorded.set()
        return recorder.counted()

    async def schedule_stream_failure_logging(  # ast-grep-ignore: no-long-names
        self, **call: Unpack[FailureSeamCall]
    ) -> None:
        recorder = self.by_logging[id(call["litellm_logging_obj"])]
        failure: Json = {
            "kind": type(call["exception"]).__name__,
            "message": str(call["exception"]),
            "endpoint_anthropic": call["endpoint_type"] == EndpointType.ANTHROPIC,
            "request_body": call["request_body"],
            "raw_frames": [frame.decode() for frame in call["raw_bytes"]],
        }
        recorder.failures.append(failure)
        recorder.recorded.set()


class RelayHook:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.fired = asyncio.Event()

    async def __call__(self, exc: Exception) -> None:
        self.calls.append(f"{type(exc).__name__}({exc})")
        self.fired.set()


def split_frames(content: bytes) -> list[bytes]:
    parts = content.split(b"\n\n")
    frames = [part + b"\n\n" for part in parts[:-1]]
    if parts[-1]:
        frames.append(parts[-1])
    return frames


class RelayStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    op: Literal[
        "start",
        "release",
        "read",
        "detach",
        "finish",
        "fail",
        "arm",
        "wait",
        "settle",
    ]
    stream: str = ""
    count: int = 0
    until: Literal["idle", "parked", "hook"] = "idle"


class MessagesNativeStreamCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    body: dict[str, Json]
    queue_maxsize: int
    max_detached_stream_drains: int
    steps: list[RelayStep]


class Session(streaming_iterator.BaseAnthropicMessagesStreamingIterator):
    def __init__(
        self,
        call: MessagesNativeStreamCall,
        api_base: str,
        client: httpx.AsyncClient,
        dispatcher: Dispatcher,
    ) -> None:
        self.logging = RecordedLogging()
        super().__init__(self.logging, call.body)
        self.body = call.body
        self.api_base = api_base
        self.client = client
        self.dispatcher = dispatcher
        self.gate = Gate()
        self.source = GatedSource(self.gate)
        self.relay = self.async_sse_wrapper(self.source)
        self.recorder = RelayRecorder()
        self.hook = RelayHook()
        self.callback_args: list[int] = []
        self.logging.set_failure_hook(self.hook)

    async def start(self) -> None:
        response = await self.client.post(f"{self.api_base}/messages", json=self.body)
        self.source.frames = split_frames(response.content)
        self.dispatcher.bind(self.logging, self.recorder)

    async def read(self) -> Json:
        try:
            chunk = await anext(self.relay)
        except StopAsyncIteration:
            return {"chunk": None, "end": True, "error": None}
        except RateLimitError as error:
            return {"chunk": None, "end": False, "error": f"{type(error).__name__}({error})"}
        return {"chunk": chunk.decode(), "end": False, "error": None}

    async def detach(self) -> None:
        await streaming_iterator.aclose_if_supported(self.relay)

    def arm(self) -> None:
        self.logging.arm_deferred(self.on_deferred)

    async def on_deferred(self, *parked: Work) -> None:
        self.callback_args.append(len(parked))
        for coroutine in parked:
            await coroutine

    async def wait(self, until: Literal["idle", "parked", "hook"]) -> None:
        match until:
            case "idle":
                await self.gate.quiesce()
            case "parked":
                await self.recorder.recorded.wait()
            case "hook":
                await self.hook.fired.wait()

    async def configure(self, step: RelayStep) -> None:
        if step.op == "release":
            self.gate.release(step.count)
        elif step.op == "detach":
            await self.detach()
        elif step.op == "finish":
            self.gate.finish()
        elif step.op == "fail":
            self.gate.fail()
        elif step.op == "arm":
            self.arm()
        elif step.op == "wait":
            await self.wait(step.until)

    async def run(self, step: RelayStep) -> Json:
        if step.op == "read":
            return await self.read()
        await self.configure(step)
        return None

    def observe(self) -> Json:
        billed: list[Json] = [
            [frame.decode() for frame in call["raw_bytes"]] for call in self.recorder.routed
        ]
        return {
            "billed": billed,
            "failures": self.recorder.failures,
            "hook_calls": self.hook.calls,
            "callback_args": self.callback_args,
            "ran": self.recorder.ran,
            "armed": self.logging.is_armed(),
            "parked": self.logging.has_parked(),
            "closed": self.source.closed,
        }


class Run:
    def __init__(
        self, call: MessagesNativeStreamCall, api_base: str, client: httpx.AsyncClient
    ) -> None:
        self.call = call
        self.api_base = api_base
        self.client = client
        self.dispatcher = Dispatcher()
        self.warnings = Recorder()
        self.sessions: dict[str, Session] = {}

    def begin(self) -> None:
        install(self.dispatcher)
        logger = logging.getLogger("LiteLLM Proxy")
        logger.addHandler(self.warnings)
        logger.setLevel(logging.WARNING)

    async def settle(self) -> Json:
        for session in self.sessions.values():
            await session.recorder.recorded.wait()
        await GLOBAL_LOGGING_WORKER.flush()
        return {
            "warnings": list(self.warnings.warnings),
            "streams": {name: session.observe() for name, session in self.sessions.items()},
        }

    async def run(self, step: RelayStep) -> Json:
        if step.op == "settle":
            return {"frame": None, "observation": await self.settle()}
        if step.op == "start":
            session = Session(self.call, self.api_base, self.client, self.dispatcher)
            self.sessions[step.stream] = session
            await session.start()
            return {"frame": None, "observation": None}
        frame = await self.sessions[step.stream].run(step)
        return {"frame": frame, "observation": None}


async def run_steps(call: MessagesNativeStreamCall, api_base: str) -> list[Json]:
    async with httpx.AsyncClient(trust_env=False, timeout=TIMEOUT_SECONDS) as client:
        run = Run(call, api_base, client)
        run.begin()
        outcomes = [await run.run(step) for step in call.steps]
        await GLOBAL_LOGGING_WORKER.stop()
        return outcomes


def run_messages_native_stream(call: MessagesNativeStreamCall, api_base: str) -> str:
    steps = asyncio.run(run_steps(call, api_base))
    outcome: Json = {"messages_native_stream": {"value": {"steps": steps}}}
    return json.dumps(outcome)
