import asyncio
import logging
import os
import time
from collections.abc import Mapping
from dataclasses import dataclass, field
from logging.handlers import BufferingHandler
from typing import Literal, TypeAlias, TypedDict, final

import httpx
from litellm._logging import verbose_logger
from litellm.integrations.azure_sentinel.azure_sentinel import AzureSentinelLogger
from litellm.llms.custom_httpx.http_handler import MaskedHTTPStatusError
from litellm.types.utils import StandardAuditLogPayload
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
EVENTS = (
    ("standard_logging_object not found in kwargs", "missing_payload"),
    ("exceeded max_queue_size", "queue_overflow"),
    ("Payload too large for a single record", "record_too_large"),
)


@dataclass(frozen=True)
class Arrivals:
    model: tuple[str, ...] = ()
    audit: tuple[str, ...] = ()


@dataclass(frozen=True)
class Reply:
    status: int = 0
    body: str = ""
    network: bool = False
    cancel: bool = False
    arrive: Arrivals = Arrivals()


@dataclass(frozen=True)
class Action:
    do: Literal["model", "audit", "flush", "advance"]
    ids: tuple[str, ...] = ()
    failure: bool = False
    missing: bool = False
    pad: int | None = None
    seconds: float = 0.0


@dataclass(frozen=True)
class Scenario:
    environment: dict[str, str] = field(default_factory=dict)
    init: dict[str, str] = field(default_factory=dict)
    batch_size: int | None = None
    max_queue_size: int | None = None
    replies: list[Reply] = field(default_factory=list)
    actions: tuple[Action, ...] = ()


@final
class SentModel(TypedDict):
    id: str
    status: str


@final
class SentAudit(TypedDict):
    id: str
    updated_at: str
    changed_by: str
    changed_by_api_key: str
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


SentRecord: TypeAlias = SentAudit | SentModel
SENT_BATCH = TypeAdapter(list[SentRecord])
SCENARIOS = TypeAdapter(list[Scenario])


class _Clock:
    def __init__(self) -> None:
        self.now = 1_000_000.0

    def __call__(self) -> float:
        return self.now


def _model_payload(identifier: str, *, failure: bool, pad: int | None) -> Object:
    status = "failure" if failure else "success"
    parameters: Object = {}
    if pad is not None:
        parameters["pad"] = "x" * pad
    return {
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
        "model_parameters": parameters,
        "hidden_params": {},
    }


def _audit_payload(identifier: str) -> StandardAuditLogPayload:
    return {
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


def _entry(record: SentRecord) -> Object:
    if "table_name" in record:
        return {
            "id": record["id"],
            "updated_at": record["updated_at"],
            "changed_by": record["changed_by"],
            "changed_by_api_key": record["changed_by_api_key"],
            "action": record["action"],
            "table_name": record["table_name"],
            "object_id": record["object_id"],
            "before_value": record["before_value"],
            "updated_values": record["updated_values"],
        }
    return {"id": record["id"], "status": record["status"]}


class _Client:
    def __init__(self, logger: AzureSentinelLogger, replies: list[Reply]) -> None:
        self.logger = logger
        self.replies = replies
        self.calls: list[Json] = []

    async def post(
        self, url: str, data: dict[str, str] | bytes, headers: Mapping[str, str]
    ) -> httpx.Response:
        lowered = {name.lower(): value for name, value in headers.items()}
        call: Object = {
            "authorization": lowered.get("authorization"),
            "content_type": lowered.get("content-type"),
            "url": url,
        }
        match data:
            case dict():
                call["kind"] = "token"
                form: Object = dict(data)
                call["form"] = form
            case bytes():
                call["kind"] = "batch"
                call["entries"] = [_entry(record) for record in SENT_BATCH.validate_json(data)]
        self.calls.append(call)
        reply = self.replies.pop(0)
        for name in reply.arrive.model:
            payload = _model_payload(name, failure=False, pad=None)
            self.logger.log_queue.append(payload)  # type: ignore[arg-type,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
        for name in reply.arrive.audit:
            self.logger.audit_log_queue.append(_audit_payload(name))
        if reply.cancel:
            raise asyncio.CancelledError
        if reply.network:
            raise RuntimeError("the scripted network failed")
        response = httpx.Response(reply.status, text=reply.body, request=httpx.Request("POST", url))
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as error:
            raise MaskedHTTPStatusError(error, message=reply.body, text=reply.body) from None
        return response


async def _perform(action: Action, logger: AzureSentinelLogger, clock: _Clock) -> Json:
    match action.do:
        case "model":
            calls: list[Object] = [{}] if action.missing else []
            for name in action.ids:
                payload = _model_payload(name, failure=action.failure, pad=action.pad)
                calls.append({"standard_logging_object": payload})
            for kwargs in calls:
                if action.failure:
                    await logger.async_log_failure_event(kwargs, None, None, None)  # type: ignore[no-untyped-call,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/azure_sentinel/azure_sentinel.py#L267
                else:
                    await logger.async_log_success_event(kwargs, None, None, None)  # type: ignore[no-untyped-call,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/azure_sentinel/azure_sentinel.py#L240
            return None
        case "audit":
            for identifier in action.ids:
                await logger.async_log_audit_log_event(_audit_payload(identifier))
            return None
        case "flush":
            await logger.flush_queue()  # type: ignore[no-untyped-call,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/azure_sentinel/azure_sentinel.py#L444
            return {"last_flush_time": int(logger.last_flush_time)}
        case "advance":
            clock.now += action.seconds
            return None


async def _run_logger(scenario: Scenario, clock: _Clock, capture: BufferingHandler) -> Json:
    try:
        logger = AzureSentinelLogger(**scenario.init)
    except ValueError as error:
        return {"initialized": False, "error": str(error)}
    client = _Client(logger, scenario.replies)
    logger.async_httpx_client = client  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L799
    settings: Object = {
        "model_url": logger.api_endpoint,
        "audit_url": logger.audit_api_endpoint,
        "token_url": f"{logger.authority_host}/{logger.tenant_id}/oauth2/v2.0/token",
        "scope": logger.oauth_scope,
        "client_id": logger.client_id,
        "client_secret": logger.client_secret,
        "batch_size": logger.batch_size,
        "flush_interval": logger.flush_interval,
        "max_queue_size": logger.max_queue_size,
    }
    logger.flush_interval = 3600
    if scenario.batch_size is not None:
        logger.batch_size = scenario.batch_size
    if scenario.max_queue_size is not None:
        logger.max_queue_size = scenario.max_queue_size
    results: list[Json] = []
    for action in scenario.actions:
        try:
            results.append(await _perform(action, logger, clock))
        except asyncio.CancelledError:
            results.append({"cancelled": True})
    messages = [record.getMessage() for record in capture.buffer]
    logs: Object = {name: sum(part in text for text in messages) for part, name in EVENTS}
    model_queue: list[Json] = [record["id"] for record in logger.log_queue]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
    return {
        "initialized": True,
        "settings": settings,
        "actions": results,
        "model_queue": model_queue,
        "audit_queue": [record["id"] for record in logger.audit_log_queue],
        "awaiting": {
            "model": logger.logs_awaiting_retry,
            "audit": logger.audit_logs_awaiting_retry,
        },
        "calls": client.calls,
        "logs": logs,
    }


async def _run_scenario(scenario: Scenario) -> Json:
    clock = _Clock()
    capture = BufferingHandler(capacity=100_000)
    saved_environment = dict(os.environ)
    saved_time = time.time
    saved_level = verbose_logger.level
    saved_handlers = verbose_logger.handlers[:]
    for name in [name for name in os.environ if name.startswith("AZURE_")]:
        del os.environ[name]
    os.environ.update(scenario.environment)
    time.time = clock
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    try:
        return await _run_logger(scenario, clock, capture)
    finally:
        time.time = saved_time
        os.environ.clear()
        os.environ.update(saved_environment)
        verbose_logger.handlers = saved_handlers
        verbose_logger.setLevel(saved_level)


def run(input_value: Object) -> Json:
    scenarios = SCENARIOS.validate_python(input_value["scenarios"])
    results: list[Json] = [asyncio.run(_run_scenario(scenario)) for scenario in scenarios]
    return {"results": results}
