import asyncio
from typing import Annotated, Final, Literal

from litellm.integrations.adaptive_concurrency import AdaptiveConcurrencyLimiter, PutSample
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MAX_FILL: Final = 4096


class State(Strict):
    limit: int
    in_flight: int
    waiters: int
    clean_streak: int


class SnapshotStep(Strict):
    action: Literal["snapshot"]


class RecordStep(Strict):
    action: Literal["record"]
    throttled: bool


class AcquireStep(Strict):
    action: Literal["acquire"]
    deadline_millis: int


class PollStep(Strict):
    action: Literal["poll"]
    id: int


class ReleaseStep(Strict):
    action: Literal["release"]
    id: int


class CancelStep(Strict):
    action: Literal["cancel"]
    id: int


class FillStep(Strict):
    action: Literal["fill"]
    count: int
    deadline_millis: int


Step = Annotated[
    SnapshotStep | RecordStep | AcquireStep | PollStep | ReleaseStep | CancelStep | FillStep,
    Field(discriminator="action"),
]


class SnapshotResult(Strict):
    action: Literal["snapshot"]
    state: State


class RecordResult(Strict):
    action: Literal["record"]
    state: State


class AcquireResult(Strict):
    action: Literal["acquire"]
    id: int
    outcome: Literal["granted", "queued", "overloaded", "timed_out"]
    state: State


class PollResult(Strict):
    action: Literal["poll"]
    id: int
    outcome: Literal["granted", "queued", "timed_out"]
    state: State


class ReleaseResult(Strict):
    action: Literal["release"]
    id: int
    state: State


class CancelResult(Strict):
    action: Literal["cancel"]
    id: int
    state: State


class FillResult(Strict):
    action: Literal["fill"]
    queued: int
    overloaded: int
    granted: int
    timed_out: int
    state: State


Result = Annotated[
    SnapshotResult | RecordResult | AcquireResult | PollResult | ReleaseResult | CancelResult | FillResult,
    Field(discriminator="action"),
]


class Output(Strict):
    results: list[Result]


class Acquisition:
    def __init__(self) -> None:
        self.task: asyncio.Task[None] | None = None
        self.release: asyncio.Event = asyncio.Event()
        self.granted: bool = False


def state(limiter: AdaptiveConcurrencyLimiter) -> State:
    return State(
        limit=limiter.limit,
        in_flight=limiter._in_flight,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        waiters=len(limiter._waiters),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        clean_streak=limiter._clean_streak,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )


def start(limiter: AdaptiveConcurrencyLimiter, acquisitions: list[Acquisition]) -> int:
    acquired = Acquisition()
    acquired.task = asyncio.create_task(enter(limiter, acquired))
    acquisitions.append(acquired)
    return len(acquisitions) - 1


async def enter(limiter: AdaptiveConcurrencyLimiter, acquired: Acquisition) -> None:
    _ = await limiter.__aenter__()
    acquired.granted = True
    _ = await acquired.release.wait()
    await limiter.__aexit__(None, None, None)


def lookup(acquisitions: list[Acquisition], identifier: int) -> Acquisition:
    if identifier < 0 or identifier >= len(acquisitions):
        raise ProbeError("invalid_request", "the step names an acquisition that was never started")
    return acquisitions[identifier]


def record_or_snapshot(
    step: SnapshotStep | RecordStep,
    limiter: AdaptiveConcurrencyLimiter,
) -> SnapshotResult | RecordResult:
    if isinstance(step, SnapshotStep):
        return SnapshotResult(action="snapshot", state=state(limiter))
    limiter.record(PutSample(throttled=step.throttled))
    return RecordResult(action="record", state=state(limiter))


async def acquire_or_fill(
    step: AcquireStep | FillStep,
    limiter: AdaptiveConcurrencyLimiter,
    acquisitions: list[Acquisition],
) -> AcquireResult | FillResult:
    if isinstance(step, AcquireStep):
        if step.deadline_millis < 0:
            raise ProbeError("invalid_request", "a deadline is a non-negative number of milliseconds")
        identifier = start(limiter, acquisitions)
        await asyncio.sleep(0)
        outcome: Literal["granted", "queued"] = "granted" if acquisitions[identifier].granted else "queued"
        return AcquireResult(action="acquire", id=identifier, outcome=outcome, state=state(limiter))
    if step.count < 0 or step.count > MAX_FILL:
        raise ProbeError("invalid_request", "a fill is limited to 4096 acquisitions")
    first = len(acquisitions)
    for _ in range(step.count):
        _ = start(limiter, acquisitions)
    await asyncio.sleep(0)
    fresh = acquisitions[first:]
    queued = sum(1 for acquired in fresh if not acquired.granted)
    return FillResult(
        action="fill",
        queued=queued,
        overloaded=0,
        granted=step.count - queued,
        timed_out=0,
        state=state(limiter),
    )


async def poll_release_or_cancel(
    step: PollStep | ReleaseStep | CancelStep,
    limiter: AdaptiveConcurrencyLimiter,
    acquisitions: list[Acquisition],
) -> PollResult | ReleaseResult | CancelResult:
    acquired = lookup(acquisitions, step.id)
    if isinstance(step, PollStep):
        await asyncio.sleep(0)
        outcome: Literal["granted", "queued"] = "granted" if acquired.granted else "queued"
        return PollResult(action="poll", id=step.id, outcome=outcome, state=state(limiter))
    if isinstance(step, ReleaseStep):
        if not acquired.granted:
            raise ProbeError("invalid_request", "release names an acquisition that holds no permit")
        acquired.release.set()
        await asyncio.sleep(0)
        return ReleaseResult(action="release", id=step.id, state=state(limiter))
    task = acquired.task
    if acquired.granted:
        raise ProbeError("invalid_request", "cancel names an acquisition that already holds a permit")
    if task is None or task.done():
        raise ProbeError("invalid_request", "cancel names a finished acquisition")
    _ = task.cancel()
    await asyncio.sleep(0)
    return CancelResult(action="cancel", id=step.id, state=state(limiter))


async def run_step(
    step: Step,
    limiter: AdaptiveConcurrencyLimiter,
    acquisitions: list[Acquisition],
) -> Result:
    if isinstance(step, SnapshotStep | RecordStep):
        return record_or_snapshot(step, limiter)
    if isinstance(step, AcquireStep | FillStep):
        return await acquire_or_fill(step, limiter, acquisitions)
    return await poll_release_or_cancel(step, limiter, acquisitions)


async def cleanup(acquisitions: list[Acquisition]) -> None:
    for acquired in acquisitions:
        task = acquired.task
        if task is not None and not task.done():
            if acquired.granted:
                acquired.release.set()
            else:
                _ = task.cancel()
    await asyncio.sleep(0)
    await asyncio.sleep(0)


class AdaptiveConcurrency(Operation):
    initial: int
    floor: int
    ceiling: int
    steps: list[Step]

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        if min(self.initial, self.floor, self.ceiling) < 0:
            raise ProbeError("invalid_request", "the initial, floor and ceiling are non-negative integers")
        try:
            limiter = AdaptiveConcurrencyLimiter(initial=self.initial, floor=self.floor, ceiling=self.ceiling)
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        acquisitions: list[Acquisition] = []
        results: list[Result] = [await run_step(step, limiter, acquisitions) for step in self.steps]
        await cleanup(acquisitions)
        return Output(results=results)
