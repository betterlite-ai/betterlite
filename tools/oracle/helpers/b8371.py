import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy.common_utils.scheduled_job_stagger import Trigger
from pydantic import JsonValue

litellm.suppress_debug_info = True

# apscheduler's scheduler states: the retention hook returns early only while stopped
SCHEDULER_STATES: Final[dict[str, int]] = {"stopped": 0, "paused": 2}


class TriggerInput(TypedDict):
    operation: Literal["cleanup_trigger"]
    settings: dict[str, JsonValue]
    start: str


class RetentionInput(TypedDict):
    operation: Literal["apply_retention"]
    settings: dict[str, JsonValue]
    previous: dict[str, JsonValue]
    last_attempt: dict[str, JsonValue] | None
    reschedule_failed: bool
    scheduler: Literal["none", "stopped", "paused"]
    job_present: bool
    add_fails: bool
    start: str


class RetentionState(TypedDict):
    previous: dict[str, JsonValue]
    last_attempt: dict[str, JsonValue] | None
    reschedule_failed: bool
    scheduler: Literal["none", "stopped", "paused"]
    job_present: bool
    add_fails: bool


class SchedulerStub:
    state: int
    job_present: bool
    add_fails: bool
    added: int
    removed: int
    trigger: Trigger | None

    def __init__(self, *, state: int, job_present: bool, add_fails: bool) -> None:
        self.state = state
        self.job_present = job_present
        self.add_fails = add_fails
        self.added = 0
        self.removed = 0
        self.trigger = None

    def get_job(self, job_id: str) -> str | None:
        if self.job_present:
            return job_id
        return None

    def add_job(
        self,
        func: Callable[..., Awaitable[None]],
        trigger: Trigger,
        **options: JsonValue,
    ) -> None:
        del func, options
        if self.add_fails:
            raise RuntimeError("the scheduler rejected the spend log cleanup job")
        self.added = self.added + 1
        self.trigger = trigger
        self.job_present = True

    def remove_job(self, job_id: str) -> None:
        del job_id
        self.removed = self.removed + 1
        self.job_present = False


def retention_state(input_value: TriggerInput | RetentionInput) -> RetentionState:
    if input_value["operation"] == "apply_retention":
        return RetentionState(
            previous=input_value["previous"],
            last_attempt=input_value["last_attempt"],
            reschedule_failed=input_value["reschedule_failed"],
            scheduler=input_value["scheduler"],
            job_present=input_value["job_present"],
            add_fails=input_value["add_fails"],
        )
    return RetentionState(
        previous={},
        last_attempt=None,
        reschedule_failed=False,
        scheduler="paused",
        job_present=False,
        add_fails=False,
    )


def trigger_gap(trigger: Trigger) -> int | None:
    now = datetime.now(UTC)
    first = trigger.get_next_fire_time(None, now)
    if first is None:
        return None
    second = trigger.get_next_fire_time(first, first)
    if second is None:
        return None
    return int((second - first).total_seconds())


def trigger_outcome(trigger: Trigger | None) -> JsonValue:
    if trigger is None:
        return {"kind": None, "gap_seconds": None}
    return {"kind": str(trigger).split("[", 1)[0], "gap_seconds": trigger_gap(trigger)}


def retention_outcome(stub: SchedulerStub | None, trigger: Trigger | None) -> JsonValue:
    added = 0 if stub is None else stub.added
    removed = 0 if stub is None else stub.removed
    present = False if stub is None else stub.job_present
    kind = None if trigger is None else str(trigger).split("[", 1)[0]
    gap = None if trigger is None else trigger_gap(trigger)
    return {
        "added": added,
        "removed": removed,
        "job_present": present,
        "kind": kind,
        "gap_seconds": gap,
    }


async def apply_retention(
    settings: dict[str, JsonValue], state: RetentionState
) -> SchedulerStub | None:
    config = proxy_server.ProxyConfig()
    config.settings.load_yaml(settings)
    proxy_server.general_settings = settings
    role = state["scheduler"]
    stub: SchedulerStub | None = None
    if role == "none":
        proxy_server.scheduler = None
    else:
        stub = SchedulerStub(
            state=SCHEDULER_STATES[role],
            job_present=state["job_present"],
            add_fails=state["add_fails"],
        )
        proxy_server.scheduler = stub  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2678
    previous = proxy_server._cleanup_schedule_of(state["previous"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L5323
    last = state["last_attempt"]
    config._last_cleanup_schedule_attempt = None if last is None else proxy_server._cleanup_schedule_of(last)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L5404
    config._cleanup_reschedule_failed = state["reschedule_failed"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L5405
    await config._apply_retention_settings({}, previous)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7963
    return stub


def run(input_value: TriggerInput | RetentionInput) -> JsonValue:
    stub = asyncio.run(apply_retention(input_value["settings"], retention_state(input_value)))
    trigger = None if stub is None else stub.trigger
    if input_value["operation"] == "cleanup_trigger":
        return trigger_outcome(trigger)
    return retention_outcome(stub, trigger)
