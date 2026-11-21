import asyncio
import time
from typing import Annotated, Final, Literal, Protocol
from unittest.mock import patch
from litellm.exceptions import Timeout
from litellm.router import Router
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.scheduler import FlowItem, Scheduler
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import Field, TypeAdapter
from typing_extensions import TypedDict, override
from probes.protocol import Operation, Strict
# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false
_MODEL: Final = "gpt-4o"
_MESSAGES: Final[list[AllMessageValues]] = [{"role": "user", "content": "hi"}]
_API_KEY: Final = "sk-probe"
_QUEUE: Final = TypeAdapter(list[tuple[int, str]])
Headers = TypedDict("Headers", {"x-litellm-request-prioritization-used": bool})
class Hidden(TypedDict):
    additional_headers: Headers
_HIDDEN: Final = TypeAdapter(Hidden)
class Entry(Strict):
    priority: int
    request_id: str
class Failure(Strict):
    status: int | None
    code: str | None
    priority: int | None
class Added(Strict):
    result: Literal["added"] = "added"
class Polled(Strict):
    result: Literal["polled"] = "polled"
    allowed: bool
class Peeked(Strict):
    result: Literal["peeked"] = "peeked"
    head: bool
class Queued(Strict):
    result: Literal["queue"] = "queue"
class Interval(Strict):
    result: Literal["interval"] = "interval"
    interval: str
class Scheduled(Strict):
    result: Literal["scheduled"] = "scheduled"
    prioritized: bool
    priority: int
class Errored(Strict):
    result: Literal["error"] = "error"
Outcome = Annotated[
    Added | Polled | Peeked | Queued | Interval | Scheduled | Errored,
    Field(discriminator="result"),
]
class StepOutput(Strict):
    outcome: Outcome
    queue: list[Entry]
    depth: int
    error: Failure | None
class Output(Strict):
    steps: list[StepOutput]
    calls: int
class Fill(Strict):
    count: int
    prefix: str
    priority: int = 0
class AddStep(Strict):
    op: Literal["add"] = "add"
    priority: int
    request_id: str
    model: str = _MODEL
class PollStep(Strict):
    op: Literal["poll"] = "poll"
    request_id: str
    model: str = _MODEL
    healthy: int
class PeekStep(Strict):
    op: Literal["peek"] = "peek"
    request_id: str
    model: str = _MODEL
class QueueStep(Strict):
    op: Literal["queue"] = "queue"
    model: str = _MODEL
class IntervalStep(Strict):
    op: Literal["interval"] = "interval"
    model: str = _MODEL
class ScheduleStep(Strict):
    op: Literal["schedule"] = "schedule"
    priority: int
    request_id: str
    model: str = _MODEL
    healthy: int
Step = Annotated[
    AddStep | PollStep | PeekStep | QueueStep | IntervalStep | ScheduleStep,
    Field(discriminator="op"),
]
# the virtual clock the polling loop reads and spends, so a timeout case costs no wall time
class Clock:
    def __init__(self) -> None:
        self.seconds: float = 0.0
    def monotonic(self) -> float:
        return self.seconds
    async def sleep(self, delay: float) -> None:
        self.seconds += delay
# Router._async_get_healthy_deployments as the case scripts it: the scheduler reads only the first list's length
class Healthy:
    def __init__(self, count: int) -> None:
        self.count: int = count
    async def __call__(self, **_kwargs: object) -> tuple[list[dict[str, str]], list[dict[str, str]]]:
        return ([{}] * self.count, [{}])
# the provider the scheduled request reaches, counted so the case proves when it ran
class Answer:
    def __init__(self) -> None:
        self.calls: int = 0
    async def __call__(self, **_kwargs: object) -> ModelResponse:
        self.calls += 1
        return ModelResponse(id="probe", created=0, model=_MODEL, choices=[])
class SchedulerCall(Protocol):
    async def schedule_acompletion(
        self, *, model: str, messages: list[AllMessageValues], priority: int, stream: bool
    ) -> ModelResponse: ...
def entry(model: str) -> dict[str, object]:
    return {
        "model_name": model,
        "litellm_params": {"model": f"openai/{model}", "api_key": _API_KEY},
    }
async def schedule_step(step: ScheduleStep, router: SchedulerCall, answer: Answer) -> tuple[Outcome, Failure | None]:
    with patch.object(router, "_async_get_healthy_deployments", Healthy(step.healthy)), patch.object(
        router, "acompletion", answer
    ):
        try:
            response = await router.schedule_acompletion(
                model=step.model,
                messages=_MESSAGES,
                priority=step.priority,
                stream=False,
            )
        except Timeout as error:
            return (Errored(), Failure(status=error.status_code, code=None, priority=None))
    used = _HIDDEN.validate_python(get_hidden_params_dict(response))["additional_headers"]
    return (Scheduled(prioritized=used["x-litellm-request-prioritization-used"], priority=step.priority), None)
async def apply_step(step: Step, router: Router, answer: Answer) -> tuple[Outcome, Failure | None]:
    scheduler: Scheduler = router.scheduler
    error: Failure | None = None
    outcome: Outcome
    if isinstance(step, AddStep):
        await scheduler.add_request(
            FlowItem(priority=step.priority, request_id=step.request_id, model_name=step.model)
        )
        outcome = Added()
    elif isinstance(step, PollStep):
        outcome = Polled(
            allowed=await scheduler.poll(
                id=step.request_id,
                model_name=step.model,
                health_deployments=[{}] * step.healthy,
            )
        )
    elif isinstance(step, PeekStep):
        outcome = Peeked(
            head=await scheduler.peek(
                id=step.request_id,
                model_name=step.model,
                health_deployments=[],
            )
        )
    elif isinstance(step, QueueStep):
        outcome = Queued()
    elif isinstance(step, ScheduleStep):
        outcome, error = await schedule_step(step, router, answer)
    else:
        outcome = Interval(interval=f"{scheduler.polling_interval:.6f}")
    return (outcome, error)
# get_queue returns the cached heap, so the case reads it in the order the heap serves it
async def read_queue(router: Router, model: str) -> list[Entry]:
    return [
        Entry(priority=priority, request_id=request_id)
        for priority, request_id in sorted(_QUEUE.validate_python(await router.scheduler.get_queue(model_name=model)))
    ]
class RequestScheduler(Operation):
    polling_interval: str | None = None
    default_priority: int | None = None
    timeout: str
    fill: Fill | None = None
    steps: list[Step]
    @override
    def run(self) -> Output:
        return asyncio.run(self.scenario())
    async def scenario(self) -> Output:
        clock = Clock()
        answer = Answer()
        router = self.router()
        with patch.object(time, "monotonic", clock.monotonic), patch.object(asyncio, "sleep", clock.sleep):
            if self.fill is not None:
                for index in range(self.fill.count):
                    await router.scheduler.add_request(
                        FlowItem(
                            priority=self.fill.priority,
                            request_id=f"{self.fill.prefix}-{index}",
                            model_name=_MODEL,
                        )
                    )
            outputs: list[StepOutput] = []
            for step in self.steps:
                outcome, error = await apply_step(step, router, answer)
                queue = await read_queue(router, step.model)
                outputs.append(StepOutput(outcome=outcome, queue=queue, depth=len(queue), error=error))
            return Output(steps=outputs, calls=answer.calls)
    def router(self) -> Router:
        models = {_MODEL}
        for step in self.steps:
            models.add(step.model)
        return Router(
            model_list=[entry(model) for model in sorted(models)],
            polling_interval=None if self.polling_interval is None else float(self.polling_interval),
            default_priority=self.default_priority,
            timeout=float(self.timeout),
        )
