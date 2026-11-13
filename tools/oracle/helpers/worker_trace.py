import asyncio
import contextvars
from typing import TYPE_CHECKING, Final, Literal, TypeAlias, cast

if TYPE_CHECKING:
    from collections.abc import Callable, Coroutine

from litellm.constants import (
    LOGGING_WORKER_CONCURRENCY,
    LOGGING_WORKER_MAX_QUEUE_SIZE,
    LOGGING_WORKER_MAX_TIME_PER_COROUTINE,
    LOGGING_WORKER_TIMEOUT_SUMMARY_WINDOW_SECONDS,
)
from litellm.litellm_core_utils.logging_worker import LoggingWorker

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Outcome: TypeAlias = Literal["success", "failure"]

_CONTEXT: Final = contextvars.ContextVar[str]("worker_trace_context")


class InputError(ValueError):
    pass


class CallbackError(RuntimeError):
    pass


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return integer(value, name)


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return value


class Recorder:
    def __init__(self) -> None:
        self.completed: list[str] = []
        self.failed: list[str] = []
        self.contexts: dict[str, str] = {}
        self.accepted = 0
        self.flushes: list[Json] = []


class ScriptedCallback:
    def __init__(self, name: str, outcome: Outcome, context: str, recorder: Recorder) -> None:
        self.name = name
        self.outcome = outcome
        self.context = context
        self.recorder = recorder

    async def run(self) -> None:
        observed = _CONTEXT.get()
        self.recorder.contexts[self.name] = observed
        await asyncio.sleep(0)
        if self.outcome == "failure":
            self.recorder.failed.append(self.name)
            raise CallbackError(self.name)
        self.recorder.completed.append(self.name)


def build_worker(settings: dict[str, Json]) -> LoggingWorker:
    timeout_ms = optional_integer(
        settings.get("timeout_milliseconds"), "settings.timeout_milliseconds"
    )
    max_queue_size = optional_integer(settings.get("max_queue_size"), "settings.max_queue_size")
    concurrency = optional_integer(settings.get("concurrency"), "settings.concurrency")
    window_ms = optional_integer(
        settings.get("window_milliseconds"), "settings.window_milliseconds"
    )
    return LoggingWorker(
        timeout=(
            LOGGING_WORKER_MAX_TIME_PER_COROUTINE
            if timeout_ms is None
            else float(timeout_ms) / 1000.0
        ),
        max_queue_size=(
            LOGGING_WORKER_MAX_QUEUE_SIZE if max_queue_size is None else max_queue_size
        ),
        concurrency=LOGGING_WORKER_CONCURRENCY if concurrency is None else concurrency,
        timeout_summary_window=(
            LOGGING_WORKER_TIMEOUT_SUMMARY_WINDOW_SECONDS
            if window_ms is None
            else float(window_ms) / 1000.0
        ),
    )


def settings_json(worker: LoggingWorker) -> Json:
    return {
        "max_queue_size": worker.max_queue_size,
        "concurrency": worker.concurrency,
        "timeout_milliseconds": int(worker.timeout * 1000),
        "timeout_summary_window_milliseconds": int(worker.timeout_summary_window * 1000),
    }


def sorted_json(values: list[str]) -> list[Json]:
    converted: list[Json] = []
    converted.extend(sorted(values))
    return converted


def flush_entry(recorder: Recorder) -> Json:
    completed = len(recorder.completed)
    failed = len(recorder.failed)
    return {
        "completed": completed,
        "failed": failed,
        "pending": recorder.accepted - completed - failed,
    }


def enqueue(worker: LoggingWorker, recorder: Recorder, event: dict[str, Json]) -> None:
    name = string_value(event.get("name"), "event.name")
    context = string_value(event.get("context"), "event.context")
    outcome_text = string_value(event.get("outcome"), "event.outcome")
    outcome: Outcome
    if outcome_text == "success":
        outcome = "success"
    elif outcome_text == "failure":
        outcome = "failure"
    else:
        raise InputError(f"unknown event outcome: {outcome_text}")
    callback = ScriptedCallback(name, outcome, context, recorder)
    enqueue_callback = cast(
        "Callable[[Coroutine[object, object, None]], None]",
        worker.ensure_initialized_and_enqueue,
    )
    token = _CONTEXT.set(context)
    try:
        enqueue_callback(callback.run())
    finally:
        _CONTEXT.reset(token)
    recorder.accepted += 1


async def run_steps(input_value: dict[str, Json]) -> Json:
    settings = object_value(input_value.get("settings", {}), "settings")
    worker = build_worker(settings)
    recorder = Recorder()
    stopped = False
    for step_value in list_value(input_value.get("steps", []), "steps"):
        step = object_value(step_value, "step")
        operation = string_value(step.get("op"), "step.op")
        if operation == "enqueue":
            if stopped:
                raise InputError("cannot enqueue after stop")
            for event_value in list_value(step.get("events", []), "events"):
                enqueue(worker, recorder, object_value(event_value, "event"))
        elif operation == "flush":
            if stopped:
                raise InputError("cannot flush after stop")
            await worker.flush()
            recorder.flushes.append(flush_entry(recorder))
        elif operation == "stop":
            await worker.stop()
            stopped = True
        else:
            raise InputError(f"unknown step operation: {operation}")
    if not stopped:
        await worker.stop()
    contexts: dict[str, Json] = dict(sorted(recorder.contexts.items()))
    result: dict[str, Json] = {
        "settings": settings_json(worker),
        "accepted": recorder.accepted,
        "completed": sorted_json(recorder.completed),
        "failed": sorted_json(recorder.failed),
        "contexts": contexts,
        "flushes": recorder.flushes,
        "status": "stopped",
    }
    return result


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(run_steps(input_value))
