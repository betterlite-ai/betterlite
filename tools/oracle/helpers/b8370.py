from datetime import UTC, datetime, timedelta
from typing import Protocol, TypeAlias, cast

from apscheduler.triggers.cron import CronTrigger
from apscheduler.triggers.interval import IntervalTrigger
from litellm.proxy.common_utils.scheduled_job_stagger import (
    _interval_seconds,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

FIRE_COUNT = 3


class Trigger(Protocol):
    def get_next_fire_time(
        self, previous_fire_time: datetime | None, now: datetime
    ) -> datetime | None: ...


class CronTriggerFactory(Protocol):
    @classmethod
    def from_crontab(cls, expr: str, timezone: str) -> Trigger: ...


class IntervalTriggerFactory(Protocol):
    def __call__(self, *, seconds: int, start_date: datetime, timezone: str) -> Trigger: ...


class _IntervalJob:
    id: str
    trigger: Trigger

    def __init__(self, trigger: Trigger) -> None:
        self.id = "b8370_interval"
        self.trigger = trigger


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def _instant(input_value: dict[str, Json]) -> datetime:
    text = string_value(input_value.get("start"), "start")
    return datetime.fromisoformat(text).astimezone(UTC)


def _cron_trigger(text: str) -> Trigger:
    factory = cast(CronTriggerFactory, CronTrigger)
    return factory.from_crontab(text, "UTC")


def _interval_trigger(seconds: int, start: datetime) -> Trigger:
    factory = cast(IntervalTriggerFactory, IntervalTrigger)
    probe = factory(seconds=seconds, start_date=start, timezone="UTC")
    period = _interval_seconds(_IntervalJob(probe))  # pyright: ignore[reportPrivateUsage]
    if period is None:
        raise ValueError("the interval trigger has no period")
    return factory(seconds=period, start_date=start + timedelta(seconds=period), timezone="UTC")


def _fires(trigger: Trigger, start: datetime) -> list[Json]:
    times: list[datetime] = []
    previous: datetime | None = None
    for _ in range(FIRE_COUNT):
        now = start if previous is None else previous
        current = trigger.get_next_fire_time(previous, now)
        if current is None:
            break
        times.append(current)
        previous = current
    return [cast(Json, moment.isoformat()) for moment in times]


def run(input_value: Json) -> Json:
    request = object_value(input_value, "input")
    start = _instant(request)
    cron = request.get("cron")
    interval = request.get("interval")
    if cron is not None and interval is not None:
        raise ValueError("give exactly one of cron or interval")
    if cron is not None:
        try:
            trigger = _cron_trigger(string_value(cron, "cron"))
        except ValueError as error:
            return {"error": str(error)}
        return {"fires": _fires(trigger, start)}
    if interval is not None:
        seconds = integer_value(interval, "interval")
        return {"fires": _fires(_interval_trigger(seconds, start), start)}
    raise ValueError("give exactly one of cron or interval")
