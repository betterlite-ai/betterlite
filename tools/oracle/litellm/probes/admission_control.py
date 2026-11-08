import asyncio
import time as time_module
from contextlib import suppress
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy.middleware.admission_control_middleware import (
    AdmissionControlMiddleware,
    AdmissionControlSettings,
    AdmissionControlState,
    AdmissionControlStats,
    get_admission_control_settings,
)
from litellm.proxy.middleware.in_flight_requests_middleware import InFlightRequestsMiddleware
from pydantic import Field, JsonValue, SerializeAsAny, TypeAdapter
from starlette.types import Message, Receive, Scope, Send
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnusedCallResult=false

WAIT_SECONDS: Final = 5.0
REAL_TIME: Final = (time_module.time, time_module.monotonic)
_JSON: Final = TypeAdapter(JsonValue)

Behavior: TypeAlias = Literal["immediate", "hold", "stream"]


class ResponseStart(TypedDict):
    type: Literal["http.response.start"]
    status: int
    headers: list[tuple[bytes, bytes]]


class ResponseBody(TypedDict):
    type: Literal["http.response.body"]
    body: NotRequired[bytes]
    more_body: NotRequired[bool]


AsgiMessage: TypeAlias = ResponseStart | ResponseBody


class RequestSpec(Strict):
    id: int
    path: str
    behavior: Behavior


RequestStepKind: TypeAlias = Literal["start", "await_started", "release", "cancel", "complete"]


class RequestStep(Strict):
    kind: RequestStepKind
    request: int


class AwaitQueuedStep(Strict):
    kind: Literal["await_queued"]
    queued: int


class StatsStep(Strict):
    kind: Literal["stats"]


Step: TypeAlias = RequestStep | AwaitQueuedStep | StatsStep

StepField: TypeAlias = SerializeAsAny[Step]


class LimitsOutput(Strict):
    max_in_flight_requests: int
    max_queued_requests: int
    queue_timeout_seconds: float


class StartedObservation(Strict):
    kind: Literal["started"] = "started"
    request: int


class QueuedObservation(Strict):
    kind: Literal["queued"] = "queued"
    queued: int


class StatsObservation(Strict):
    kind: Literal["stats"] = "stats"
    admitted: int
    queued: int
    rejected_total: int
    in_flight: int


class CompletedObservation(Strict):
    kind: Literal["completed"] = "completed"
    request: int
    status: int
    retry_after: str | None
    content_type: str | None
    body: JsonValue


class TimedOutObservation(Strict):
    kind: Literal["timed_out"] = "timed_out"
    wait: str


Observation: TypeAlias = (
    StartedObservation | QueuedObservation | StatsObservation | CompletedObservation | TimedOutObservation
)

ObservationField: TypeAlias = SerializeAsAny[Observation]


class Output(Strict):
    limits: LimitsOutput | None
    observations: list[ObservationField]


class Gate:
    behavior: Behavior
    started: asyncio.Event
    release: asyncio.Event

    def __init__(self, behavior: Behavior) -> None:
        self.behavior = behavior
        self.started = asyncio.Event()
        self.release = asyncio.Event()


class ObservedState(AdmissionControlState):
    changed: asyncio.Event

    def __init__(self) -> None:
        super().__init__(lambda: None)
        self.changed = asyncio.Event()

    @override
    def record_queue(self) -> None:
        super().record_queue()
        self.changed.set()

    @override
    def record_dequeue(self) -> None:
        super().record_dequeue()
        self.changed.set()


class Recorder:
    messages: list[object]

    def __init__(self) -> None:
        self.messages = []

    async def receive(self) -> Message:
        return {"type": "http.request", "body": b"", "more_body": False}

    async def send(self, message: Message) -> None:
        self.messages.append(message)


Answer: TypeAlias = tuple[int, dict[str, str], bytes]


class Harness:
    parsed: AdmissionControlSettings | None
    state: ObservedState
    root_path: str
    specs: dict[int, RequestSpec]
    by_path: dict[str, int]
    gates: dict[str, Gate]
    tasks: dict[int, asyncio.Task[None]]
    answers: dict[int, Answer]
    limits_output: LimitsOutput | None
    stack: AdmissionControlMiddleware

    def __init__(self, settings: dict[str, JsonValue], root_path: str, requests: list[RequestSpec]) -> None:
        self.parsed = get_admission_control_settings(settings)
        self.limits_output = self.limit_values(self.parsed)
        self.state = ObservedState()
        self.root_path = root_path
        self.specs = {}
        self.by_path = {}
        self.gates = {}
        for spec in requests:
            self.specs[spec.id] = spec
            self.by_path[spec.path] = spec.id
            self.gates[spec.path] = Gate(spec.behavior)
        self.tasks = {}
        self.answers = {}
        in_flight = InFlightRequestsMiddleware(self.app)
        self.stack = AdmissionControlMiddleware(in_flight, self.get_settings, self.state)

    def get_settings(self) -> AdmissionControlSettings | None:
        return self.parsed

    def limit_values(self, settings: AdmissionControlSettings | None) -> LimitsOutput | None:
        if settings is None:
            return None
        return LimitsOutput(
            max_in_flight_requests=settings.max_in_flight_requests,
            max_queued_requests=settings.max_queued_requests,
            queue_timeout_seconds=settings.queue_timeout_seconds,
        )

    def spec(self, request_id: int) -> RequestSpec:
        spec = self.specs.get(request_id)
        if spec is None:
            raise ProbeError("invalid_request", "a step names an unknown request")
        return spec

    def gate(self, request_id: int) -> Gate:
        return self.gates[self.spec(request_id).path]

    async def app(self, scope: Scope, _receive: Receive, send: Send) -> None:
        request_id = self.by_path.get(cast("str", scope["path"]))
        spec = None if request_id is None else self.specs.get(request_id)
        if spec is None:
            await send({"type": "http.response.start", "status": 200, "headers": []})
            await send({"type": "http.response.body", "body": b"ok", "more_body": False})
            return
        gate = self.gates[spec.path]
        if spec.behavior == "immediate":
            gate.started.set()
            await send({"type": "http.response.start", "status": 200, "headers": []})
            await send({"type": "http.response.body", "body": b"ok", "more_body": False})
            return
        if spec.behavior == "hold":
            gate.started.set()
            await gate.release.wait()
            await send({"type": "http.response.start", "status": 200, "headers": []})
            await send({"type": "http.response.body", "body": b"ok", "more_body": False})
            return
        await send({"type": "http.response.start", "status": 200, "headers": []})
        await send({"type": "http.response.body", "body": b"first", "more_body": True})
        gate.started.set()
        await gate.release.wait()
        await send({"type": "http.response.body", "body": b"last", "more_body": False})

    async def call(self, request_id: int, spec: RequestSpec) -> None:
        recorder = Recorder()
        scope: Scope = {
            "type": "http",
            "path": spec.path,
            "root_path": self.root_path,
            "method": "GET",
            "headers": [],
        }
        await self.stack(scope, recorder.receive, recorder.send)
        self.answers[request_id] = self.collect(recorder.messages)

    def collect(self, messages: list[object]) -> Answer:
        status = 0
        headers: dict[str, str] = {}
        body = bytearray()
        for raw in messages:
            message = cast("AsgiMessage", raw)
            if message["type"] == "http.response.start":
                status = message["status"]
                headers = {key.decode("latin-1").lower(): value.decode("latin-1") for key, value in message["headers"]}
            else:
                body.extend(message.get("body", b""))
        return status, headers, bytes(body)

    async def drive(self, steps: list[Step]) -> Output:
        observations: list[ObservationField] = []
        for step in steps:
            match step:
                case RequestStep():
                    await self.request_step(step, observations)
                case AwaitQueuedStep():
                    observations.append(await self.await_queued(step.queued))
                case StatsStep():
                    observations.append(self.stats())
        return Output(limits=self.limits_output, observations=observations)

    async def request_step(self, step: RequestStep, observations: list[ObservationField]) -> None:
        match step.kind:
            case "start":
                self.start(step.request)
            case "await_started":
                observations.append(await self.await_started(step.request))
            case "release":
                self.gate(step.request).release.set()
            case "cancel":
                await self.cancel(step.request)
            case "complete":
                observations.append(await self.complete(step.request))

    def start(self, request_id: int) -> None:
        spec = self.spec(request_id)
        if request_id in self.tasks:
            raise ProbeError("invalid_request", "a request is started twice")
        self.tasks[request_id] = asyncio.create_task(self.call(request_id, spec))

    async def await_started(self, request_id: int) -> Observation:
        gate = self.gate(request_id)
        try:
            await asyncio.wait_for(gate.started.wait(), timeout=WAIT_SECONDS)
        except TimeoutError:
            return TimedOutObservation(wait="started")
        return StartedObservation(request=request_id)

    async def await_queued(self, queued: int) -> Observation:
        loop = asyncio.get_running_loop()
        deadline = loop.time() + WAIT_SECONDS
        while self.state.get_stats().queued != queued:
            if loop.time() > deadline:
                return TimedOutObservation(wait="queued")
            self.state.changed.clear()
            with suppress(TimeoutError):
                await asyncio.wait_for(self.state.changed.wait(), timeout=0.001)
        return QueuedObservation(queued=queued)

    async def cancel(self, request_id: int) -> None:
        task = self.tasks.pop(request_id, None)
        if task is None:
            raise ProbeError("invalid_request", "a request is cancelled twice")
        task.cancel()
        with suppress(asyncio.CancelledError):
            await task

    async def complete(self, request_id: int) -> Observation:
        task = self.tasks.pop(request_id, None)
        if task is None:
            raise ProbeError("invalid_request", "a request completes twice")
        try:
            await asyncio.wait_for(task, timeout=WAIT_SECONDS)
        except TimeoutError as error:
            task.cancel()
            raise ProbeError("invalid_request", "a request never completed") from error
        answer = self.answers.get(request_id)
        if answer is None:
            raise ProbeError("invalid_request", "a request never completed")
        status, headers, body = answer
        content_type = headers.get("content-type")
        return CompletedObservation(
            request=request_id,
            status=status,
            retry_after=headers.get("retry-after"),
            content_type=content_type,
            body=self.body_value(content_type, body),
        )

    def body_value(self, content_type: str | None, body: bytes) -> JsonValue:
        if not body:
            return None
        text = body.decode("utf-8")
        if content_type is not None and content_type.startswith("application/json"):
            return _JSON.validate_json(text)
        return text

    def stats(self) -> Observation:
        stats: AdmissionControlStats = self.state.get_stats()
        return StatsObservation(
            admitted=stats.admitted,
            queued=stats.queued,
            rejected_total=stats.rejected_total,
            in_flight=InFlightRequestsMiddleware.get_count(),
        )


class AdmissionControl(Operation):
    settings: dict[str, JsonValue] = Field(default_factory=dict)
    root_path: str = ""
    requests: list[RequestSpec]
    steps: list[StepField]

    @override
    def run(self) -> Output:
        # the probe protocol freezes time.monotonic, and asyncio schedules its queue timers with it
        frozen = (time_module.time, time_module.monotonic)
        time_module.time, time_module.monotonic = REAL_TIME
        try:
            harness = Harness(self.settings, self.root_path, self.requests)
            return asyncio.run(harness.drive(self.steps))
        finally:
            time_module.time, time_module.monotonic = frozen
