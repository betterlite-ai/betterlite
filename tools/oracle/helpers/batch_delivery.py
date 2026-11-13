import asyncio
import logging
from collections.abc import Sequence
from typing import Final, Literal, TypeAlias, cast

import httpx
from litellm._logging import verbose_logger
from litellm.integrations.batch_utils import (
    BatchSendCancelled,
    is_retryable_status,
    requeue_after_http_error,
    send_batch_with_413_split,
    undelivered_after_http_error,
)
from litellm.integrations.custom_batch_logger import CustomBatchLogger
from litellm.llms.custom_httpx.http_handler import MaskedHTTPStatusError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
FlushReply: TypeAlias = Literal["delivered", "failed"]
Policy: TypeAlias = Literal["undelivered", "requeue"]

_URL: Final = "https://batch-delivery.invalid/ingest"


class InputError(ValueError):
    pass


class DeliveryError(RuntimeError):
    pass


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return value


def integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return integer(value, name)


def event_ids(value: Json, name: str) -> list[str]:
    events: list[str] = []
    for item in list_value(value, name):
        if not isinstance(item, str):
            raise InputError(f"{name} entries must be strings")
        events.append(item)
    return events


def as_json(values: list[str]) -> list[Json]:
    converted: list[Json] = []
    converted.extend(values)
    return converted


def status_rows(value: Json) -> list[Json]:
    scenario = object_value(value, "statuses")
    events = event_ids(scenario.get("events"), "statuses.events")
    rows: list[Json] = []
    for code in list_value(scenario.get("codes"), "statuses.codes"):
        status = integer(code, "statuses.codes entry")
        undelivered = undelivered_after_http_error(
            events, status, "BatchDelivery", "classification"
        )
        requeued = requeue_after_http_error(events, status, "BatchDelivery", "classification")
        row: dict[str, Json] = {
            "code": status,
            "retryable": is_retryable_status(status),
            "undelivered": as_json(list(undelivered)),
            "requeued": as_json(list(requeued)),
        }
        rows.append(row)
    return rows


def http_response(status: int) -> httpx.Response:
    return httpx.Response(status, request=httpx.Request("POST", _URL), text="")


def status_error(status: int) -> MaskedHTTPStatusError:
    request = httpx.Request("POST", _URL)
    response = httpx.Response(status, request=request, text="")
    return MaskedHTTPStatusError(
        httpx.HTTPStatusError("the scripted transport failed", request=request, response=response)
    )


class ScriptedSend:
    def __init__(self, scenario: dict[str, Json]) -> None:
        self.script = list_value(scenario.get("script"), "send.script")
        self.max_events = optional_integer(
            scenario.get("max_events_per_request"), "send.max_events_per_request"
        )
        self.oversized = event_ids(scenario.get("oversized", []), "send.oversized")
        self.unserializable = event_ids(scenario.get("unserializable", []), "send.unserializable")
        codes = list_value(scenario.get("success_status_codes"), "send.success_status_codes")
        self.success_status_codes = [
            integer(code, "send.success_status_codes entry") for code in codes
        ]
        self.attempts: list[Json] = []
        self.delivered: list[str] = []

    def exceeds_limits(self, batch: Sequence[str]) -> bool:
        if any(event in self.unserializable for event in batch):
            raise InputError("the scripted record cannot be serialized")
        if any(event in self.oversized for event in batch):
            return True
        return self.max_events is not None and len(batch) > self.max_events

    async def send(self, batch: Sequence[str]) -> httpx.Response:
        if not self.script:
            raise InputError("the send script is empty")
        step = object_value(self.script.pop(0), "send.script entry")
        outcome = step.get("outcome")
        status = step.get("status")
        numeric_status = integer(status, "send.script.status") if status is not None else None
        events = list(batch)
        if outcome == "response" and numeric_status in self.success_status_codes:
            self.delivered.extend(events)
        attempt: dict[str, Json] = {
            "events": as_json(events),
            "outcome": outcome if isinstance(outcome, str) else "error",
            "status": numeric_status,
        }
        self.attempts.append(attempt)
        if outcome == "response" and numeric_status is not None:
            return http_response(numeric_status)
        if outcome == "status_error" and numeric_status is not None:
            raise status_error(numeric_status)
        if outcome == "error":
            raise DeliveryError("the scripted transport failed")
        if outcome == "cancel":
            raise asyncio.CancelledError
        raise InputError(f"unknown send outcome: {outcome}")


async def send_section(value: Json) -> Json:
    scenario = object_value(value, "send")
    events = event_ids(scenario.get("events"), "send.events")
    scripted = ScriptedSend(scenario)
    policy = scenario.get("policy", "undelivered")
    if policy not in ("undelivered", "requeue"):
        raise InputError(f"unknown send policy: {policy}")
    handler = (
        undelivered_after_http_error if policy == "undelivered" else requeue_after_http_error
    )
    try:
        remaining: tuple[str, ...] = await send_batch_with_413_split(
            batch=events,
            send_batch=scripted.send,
            exceeds_limits=scripted.exceeds_limits,
            success_status_codes=frozenset(scripted.success_status_codes),
            integration_name="BatchDelivery",
            drop_error_message="batch delivery dropped a single event the destination rejected",
            non_success_handler=handler,
        )
        cancelled = False
    except BatchSendCancelled as error:  # pyright: ignore[reportUnknownVariableType]
        remaining = cast("BatchSendCancelled[str]", error).undelivered
        cancelled = True
    result: dict[str, Json] = {
        "attempts": scripted.attempts,
        "delivered": as_json(scripted.delivered),
        "undelivered": as_json(list(remaining)),
        "cancelled": cancelled,
    }
    return result


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


class ScriptedLogger(CustomBatchLogger):
    preserve_events_added_during_flush = True

    def __init__(self, settings: dict[str, Json]) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]
            flush_lock=asyncio.Lock(),
            batch_size=optional_integer(settings.get("batch_size"), "flush.settings.batch_size"),
            flush_interval=optional_integer(
                settings.get("flush_interval_seconds"), "flush.settings.flush_interval_seconds"
            ),
            max_queue_size=optional_integer(
                settings.get("max_queue_size"), "flush.settings.max_queue_size"
            ),
        )
        self.script: list[FlushReply] = []
        self.attempts: list[list[str]] = []
        self.outcomes: list[FlushReply] = []
        self.during: list[str] = []

    async def async_send_batch(self, *_args: object, **_kwargs: object) -> None:
        if not self.script:
            raise InputError("the flush script is empty")
        outcome = self.script.pop(0)
        queue = cast("list[str]", self.log_queue)
        self.attempts.append(list(queue))
        self.outcomes.append(outcome)
        queue.extend(self.during)
        if outcome == "failed":
            raise DeliveryError("the scripted delivery failed")


def flush_reply(value: Json) -> FlushReply:
    outcome = object_value(value, "flush script entry").get("outcome")
    if outcome == "delivered":
        return "delivered"
    if outcome == "failed":
        return "failed"
    raise InputError(f"unknown flush reply: {outcome}")


def flush_dropped(
    sent: list[str],
    during: list[str],
    outcome: FlushReply | Literal["skipped"],
    limit: int,
) -> int:
    if outcome == "failed":
        return max(0, len(sent) + len(during) - limit)
    return 0


async def flush_section(value: Json) -> Json:
    scenario = object_value(value, "flush")
    settings = object_value(scenario.get("settings", {}), "flush.settings")
    logger = ScriptedLogger(settings)
    recorder = WarningRecorder()
    verbose_logger.addHandler(recorder)
    flushes: list[Json] = []
    delivered: list[str] = []
    try:
        for step_value in list_value(scenario.get("steps"), "flush.steps"):
            step = object_value(step_value, "flush step")
            operation = step.get("op")
            if operation == "enqueue":
                cast("list[str]", logger.log_queue).extend(
                    event_ids(step.get("events"), "flush step events")
                )
            elif operation == "flush":
                script = list_value(step.get("script", []), "flush script")
                during = event_ids(step.get("during", []), "flush step during")
                logger.script = [flush_reply(entry) for entry in script]
                logger.during = during
                logger.attempts.clear()
                logger.outcomes.clear()
                await logger.flush_queue()
                sent = logger.attempts[0] if logger.attempts else []
                outcome: FlushReply | Literal["skipped"] = (
                    logger.outcomes[0] if logger.outcomes else "skipped"
                )
                if outcome == "delivered":
                    delivered.extend(sent)
                entry: dict[str, Json] = {
                    "sent": as_json(sent),
                    "outcome": outcome,
                    "queue": as_json(list(cast("list[str]", logger.log_queue))),
                    "dropped_oldest": flush_dropped(
                        sent, during, outcome, logger.max_queue_size
                    ),
                }
                flushes.append(entry)
            else:
                raise InputError(f"unknown flush operation: {operation}")
    finally:
        verbose_logger.removeHandler(recorder)
    warnings = len([message for message in recorder.messages if "max_queue_size" in message])
    settings_out: dict[str, Json] = {
        "batch_size": logger.batch_size,
        "flush_interval_seconds": int(logger.flush_interval),
        "max_queue_size": logger.max_queue_size,
    }
    result: dict[str, Json] = {
        "settings": settings_out,
        "flushes": flushes,
        "queue": as_json(list(cast("list[str]", logger.log_queue))),
        "delivered": as_json(delivered),
        "warnings": warnings,
    }
    return result


async def run_sections(input_value: dict[str, Json]) -> Json:
    statuses: Json = status_rows(input_value["statuses"]) if "statuses" in input_value else None
    send: Json = await send_section(input_value["send"]) if "send" in input_value else None
    flush: Json = await flush_section(input_value["flush"]) if "flush" in input_value else None
    result: dict[str, Json] = {"statuses": statuses, "send": send, "flush": flush}
    return result


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(run_sections(input_value))
