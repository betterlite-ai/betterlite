import asyncio
import gzip
import logging
import os
import time
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime
from functools import partial
from typing import Literal, NotRequired, TypedDict, cast

import httpx
from litellm._logging import verbose_logger
from litellm.integrations.datadog.datadog_metrics import DatadogMetricsLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.integrations.base_health_check import IntegrationHealthCheckStatus
from litellm.types.utils import StandardLoggingPayload
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

GZIP_MAGIC = b"\x1f\x8b"
SUCCESS_STATUSES = range(200, 300)
DEFAULT_CLOCK = 1_700_000_000.0
DEFAULT_STATUS = "200"
JSON_BODY = TypeAdapter(JsonValue)


class Reply(TypedDict):
    status: int


class Input(TypedDict):
    operation: Literal[
        "init",
        "add_metrics",
        "success_event",
        "failure_event",
        "flush",
        "health_check",
        "extract_tags",
    ]
    environment: dict[str, str]
    payload: NotRequired[dict[str, JsonValue]]
    start_time: NotRequired[float]
    end_time: NotRequired[float]
    api_call_start_time: NotRequired[float]
    status_code: NotRequired[str]
    batch_size: NotRequired[int]
    clock: NotRequired[float]
    replies: NotRequired[list[Reply]]


class Call(TypedDict):
    url: str
    headers: dict[str, str]
    body: JsonValue
    gzip: bool


class Report(TypedDict):
    batch: int
    delivered: int
    requeued: int


class Answer(TypedDict):
    series: list[dict[str, JsonValue]]
    requests: list[Call]
    warnings: int
    upload_url: str
    authenticated: bool
    batch_size: int
    flush_interval: int
    health: IntegrationHealthCheckStatus | None
    report: Report | None
    tags: list[str] | None


class InputError(ValueError):
    pass


class _Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


class _Response:
    def __init__(self, status: int) -> None:
        self.status_code = status

    def raise_for_status(self) -> None:
        if self.status_code in SUCCESS_STATUSES:
            return
        request = httpx.Request("POST", "https://api.datadoghq.com/api/v2/series")
        response = httpx.Response(self.status_code, request=request)
        raise httpx.HTTPStatusError(f"HTTP {self.status_code}", request=request, response=response)


class _Client:
    def __init__(self, replies: Sequence[Reply] | None) -> None:
        self.replies = [] if replies is None else [reply["status"] for reply in replies]
        self.calls: list[Call] = []

    async def post(self, url: str, content: bytes, headers: Mapping[str, str]) -> _Response:
        gzipped = content.startswith(GZIP_MAGIC)
        raw = gzip.decompress(content) if gzipped else content
        self.calls.append(
            {
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "body": JSON_BODY.validate_json(raw),
                "gzip": gzipped,
            }
        )
        if not self.replies:
            raise InputError("a scripted Datadog reply is missing")
        return _Response(self.replies.pop(0))


def _logger(input_value: Input, client: _Client) -> DatadogMetricsLogger:
    environment = input_value["environment"]
    logger = DatadogMetricsLogger(start_periodic_flush=False)
    if "batch_size" in input_value:
        logger.batch_size = input_value["batch_size"]
    elif environment.get("DEFAULT_BATCH_SIZE"):
        logger.batch_size = int(environment["DEFAULT_BATCH_SIZE"])
    logger.async_client = cast(AsyncHTTPHandler, client)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_metrics.py#L42
    return logger


def _kwargs(input_value: Input) -> dict[str, JsonValue | datetime]:
    kwargs: dict[str, JsonValue | datetime] = {}
    if "payload" in input_value:
        kwargs["standard_logging_object"] = input_value["payload"]
    if "start_time" in input_value:
        kwargs["start_time"] = datetime.fromtimestamp(input_value["start_time"], tz=UTC)
    if "end_time" in input_value:
        kwargs["end_time"] = datetime.fromtimestamp(input_value["end_time"], tz=UTC)
    if "api_call_start_time" in input_value:
        kwargs["api_call_start_time"] = datetime.fromtimestamp(
            input_value["api_call_start_time"], tz=UTC
        )
    return kwargs


async def _execute(input_value: Input, warnings: _Warnings) -> Answer:
    operation = input_value["operation"]
    client = _Client(input_value.get("replies"))
    logger = _logger(input_value, client)
    queue = cast("list[dict[str, JsonValue]]", logger.log_queue)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_batch_logger.py#L39
    kwargs = _kwargs(input_value)
    status_code = input_value.get("status_code")
    if operation in ("add_metrics", "flush"):
        logger._add_metrics_from_log(  # noqa: SLF001  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_metrics.py#L105
            log=cast(StandardLoggingPayload, input_value["payload"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
            kwargs=kwargs,
            status_code=DEFAULT_STATUS if status_code is None else status_code,
        )
    health: IntegrationHealthCheckStatus | None = None
    report: Report | None = None
    tags: list[str] | None = None
    match operation:
        case "init" | "add_metrics":
            pass
        case "success_event":
            await logger.async_log_success_event(kwargs, None, None, None)  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_metrics.py#L172
        case "failure_event":
            await logger.async_log_failure_event(kwargs, None, None, None)  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_metrics.py#L187
        case "flush":
            before = len(queue)
            await logger.flush_queue()  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_batch_logger.py#L54
            report = {"batch": before, "delivered": before - len(queue), "requeued": len(queue)}
        case "health_check":
            health = await logger.async_health_check()
        case "extract_tags":
            tags = logger._extract_tags(  # noqa: SLF001  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_metrics.py#L61
                cast(StandardLoggingPayload, input_value["payload"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
                status_code=status_code,
            )
    return {
        "series": queue,
        "requests": client.calls,
        "warnings": len([message for message in warnings.messages if "DD_API_KEY" in message]),
        "upload_url": logger.upload_url,
        "authenticated": bool(logger.dd_api_key),
        "batch_size": logger.batch_size,
        "flush_interval": int(logger.flush_interval),
        "health": health,
        "report": report,
        "tags": tags,
    }


def run(input_value: Input) -> Answer:
    clock = input_value.get("clock")
    # the case environment is the process environment for the whole run, as an operator would set it
    os.environ.clear()
    os.environ.update(input_value["environment"])
    warnings = _Warnings()
    verbose_logger.addHandler(warnings)
    time.time = partial(float, DEFAULT_CLOCK if clock is None else clock)
    return asyncio.run(_execute(input_value, warnings))
