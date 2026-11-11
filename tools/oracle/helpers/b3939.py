import asyncio
import gzip
import json
import os
import time
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm._logging import verbose_logger
from litellm.integrations.datadog import datadog_metrics
from litellm.integrations.datadog.datadog_metrics import DatadogMetricsLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Kwargs: TypeAlias = dict[str, object]

GZIP_MAGIC = b"\x1f\x8b"
SUCCESS_STATUSES = range(200, 300)
DEFAULT_CLOCK = 1_700_000_000.0
DEFAULT_STATUS = "200"


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


def _optional_int(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise InputError(f"{name} must be an integer")
    return value


def _integer(value: Json, name: str) -> int:
    result = _optional_int(value, name)
    if result is None:
        raise InputError(f"{name} must be an integer")
    return result


def _string_map(value: Json, name: str) -> dict[str, str]:
    fields = _object(value, name)
    result: dict[str, str] = {}
    for key, item in fields.items():
        result[key] = _string(item, f"{name}.{key}")
    return result


def _to_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def _plain(value: object) -> Json:
    if isinstance(value, Mapping):
        fields = cast("Mapping[object, object]", value)
        return {str(key): _plain(item) for key, item in fields.items()}
    if isinstance(value, (list, tuple)):
        return [_plain(item) for item in cast("tuple[object, ...]", value)]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return cast(Json, value)
    raise InputError(f"cannot represent a {type(value).__name__} in a helper answer")


def _moment(seconds: float) -> datetime:
    return datetime.fromtimestamp(seconds, tz=UTC)


class _Now:
    def __init__(self, moment: datetime) -> None:
        self._moment = moment

    def now(self) -> datetime:
        return self._moment


class _Warnings:
    def __init__(self) -> None:
        self.messages: list[str] = []

    def __call__(self, message: str, *args: object, **kwargs: object) -> None:
        del kwargs
        self.messages.append(message % args if args else message)


class _Response:
    def __init__(self, status: int) -> None:
        self.status_code = status

    def raise_for_status(self) -> None:
        if self.status_code in SUCCESS_STATUSES:
            return
        request = httpx.Request("POST", "https://api.datadoghq.com/api/v2/series")
        response = httpx.Response(self.status_code, request=request)
        raise httpx.HTTPStatusError(
            f"HTTP {self.status_code}", request=request, response=response
        )


class _Client:
    def __init__(self, replies: Sequence[int]) -> None:
        self.replies = list(replies)
        self.calls: list[Json] = []

    async def post(self, url: str, content: bytes, headers: Mapping[str, str]) -> _Response:
        gzipped = content.startswith(GZIP_MAGIC)
        raw = gzip.decompress(content) if gzipped else content
        self.calls.append(
            {
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "body": cast(Json, json.loads(raw)),
                "gzip": gzipped,
            }
        )
        if not self.replies:
            raise InputError("a scripted Datadog reply is missing")
        return _Response(self.replies.pop(0))


def _statuses(value: Json) -> list[int]:
    statuses: list[int] = []
    for item in _items(value, "replies"):
        fields = _object(item, "reply")
        statuses.append(_integer(fields.get("status", 200), "reply.status"))
    return statuses


def _build_logger(
    batch_size: int | None, environment: Mapping[str, str]
) -> DatadogMetricsLogger:
    logger = DatadogMetricsLogger(start_periodic_flush=False)
    size = batch_size if batch_size is not None else _environment_batch_size(environment)
    if size is not None:
        logger.batch_size = size
    return logger


def _environment_batch_size(environment: Mapping[str, str]) -> int | None:
    text = environment.get("DEFAULT_BATCH_SIZE")
    if text is None or text == "":
        return None
    return int(text)


def _payload(input_value: Fields) -> "StandardLoggingPayload":
    return cast("StandardLoggingPayload", _object(input_value.get("payload"), "payload"))


def _log_kwargs(input_value: Fields) -> Kwargs:
    kwargs: Kwargs = {"standard_logging_object": _payload(input_value)}
    start = _optional_float(input_value.get("start_time"), "start_time")
    if start is not None:
        kwargs["start_time"] = _moment(start)
    end = _optional_float(input_value.get("end_time"), "end_time")
    if end is not None:
        kwargs["end_time"] = _moment(end)
    api_start = _optional_float(input_value.get("api_call_start_time"), "api_call_start_time")
    if api_start is not None:
        kwargs["api_call_start_time"] = _moment(api_start)
    return kwargs


def _seed(input_value: Fields, logger: DatadogMetricsLogger) -> None:
    kwargs = _log_kwargs(input_value)
    status_code = _optional_string(input_value.get("status_code"), "status_code") or DEFAULT_STATUS
    logger._add_metrics_from_log(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        log=cast("StandardLoggingPayload", kwargs["standard_logging_object"]),
        kwargs=kwargs,
        status_code=status_code,
    )


async def _run(input_value: Fields) -> Json:
    operation = _string(input_value.get("operation"), "operation")
    environment = _string_map(input_value.get("environment"), "environment")
    batch_size = _optional_int(input_value.get("batch_size"), "batch_size")
    clock = _float(input_value.get("clock", DEFAULT_CLOCK), "clock")
    warnings = _Warnings()
    health: Json = None
    report: Json = None
    tags: Json = None
    # the case environment is the process environment for the whole run, as an operator would set it
    with (
        patch.dict(os.environ, environment, clear=True),
        patch.object(verbose_logger, "warning", warnings),
        patch.object(time, "time", lambda: clock),
        patch.object(datadog_metrics, "datetime", _Now(_moment(clock))),
    ):
        logger = _build_logger(batch_size, environment)
        queue = cast("list[dict[str, Json]]", logger.log_queue)
        client = _Client(_statuses(input_value.get("replies")))
        logger.async_client = cast(AsyncHTTPHandler, client)
        match operation:
            case "init":
                pass
            case "add_metrics":
                _seed(input_value, logger)
            case "success_event":
                await logger.async_log_success_event(  # pyright: ignore[reportUnknownMemberType]
                    _log_kwargs(input_value), None, None, None
                )
            case "failure_event":
                await logger.async_log_failure_event(  # pyright: ignore[reportUnknownMemberType]
                    _log_kwargs(input_value), None, None, None
                )
            case "flush":
                _seed(input_value, logger)
                before = len(queue)
                await logger.flush_queue()
                after = len(queue)
                report = {"batch": before, "delivered": before - after, "requeued": after}
            case "health_check":
                health = _plain(await logger.async_health_check())
            case "extract_tags":
                status_code = _optional_string(input_value.get("status_code"), "status_code")
                extract = logger._extract_tags  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                tags = cast(Json, extract(_payload(input_value), status_code=status_code))
            case _:
                raise InputError(f"unknown datadog metrics operation {operation}")
        return {
            "series": _to_json(queue),
            "requests": _to_json(client.calls),
            "warnings": len([message for message in warnings.messages if "DD_API_KEY" in message]),
            "upload_url": logger.upload_url,
            "authenticated": bool(logger.dd_api_key),
            "batch_size": logger.batch_size,
            "flush_interval": int(logger.flush_interval),
            "health": health,
            "report": report,
            "tags": tags,
        }


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(_run(_object(input_value, "input")))
