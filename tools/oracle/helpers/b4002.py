import asyncio
import json
import logging
import os
from collections.abc import Mapping
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm.integrations.azure_sentinel.azure_sentinel as sentinel
from litellm._logging import verbose_logger  # pyright: ignore[reportPrivateUsage]
from litellm.integrations.azure_sentinel.azure_sentinel import AzureSentinelLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, MaskedHTTPStatusError
from litellm.types.utils import StandardAuditLogPayload, StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
START = 1_000_000.0
TOKEN_PATH = "/oauth2/v2.0/token"  # noqa: S105 - a URL path, not a credential
EVENTS = (
    ("standard_logging_object not found in kwargs", "missing_payload"),
    ("exceeded max_queue_size", "queue_overflow"),
    ("Payload too large for a single record", "record_too_large"),
)


class _Clock:
    def __init__(self) -> None:
        self.now = START

    def time(self) -> float:
        return self.now


class _Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.counts = {name: 0 for _, name in EVENTS}

    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        for fragment, name in EVENTS:
            if fragment in message:
                self.counts[name] += 1


def _object(value: Json) -> Object:
    return cast(Object, value) if isinstance(value, dict) else {}


def _items(value: Json) -> list[Json]:
    return cast("list[Json]", value) if isinstance(value, list) else []


def _strings(value: Json) -> list[str]:
    return [cast(str, item) for item in _items(value)]


def _model_payload(identifier: str, *, failure: bool, pad: int) -> StandardLoggingPayload:
    status = "failure" if failure else "success"
    payload: Object = {
        "id": identifier,
        "trace_id": "trace-1",
        "session_id": "session-1",
        "call_type": "completion",
        "response_cost": 0.5,
        "status": status,
        "status_fields": {"llm_api_status": status, "guardrail_status": "not_run"},
        "total_tokens": 5,
        "prompt_tokens": 2,
        "completion_tokens": 3,
        "startTime": 1704067200.0,
        "endTime": 1704067201.5,
        "completionStartTime": 1704067200.5,
        "response_time": 1.0,
        "model_map_information": {},
        "model": "gpt-4o",
        "api_base": "https://api.openai.com",
        "metadata": {},
        "saved_cache_cost": 0.0,
        "request_tags": [],
        "model_parameters": {"pad": "x" * pad} if pad else {},
        "hidden_params": {},
    }
    return cast(StandardLoggingPayload, payload)


def _audit_payload(identifier: str) -> StandardAuditLogPayload:
    payload: Object = {
        "id": identifier,
        "updated_at": "2024-01-01T00:00:00Z",
        "changed_by": "admin",
        "changed_by_api_key": "hashed-key",
        "action": "created",
        "table_name": "LiteLLM_VerificationToken",
        "object_id": "object-1",
        "before_value": None,
        "updated_values": "{}",
    }
    return cast(StandardAuditLogPayload, payload)


def _entry(record: Object) -> Object:
    if "table_name" in record:
        return record
    return {"id": record["id"], "status": record["status"]}


class _Client:
    def __init__(self, replies: list[Object]) -> None:
        self.replies = replies
        self.calls: list[Json] = []
        self.logger: AzureSentinelLogger | None = None

    def _arrive(self, arrive: Object) -> None:
        logger = self.logger
        if logger is None:
            return
        for identifier in _strings(arrive.get("model")):
            logger.log_queue.append(_model_payload(identifier, failure=False, pad=0))
        for identifier in _strings(arrive.get("audit")):
            logger.audit_log_queue.append(_audit_payload(identifier))

    def _record(self, url: str, data: object, headers: Mapping[str, str]) -> None:
        lowered = {name.lower(): value for name, value in headers.items()}
        call: Object = {
            "authorization": lowered.get("authorization"),
            "content_type": lowered.get("content-type"),
        }
        if url.endswith(TOKEN_PATH):
            call["kind"] = "token"
            call["url"] = url
            call["form"] = cast(Json, data)
        else:
            call["kind"] = "batch"
            call["url"] = url
            records = _items(cast(Json, json.loads(cast(bytes, data))))
            call["entries"] = [_entry(_object(record)) for record in records]
        self.calls.append(call)

    async def post(self, url: str, data: object, headers: Mapping[str, str]) -> httpx.Response:
        self._record(url, data, headers)
        reply = self.replies.pop(0)
        self._arrive(_object(reply.get("arrive")))
        if reply.get("cancel"):
            raise asyncio.CancelledError
        if reply.get("network"):
            raise RuntimeError("the scripted network failed")
        status = cast(int, reply["status"])
        text = cast(str, reply.get("body", ""))
        response = httpx.Response(status, text=text, request=httpx.Request("POST", url))
        if status >= 400:  # noqa: PLR2004 - the real client raises on every error status
            try:
                response.raise_for_status()
            except httpx.HTTPStatusError as error:
                raise MaskedHTTPStatusError(error, message=text, text=text) from None
        return response


async def _perform(action: Object, logger: AzureSentinelLogger, clock: _Clock) -> Json:
    step = action.get("do")
    match step:
        case "model":
            failure = bool(action.get("failure"))
            if action.get("missing"):
                if failure:
                    await logger.async_log_failure_event({}, None, None, None)  # pyright: ignore[reportUnknownMemberType]
                else:
                    await logger.async_log_success_event({}, None, None, None)  # pyright: ignore[reportUnknownMemberType]
                return None
            pad = cast(int, action.get("pad") or 0)
            for identifier in _strings(action.get("ids")):
                payload = _model_payload(identifier, failure=failure, pad=pad)
                kwargs = {"standard_logging_object": payload}
                if failure:
                    await logger.async_log_failure_event(kwargs, None, None, None)  # pyright: ignore[reportUnknownMemberType]
                else:
                    await logger.async_log_success_event(kwargs, None, None, None)  # pyright: ignore[reportUnknownMemberType]
            return None
        case "audit":
            for identifier in _strings(action.get("ids")):
                await logger.async_log_audit_log_event(_audit_payload(identifier))
            return None
        case "flush":
            await logger.flush_queue()
            return {"last_flush_time": int(logger.last_flush_time)}
        case "advance":
            clock.now += cast(float, action["seconds"])
            return None
        case _:
            message = f"unknown action: {step}"
            raise ValueError(message)


async def _run_scenario(scenario: Object) -> Json:
    clock = _Clock()
    capture = _Capture()
    client = _Client([_object(reply) for reply in _items(scenario.get("replies"))])
    environment = cast("dict[str, str]", scenario.get("environment") or {})
    saved_level = verbose_logger.level
    saved_handlers = verbose_logger.handlers[:]
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    try:
        with patch.dict(os.environ, environment), patch.object(sentinel, "time", clock):
            for name in [name for name in os.environ if name.startswith("AZURE_")]:
                if name not in environment:
                    del os.environ[name]
            try:
                init = cast("dict[str, str | None]", scenario.get("init") or {})
                logger = AzureSentinelLogger(**init)
            except ValueError as error:
                return {"initialized": False, "error": str(error)}
            logger.async_httpx_client = cast(AsyncHTTPHandler, client)
            client.logger = logger
            settings: Object = {
                "model_url": logger.api_endpoint,
                "audit_url": logger.audit_api_endpoint,
                "token_url": f"{logger.authority_host}/{logger.tenant_id}{TOKEN_PATH}",
                "scope": logger.oauth_scope,
                "client_id": logger.client_id,
                "client_secret": logger.client_secret,
                "batch_size": logger.batch_size,
                "flush_interval": logger.flush_interval,
                "max_queue_size": logger.max_queue_size,
            }
            logger.flush_interval = 3600
            batch_size = scenario.get("batch_size")
            if isinstance(batch_size, int):
                logger.batch_size = batch_size
            max_queue_size = scenario.get("max_queue_size")
            if isinstance(max_queue_size, int):
                logger.max_queue_size = max_queue_size
            results: list[Json] = []
            for action in _items(scenario.get("actions")):
                try:
                    results.append(await _perform(_object(action), logger, clock))
                except asyncio.CancelledError:
                    results.append({"cancelled": True})
            return {
                "initialized": True,
                "settings": settings,
                "actions": results,
                "model_queue": [record["id"] for record in logger.log_queue],
                "audit_queue": [record["id"] for record in logger.audit_log_queue],
                "awaiting": {
                    "model": logger.logs_awaiting_retry,
                    "audit": logger.audit_logs_awaiting_retry,
                },
                "calls": client.calls,
                "logs": cast(Json, capture.counts),
            }
    finally:
        verbose_logger.handlers = saved_handlers
        verbose_logger.setLevel(saved_level)


def run(input_value: dict[str, Json]) -> Json:
    scenarios = _items(input_value.get("scenarios"))
    results = [asyncio.run(_run_scenario(_object(scenario))) for scenario in scenarios]
    return {"results": cast(Json, results)}
