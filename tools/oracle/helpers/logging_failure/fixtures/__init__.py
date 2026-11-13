from __future__ import annotations

import asyncio
import calendar
import datetime as dt
from collections.abc import Callable
from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast, get_type_hints
from unittest.mock import patch

from litellm._logging import session_id_var, trace_id_var
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import (
    StandardLoggingHiddenParams,
    StandardLoggingMetadata,
    StandardLoggingPayload,
    Usage,
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

# pyright: reportPrivateUsage=false, reportUnknownMemberType=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CallbackValue: TypeAlias = str | Callable[[], None] | CustomLogger


class Probe(CustomLogger):
    def __init__(self, spec: dict[str, Json], calls: list[dict[str, Json]]) -> None:
        super().__init__()
        self.name = string_value(spec.get("name"))
        flags = spec.get("flags")
        self.flags: set[str] = (
            {cast(str, flag) for flag in flags} if isinstance(flags, list) else set()
        )
        self.calls = calls
        self.stream_flag = False

    def redacts_messages_itself(self) -> bool:
        return "redacts_messages_itself" in self.flags

    def log_failure_event(
        self,
        kwargs: dict[str, object],
        response_obj: object | None = None,
        start_time: dt.datetime | None = None,
        end_time: dt.datetime | None = None,
    ) -> None:
        del response_obj, start_time, end_time
        self._record("sync", kwargs)

    async def async_log_failure_event(
        self,
        kwargs: dict[str, object],
        response_obj: object | None = None,
        start_time: dt.datetime | None = None,
        end_time: dt.datetime | None = None,
    ) -> None:
        del response_obj, start_time, end_time
        await asyncio.sleep(0)
        self._record("async", kwargs)

    def _record(self, method: str, kwargs: dict[str, object]) -> None:
        del kwargs
        entry: dict[str, Json] = {"logger": self.name, "method": method, "stream": self.stream_flag}
        self.calls.append(entry)
        if method == "async" and "fail_async" in self.flags:
            raise RuntimeError("the probe refused the event")
        if method == "sync" and "fail_sync" in self.flags:
            raise RuntimeError("the probe refused the event")


class Instances(type):
    def __instancecheck__(cls, instance: object) -> bool:
        return isinstance(instance, dt.datetime)


class Clock(metaclass=Instances):
    moment: dt.datetime

    @classmethod
    def now(cls, tz: dt.tzinfo | None = None) -> dt.datetime:
        del tz
        return cls.moment


def frozen_datetime(moment: dt.datetime) -> SimpleNamespace:
    return SimpleNamespace(datetime=type("Frozen", (Clock,), {"moment": moment}))


class Executor:
    def __init__(self, jobs: list[str]) -> None:
        self.jobs = jobs

    def submit(self, function: Callable[..., None], /, *args: object) -> None:
        self.jobs.append("sync_failure")
        function(*args)


class Cost:
    def __init__(self, recorders: Recorders) -> None:
        self.recorders = recorders

    def __call__(self, *args: object, **kwargs: object) -> float | None:
        del args, kwargs
        return self.recorders.cost


class Recorders:
    def __init__(self) -> None:
        self.calls: list[dict[str, Json]] = []
        self.jobs: list[str] = []
        self.cost: float | None = None


class Report:
    def __init__(
        self, error: Exception, traceback: str, start_time: dt.datetime, end_time: dt.datetime
    ) -> None:
        self.error = error
        self.traceback = traceback
        self.start_time = start_time
        self.end_time = end_time


def payload_builder(payload: dict[str, Json] | None) -> Callable[..., object]:
    def build(**kwargs: object) -> object:
        del kwargs
        return payload

    return build


def payload_record(raw: dict[str, Json]) -> dict[str, Json]:
    metadata = null_record(StandardLoggingMetadata)
    metadata.update(object_value(raw.get("metadata")))
    hidden = null_record(StandardLoggingHiddenParams)
    hidden.update(object_value(raw.get("hidden_params")))
    payload: dict[str, Json] = {}
    for key in StandardLoggingPayload.__required_keys__:
        payload[key] = raw.get(key)
    for key in StandardLoggingPayload.__optional_keys__:
        if key in raw:
            payload[key] = raw.get(key)
    payload["metadata"] = metadata
    payload["hidden_params"] = hidden
    return payload


def null_record(template: type[object]) -> dict[str, Json]:
    return cast("dict[str, Json]", dict.fromkeys(get_type_hints(template)))


def exception(value: dict[str, Json]) -> Exception:
    class_name = string_value(value.get("class_name")) or "APIError"
    message = string_value(value.get("message"))
    error = type(class_name, (Exception,), {})(message)
    attributes = cast("dict[str, object]", error.__dict__)
    attributes["message"] = message
    headers = value.get("headers")
    if isinstance(headers, dict):
        attributes["headers"] = headers
    return error


def callbacks(value: Json) -> list[CallbackValue] | None:
    if value is None:
        return None
    return cast("list[CallbackValue]", value)


def identity(params: dict[str, Json]) -> dict[str, Json]:
    return params


def micros(moment: dt.datetime) -> int:
    return calendar.timegm(moment.utctimetuple()) * 1_000_000 + moment.microsecond


def optional_moment(value: Json) -> dt.datetime | None:
    if isinstance(value, str):
        return dt.datetime.fromisoformat(value)
    return None


def optional_float(value: Json) -> float | None:
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    return None


def float_value(value: Json) -> float:
    converted = optional_float(value)
    if converted is None:
        raise ValueError("a cost must be a number")
    return converted


def list_value(value: Json) -> list[Json]:
    if isinstance(value, list):
        return cast("list[Json]", value)
    return []


def object_value(value: Json) -> dict[str, Json]:
    if isinstance(value, dict):
        return cast("dict[str, Json]", value)
    return {}


def string_value(value: Json) -> str:
    return value if isinstance(value, str) else ""


def plain_value(value: object) -> Json:
    if isinstance(value, Usage):
        return cast(Json, value.model_dump())
    if isinstance(value, dict):
        return {
            str(name): plain_value(entry)
            for name, entry in cast("dict[object, object]", value).items()
        }
    if isinstance(value, (list, tuple)):
        return [plain_value(entry) for entry in cast("list[object]", value)]
    if isinstance(value, (bool, int, float, str)) or value is None:
        return cast(Json, value)
    return None


DETAIL_KEYS: tuple[str, ...] = (
    "log_event_type",
    "traceback_exception",
    "original_response",
    "response_cost",
    "combined_usage_object",
    "standard_logging_object",
    "litellm_params",
)
CLAIM_KEYS: tuple[str, ...] = ("has_logged_sync_failure", "has_logged_async_failure")



def build_logging(init: dict[str, Json], settings: dict[str, Json]) -> Logging:
    dynamic = object_value(init.get("dynamic_callbacks"))
    flags = object_value(init.get("request_logging"))
    with (
        patch.object(
            litellm_logging,
            "get_litellm_params",
            return_value=object_value(init.get("litellm_params")),
        ),
        patch.object(litellm_logging, "scrub_sensitive_keys_in_metadata", identity),
        patch.object(
            litellm_logging, "_initialize_standard_callback_dynamic_params", return_value={}
        ),
        patch.object(litellm_logging, "get_trusted_callback_params", return_value=()),
        patch.object(Logging, "process_dynamic_callbacks", return_value=None),
        patch.object(
            Logging,
            "initialize_standard_built_in_tools_params",
            return_value=object_value(init.get("standard_built_in_tools_params")),
        ),
    ):
        return Logging(
            model=string_value(init.get("model")),
            messages=cast("list[AllMessageValues] | None", init.get("messages")),
            stream=cast("bool | None", init.get("stream")),
            call_type=string_value(init.get("call_type")),
            start_time=dt.datetime.fromisoformat(string_value(init.get("start_time"))),
            litellm_call_id=string_value(init.get("litellm_call_id")),
            function_id=string_value(init.get("function_id")),
            litellm_trace_id=cast("str | None", init.get("litellm_trace_id")),
            dynamic_failure_callbacks=callbacks(dynamic.get("failure")),
            dynamic_async_failure_callbacks=callbacks(dynamic.get("async_failure")),
            kwargs=object_value(init.get("kwargs")),
            log_raw_request_response=flags.get("log_raw_request_response") is True,
            supports_correlation_logging=settings.get("supports_correlation_logging") is not False,
        )


def build_report(failure: dict[str, Json], init: dict[str, Json]) -> Report:
    clock = dt.datetime.fromisoformat(string_value(init.get("start_time")))
    start = optional_moment(failure.get("start_time")) or clock
    end = optional_moment(failure.get("end_time")) or clock
    return Report(
        exception(object_value(failure.get("exception"))),
        string_value(failure.get("traceback")),
        start,
        end,
    )


def snapshot_details(logging_obj: Logging) -> Json:
    details = logging_obj.model_call_details
    output: dict[str, Json] = {}
    for key in DETAIL_KEYS:
        output[key] = plain_value(details.get(key))
    end_time = details.get("end_time")
    output["end_time_micros"] = micros(end_time) if isinstance(end_time, dt.datetime) else None
    for claim in CLAIM_KEYS:
        output[claim] = details.get(claim) is True
    return cast(Json, output)


def snapshot_context() -> Json:
    return cast(Json, {"trace_id": trace_id_var.get(), "session_id": session_id_var.get()})
