import os
import time as time_module
from datetime import UTC, datetime, timedelta, tzinfo
from typing import TypeAlias
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils import duration_parser
from litellm.proxy.common_utils import timezone_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)
_MICROSECOND = timedelta(microseconds=1)


class _FixedDatetime:
    def __init__(self, instant: datetime) -> None:
        self.instant = instant

    def now(self, tz: tzinfo | None = None) -> datetime:
        return self.instant if tz is None else self.instant.astimezone(tz)

    @staticmethod
    def strptime(text: str, fmt: str) -> datetime:
        return datetime.strptime(text, fmt)  # noqa: DTZ007 - LiteLLM reads only the wall-clock fields


class _WarningLog:
    def __init__(self) -> None:
        self.calls: list[tuple[object, ...]] = []

    def __call__(self, *args: object, **_kwargs: object) -> None:
        self.calls.append(args)


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(value: Json, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _moment(value: Json, name: str) -> datetime:
    parsed = datetime.fromisoformat(_string(value, name))
    if parsed.tzinfo is None:
        raise ValueError(f"{name} must carry a UTC offset")
    return parsed


def _micros(instant: datetime) -> int:
    return (instant.astimezone(UTC) - _EPOCH) // _MICROSECOND


def _settings_json(settings: timezone_utils.BudgetResetSettings) -> dict[str, Json]:
    return {
        "timezone": settings.timezone,
        "reset_time_of_day": settings.reset_time_of_day.isoformat(),
    }


def _use_calendar_zone(name: str | None) -> None:
    os.environ["TZ"] = "UTC" if name is None else name
    time_module.tzset()


def _apply_settings(settings: dict[str, Json]) -> None:
    setattr(litellm, "timezone", settings.get("timezone"))  # noqa: B010 - the module setting has no declared attribute
    setattr(litellm, "budget_reset_time", settings.get("budget_reset_time"))  # noqa: B010


def _call_category(error: Exception) -> str:
    if isinstance(error, ValueError):
        return "value_error"
    if isinstance(error, OverflowError):
        return "overflow_error"
    return "other_error"


def _rejected(error: Exception) -> dict[str, Json]:
    return {"ok": False, "error": _call_category(error), "warned": False}


def _run_call(
    call: dict[str, Json],
    reset_now: datetime,
    calendar_now: datetime,
    calendar_timezone: str | None,
) -> dict[str, Json]:
    function = _string(call.get("function"), "function")
    settings = _object(call.get("settings", {}), "settings")
    log = _WarningLog()
    try:
        if function == "parse_budget_reset_time":
            parsed = timezone_utils.parse_budget_reset_time(call.get("value"))
            value: Json = parsed.isoformat()
            warned = False
        elif function == "get_budget_reset_settings":
            _apply_settings(settings)
            value = _settings_json(timezone_utils.get_budget_reset_settings())
            warned = False
        elif function == "compute_budget_reset_at":
            _apply_settings(settings)
            with (
                patch.object(timezone_utils, "datetime", _FixedDatetime(reset_now)),
                patch.object(
                    duration_parser.verbose_logger,  # pyright: ignore[reportPrivateImportUsage]
                    "warning",
                    log,
                ),
            ):
                current = timezone_utils.get_budget_reset_settings()
                computed = timezone_utils.compute_budget_reset_at(
                    _string(call.get("duration"), "duration"), current
                )
            value = _micros(computed)
            warned = bool(log.calls)
        elif function == "get_budget_reset_time":
            _apply_settings(settings)
            with (
                patch.object(timezone_utils, "datetime", _FixedDatetime(reset_now)),
                patch.object(
                    duration_parser.verbose_logger,  # pyright: ignore[reportPrivateImportUsage]
                    "warning",
                    log,
                ),
            ):
                current_time = timezone_utils.get_budget_reset_time(
                    _string(call.get("duration"), "duration")
                )
            value = _micros(current_time)
            warned = bool(log.calls)
        elif function == "is_persistable":
            _apply_settings(settings)
            _use_calendar_zone(calendar_timezone)
            check = timezone_utils._is_persistable_budget_duration  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            with (
                patch.object(
                    duration_parser.time_module,  # pyright: ignore[reportPrivateImportUsage]
                    "time",
                    return_value=calendar_now.timestamp(),
                ),
                patch.object(timezone_utils, "datetime", _FixedDatetime(reset_now)),
            ):
                persistable = check(_string(call.get("duration"), "duration"))
            value = persistable
            warned = False
        elif function == "budget_duration_error":
            _apply_settings(settings)
            _use_calendar_zone(calendar_timezone)
            with (
                patch.object(
                    duration_parser.time_module,  # pyright: ignore[reportPrivateImportUsage]
                    "time",
                    return_value=calendar_now.timestamp(),
                ),
                patch.object(timezone_utils, "datetime", _FixedDatetime(reset_now)),
            ):
                error_text = timezone_utils.budget_duration_error(
                    _optional_string(call.get("duration"), "duration")
                )
            value = None if error_text is None else "invalid"
            warned = False
        else:
            raise ValueError(f"unknown function {function}")
    except Exception as error:  # noqa: BLE001 - every LiteLLM failure is an outcome the case compares
        return _rejected(error)
    return {"ok": True, "value": value, "warned": warned}


def run(input_value: dict[str, Json]) -> Json:
    reset_now = _moment(input_value.get("reset_now"), "reset_now")
    calendar_now = _moment(input_value.get("calendar_now"), "calendar_now")
    calendar_timezone = _optional_string(
        input_value.get("calendar_timezone"), "calendar_timezone"
    )
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [
        _run_call(_object(call, "call"), reset_now, calendar_now, calendar_timezone)
        for call in calls
    ]
