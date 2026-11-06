import os
import time as time_module
from datetime import UTC, datetime, time, timedelta
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils import duration_parser

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)
_MICROSECOND = timedelta(microseconds=1)


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(value: Json, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _microseconds(moment: datetime) -> int:
    return (moment.astimezone(UTC) - _EPOCH) // _MICROSECOND


def _set_local_zone(name: str | None) -> None:
    os.environ["TZ"] = "UTC" if name is None else name
    time_module.tzset()


def _duration_seconds(duration: str, now: datetime, timezone_name: str | None) -> int:
    _set_local_zone(timezone_name)
    with patch.object(
        duration_parser.time_module,  # pyright: ignore[reportUnknownMemberType, reportPrivateImportUsage]
        "time",
        return_value=now.timestamp(),
    ):
        seconds: int = duration_parser.duration_in_seconds(duration)  # pyright: ignore[reportUnknownMemberType]
        return seconds


def _reset_instant(
    duration: str,
    now: datetime,
    timezone_name: str | None,
    reset_time_of_day: time,
) -> int:
    result: datetime = duration_parser.get_next_standardized_reset_time(  # pyright: ignore[reportUnknownMemberType]
        duration,
        now,
        cast(str, timezone_name),
        reset_time_of_day,
    )
    return _microseconds(result)


def _run_call(call: dict[str, Json]) -> dict[str, Json]:
    function = _string(call.get("function"), "function")
    duration = _string(call.get("duration"), "duration")
    now = datetime.fromisoformat(_string(call.get("now"), "now"))
    if now.tzinfo is None:
        raise ValueError("now must carry a UTC offset")
    timezone_name = _optional_string(call.get("timezone"), "timezone")
    reset_time_of_day = time.fromisoformat(
        _string(call.get("reset_time_of_day", "00:00:00"), "reset_time_of_day")
    )
    try:
        if function == "duration_in_seconds":
            value = _duration_seconds(duration, now, timezone_name)
        elif function == "reset":
            value = _reset_instant(duration, now, timezone_name, reset_time_of_day)
        else:
            raise ValueError(f"unknown function {function}")
    except Exception:  # noqa: BLE001 - every LiteLLM failure is the outcome the case compares
        return {"ok": False}
    else:
        return {"ok": True, "value": value}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_run_call(_object(call, "call")) for call in calls]
