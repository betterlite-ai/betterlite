import asyncio
import functools
import gzip
import json
import time
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import asdict, dataclass
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.integrations.newrelic.newrelic_metrics import NewRelicMetricsLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.integrations.newrelic import NewRelicMetricRecord
from litellm.types.utils import StandardLoggingPayload

if TYPE_CHECKING:
    from collections.abc import Coroutine

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
AsyncEvent: TypeAlias = Callable[
    [dict[str, StandardLoggingPayload], object, object, object], Awaitable[None]
]

SUCCESS_LOW = 200
SUCCESS_HIGH = 300
GZIP_MAGIC = b"\x1f\x8b"
SCRIPTED_KEY = "team-key"
FLUSH_INTERVAL_SECONDS = 3600.0
QUEUE_PREVIEW = 3


class InputError(ValueError):
    pass


def _object(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return cast(Fields, value)


def _items(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return cast("list[Json]", value)


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def _optional_string(value: Json, name: str) -> str | None:
    if value is None or isinstance(value, str):
        return value
    raise InputError(f"{name} must be a string or null")


def _float(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise InputError(f"{name} must be a number")
    return float(value)


def _optional_float(value: Json, name: str) -> float | None:
    if value is None:
        return None
    return _float(value, name)


def _to_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


@dataclass(frozen=True, slots=True)
class _Reply:
    status: int
    network: bool


def _reply(value: Json) -> _Reply:
    fields = _object(value, "reply")
    network = fields.get("network")
    if network is True:
        return _Reply(status=0, network=True)
    return _Reply(status=_integer(fields.get("status", SUCCESS_LOW), "reply.status"), network=False)


def _record(fields: Fields) -> NewRelicMetricRecord:
    return NewRelicMetricRecord(
        team_id=_string(fields.get("team_id"), "team_id"),
        team_alias=_string(fields.get("team_alias"), "team_alias"),
        model_group=_string(fields.get("model_group"), "model_group"),
        model=_string(fields.get("model"), "model"),
        custom_llm_provider=_string(fields.get("custom_llm_provider"), "custom_llm_provider"),
        status=_string(fields.get("status"), "status"),
        response_cost=_float(fields.get("response_cost"), "response_cost"),
        prompt_tokens=_integer(fields.get("prompt_tokens"), "prompt_tokens"),
        completion_tokens=_integer(fields.get("completion_tokens"), "completion_tokens"),
        total_tokens=_integer(fields.get("total_tokens"), "total_tokens"),
        duration_ms=_float(fields.get("duration_ms"), "duration_ms"),
        team_max_budget=_optional_float(fields.get("team_max_budget"), "team_max_budget"),
        team_spend=_optional_float(fields.get("team_spend"), "team_spend"),
    )


def _records(value: Json) -> list[NewRelicMetricRecord]:
    records: list[NewRelicMetricRecord] = []
    for item in _items(value, "records"):
        fields = _object(item, "record")
        repeat = _integer(fields.get("repeat", 1), "repeat")
        records.extend(_record(fields) for _copy in range(repeat))
    return records


class _ScriptedClock:
    def __init__(self, seconds: Sequence[float]) -> None:
        self.remaining = list(seconds)
        self.fallback = self.remaining[-1] if self.remaining else 0.0

    def __call__(self) -> float:
        if self.remaining:
            return self.remaining.pop(0)
        return self.fallback


class _ScriptedSleeps:
    def __init__(self) -> None:
        self.recorded: list[float] = []
        self._real = asyncio.sleep

    async def __call__(self, delay: float, _result: object = None) -> None:
        if delay >= FLUSH_INTERVAL_SECONDS:
            await self._real(delay)
            return
        self.recorded.append(float(delay))


class _LoopProxy:
    def __init__(self) -> None:
        self.tasks: list[asyncio.Future[None]] = []

    def create_task(self, coro: "Coroutine[object, object, None]") -> asyncio.Future[None]:
        task = asyncio.ensure_future(coro)
        self.tasks.append(task)
        return task


class _Response:
    def __init__(self, status: int) -> None:
        self.status_code = status


class _Client:
    def __init__(
        self,
        replies: list[_Reply],
        during_send: NewRelicMetricRecord | None,
    ) -> None:
        self.replies = replies
        self.during_send = during_send
        self.logger: NewRelicMetricsLogger | None = None
        self.calls: list[Json] = []

    def attach(self, logger: NewRelicMetricsLogger) -> None:
        self.logger = logger

    def _append_during_send(self) -> None:
        if self.during_send is None or self.logger is None:
            return
        queue = cast("list[NewRelicMetricRecord]", self.logger.log_queue)
        queue.append(self.during_send)
        self.during_send = None

    def _record(self, url: str, data: bytes, headers: Mapping[str, str] | None) -> Json:
        gzipped = data.startswith(GZIP_MAGIC)
        raw = gzip.decompress(data) if gzipped else data
        return {
            "url": url,
            "headers": {name.lower(): value for name, value in (headers or {}).items()},
            "body": _to_json(cast(Json, json.loads(raw))),
            "gzip": gzipped,
        }

    async def post(
        self,
        url: str,
        data: bytes = b"",
        headers: Mapping[str, str] | None = None,
    ) -> _Response:
        self.calls.append(self._record(url, data, headers))
        self._append_during_send()
        if not self.replies:
            raise InputError("a scripted Metric API reply is missing")
        reply = self.replies.pop(0)
        if reply.network:
            raise ConnectionError("scripted network failure")
        if SUCCESS_LOW <= reply.status < SUCCESS_HIGH:
            return _Response(reply.status)
        request = httpx.Request("POST", url)
        raise httpx.HTTPStatusError(
            f"HTTP {reply.status}",
            request=request,
            response=httpx.Response(reply.status, request=request),
        )


def _clock_values(value: Json) -> list[float]:
    values = _items(value, "clock")
    return [_float(entry, "clock entry") for entry in values]


def _drain(logger: NewRelicMetricsLogger) -> list[asyncio.Future[None]]:
    loop = _LoopProxy()
    with patch.object(asyncio, "get_running_loop", lambda: loop):
        logger.stop()
    return loop.tasks


def _queue(logger: NewRelicMetricsLogger) -> Fields:
    queue = cast("list[NewRelicMetricRecord]", logger.log_queue)
    head = [_to_json(asdict(record)) for record in queue[:QUEUE_PREVIEW]]
    tail = [_to_json(asdict(record)) for record in queue[-QUEUE_PREVIEW:]]
    return {"length": len(queue), "head": head, "tail": tail}


async def _run(input_value: Fields) -> Json:
    operation = _string(input_value.get("operation"), "operation")
    replies = [_reply(entry) for entry in _items(input_value.get("replies"), "replies")]
    during = input_value.get("during_send")
    during_record = None if during is None else _record(_object(during, "during_send"))
    sleeps = _ScriptedSleeps()
    client = _Client(replies, during_record)
    clock = _ScriptedClock(_clock_values(input_value.get("clock")))
    logger = NewRelicMetricsLogger(
        newrelic_api_key=_string(input_value.get("api_key", SCRIPTED_KEY), "api_key"),
        newrelic_region=_optional_string(input_value.get("region"), "region"),
    )
    # gzip.compress reads time.time() without an mtime, which would eat a scripted clock value
    compress = functools.partial(gzip.compress, mtime=0)
    with (
        patch.object(time, "time", clock),
        patch.object(gzip, "compress", compress),
        patch.object(asyncio, "sleep", sleeps),
    ):
        logger.last_flush_time = clock()
        logger.flush_interval = FLUSH_INTERVAL_SECONDS
        logger.async_client = cast(AsyncHTTPHandler, client)
        client.attach(logger)
        queued = cast("list[NewRelicMetricRecord]", logger.log_queue)
        queued.extend(_records(input_value.get("records")))
        scheduled: bool | None = None
        delivered: bool | None = None
        match operation:
            case "send_batch":
                delivered = await logger.async_send_batch()
            case "flush":
                await logger.flush_queue()
            case "drain":
                tasks = _drain(logger)
                for task in tasks:
                    await task
                scheduled = True
            case "post_stop":
                tasks = _drain(logger)
                for task in tasks:
                    await task
                scheduled = True
                payload = cast(StandardLoggingPayload, input_value.get("payload"))
                log_event = cast(AsyncEvent, logger.async_log_success_event)
                await log_event({"standard_logging_object": payload}, None, None, None)
            case _:
                raise InputError(f"unknown newrelic metric operation {operation}")
        return {
            "requests": _to_json(client.calls),
            "queue": _queue(logger),
            "sleeps": _to_json(sleeps.recorded),
            "scheduled": scheduled,
            "delivered": delivered,
        }


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(_run(input_value))
