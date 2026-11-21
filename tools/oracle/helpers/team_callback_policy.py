import asyncio
import logging
import os
import sys
from collections.abc import Callable
from datetime import datetime, tzinfo
from types import ModuleType
from typing import Final, Literal, TypedDict, cast

import litellm
from fastapi import HTTPException
from litellm.proxy._types import (
    AddTeamCallback,
    LiteLLM_AuditLogs,
    LitellmUserRoles,
    UserAPIKeyAuth,
)
from litellm.proxy.common_utils import callback_utils
from litellm.proxy.management_endpoints import team_callback_endpoints as endpoints
from litellm.proxy.management_helpers import audit_logs
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
AUDIT_LOGS_MODULE: Final = "litellm.proxy.management_helpers.audit_logs"
SALT_KEY_ENV: Final = "LITELLM_SALT_KEY"
ENCRYPT = cast("Callable[[JsonValue], JsonValue]", callback_utils.encrypt_callback_vars)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/callback_utils.py#L717
VALIDATE = endpoints._validate_team_callback  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L63-L66
REDACT = endpoints._redact_callback_secrets  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L69-L92
RESOLVE = endpoints._resolve_team_callbacks  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L121-L161
EMIT = endpoints._emit_team_callback_audit_log  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L179-L225
UNKNOWN_TEAM = endpoints._unknown_team_error  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L236-L249

UserRole = Literal[
    "proxy_admin", "proxy_admin_viewer", "org_admin", "internal_user",
    "internal_user_viewer", "team", "customer",
]


class ValidateStep(TypedDict):
    function: Literal["validate"]
    callback_name: str
    callback_vars: dict[str, str]


class RedactStep(TypedDict):
    function: Literal["redact"]
    metadata: JsonValue


class ResolveStep(TypedDict):
    function: Literal["resolve"]
    metadata: JsonValue
    encrypt: bool
    salt_key: str


class AuditStep(TypedDict):
    function: Literal["audit"]
    store_audit_logs: bool
    team_id: str
    before: JsonValue
    after: JsonValue
    user_id: str | None
    api_key: str | None
    litellm_changed_by: str | None
    admin_name: str | None
    outcome: Literal["written", "failed", "cancelled"]
    now: str


class UnknownTeamStep(TypedDict):
    function: Literal["unknown_team"]
    team_id: str
    user_role: UserRole | None
    status: int


Step = ValidateStep | RedactStep | ResolveStep | AuditStep | UnknownTeamStep


class Input(TypedDict):
    steps: list[Step]


class ErrorOut(TypedDict):
    status: int | None
    detail: dict[str, str] | str | None


class StepResult(TypedDict):
    value: JsonValue
    error: ErrorOut | None
    warnings: list[str]


class StoredRow(TypedDict):
    id: str
    updated_at: str
    changed_by: str | None
    changed_by_api_key: str | None
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


JSON: Final = TypeAdapter(JsonValue)
STORED: Final = TypeAdapter(StoredRow)


# the HTTPException fields read by attribute, since its detail is typed Any
class RaisedHttp(BaseModel):
    status_code: int
    detail: dict[str, str] | str


# the proxy_server the audit emission and LiteLLM's encryption helpers import
class ProxyServer(ModuleType):
    master_key: str | None = None
    premium_user: bool = False
    litellm_proxy_admin_name: str | None = None


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.fixed: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        head, _, _ = record.getMessage().partition(": ")
        self.fixed.append(head)


def install_module(name: str, module: ModuleType) -> ModuleType | None:
    saved = sys.modules.get(name)
    sys.modules[name] = module
    return saved


def restore_module(name: str, saved: ModuleType | None) -> None:
    if saved is None:
        del sys.modules[name]
        return
    sys.modules[name] = saved


class FixedClock:
    def __init__(self, moment: datetime) -> None:
        self.moment = moment

    def now(self, tz: tzinfo | None = None) -> datetime:
        if tz is None:
            return self.moment
        return self.moment.astimezone(tz)


class AuditRecorder:
    def __init__(self, outcome: Literal["written", "failed", "cancelled"]) -> None:
        self.outcome = outcome
        self.calls = 0
        self.rows: list[LiteLLM_AuditLogs] = []

    async def create_audit_log_for_update(self, request_data: LiteLLM_AuditLogs) -> None:
        self.calls += 1
        if self.outcome == "failed":
            raise RuntimeError("the audit write failed")
        if self.outcome == "cancelled":
            raise asyncio.CancelledError
        self.rows.append(request_data)


# the create_audit_log_for_update the case injects in place of the database
class AuditLogsModule(ModuleType):
    def __init__(self, writer: AuditRecorder) -> None:
        super().__init__(AUDIT_LOGS_MODULE)
        self.is_audit_logging_enabled = audit_logs.is_audit_logging_enabled
        self.create_audit_log_for_update = writer.create_audit_log_for_update


def http_error(raised: HTTPException) -> ErrorOut:
    view = RaisedHttp.model_validate(raised, from_attributes=True)
    return {"status": view.status_code, "detail": view.detail}


def validate_answer(step: ValidateStep) -> tuple[JsonValue, ErrorOut | None]:
    callback = AddTeamCallback(
        callback_name=step["callback_name"], callback_vars=step["callback_vars"],
    )
    try:
        VALIDATE(callback)
    except HTTPException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L59-L60
        return None, http_error(raised)
    return None, None


def redact_answer(step: RedactStep) -> tuple[JsonValue, ErrorOut | None]:
    metadata = step["metadata"]
    redacted = JSON.validate_python(REDACT(metadata))
    return {"redacted": redacted, "original": metadata}, None


def resolve_answer(step: ResolveStep) -> tuple[JsonValue, ErrorOut | None]:
    os.environ[SALT_KEY_ENV] = step["salt_key"]
    try:
        stored = ENCRYPT(step["metadata"]) if step["encrypt"] else step["metadata"]
        resolved = RESOLVE(stored)
    except ValidationError:
        return None, {"status": None, "detail": None}
    finally:
        del os.environ[SALT_KEY_ENV]
    return JSON.validate_python(resolved.model_dump_json()), None


def row_view(model: LiteLLM_AuditLogs) -> JsonValue:
    view = STORED.validate_json(model.model_dump_json())
    before, after = view["before_value"], view["updated_values"]
    return {
        "id": view["id"],
        "updated_at": datetime.fromisoformat(view["updated_at"]).isoformat(),
        "changed_by": view["changed_by"],
        "changed_by_api_key": view["changed_by_api_key"],
        "action": view["action"],
        "table_name": view["table_name"],
        "object_id": view["object_id"],
        "before_value": None if before is None else JSON.validate_json(before),
        "updated_values": None if after is None else JSON.validate_json(after),
    }


async def emit_and_settle(step: AuditStep) -> None:
    caller = UserAPIKeyAuth(user_id=step["user_id"], api_key=step["api_key"])
    await EMIT(
        team_id=step["team_id"], before_metadata=step["before"], after_metadata=step["after"],
        user_api_key_dict=caller, litellm_changed_by=step["litellm_changed_by"],
    )
    for _ in range(3):
        await asyncio.sleep(0)


def audit_answer(step: AuditStep, proxy: ProxyServer) -> tuple[JsonValue, None]:
    recorder = AuditRecorder(step["outcome"])
    saved_logs = install_module(AUDIT_LOGS_MODULE, AuditLogsModule(recorder))
    proxy.litellm_proxy_admin_name = step["admin_name"]
    litellm.store_audit_logs = step["store_audit_logs"]
    endpoints.datetime = FixedClock(datetime.fromisoformat(step["now"]))  # type: ignore[attr-defined,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L201
    try:
        asyncio.run(emit_and_settle(step))
    finally:
        endpoints.datetime = datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_callback_endpoints.py#L11
        restore_module(AUDIT_LOGS_MODULE, saved_logs)
        litellm.store_audit_logs = None
    record = row_view(recorder.rows[0]) if recorder.rows else None
    return {"scheduled": recorder.calls > 0, "record": record}, None


def unknown_team_answer(step: UnknownTeamStep) -> tuple[JsonValue, ErrorOut | None]:
    role = step["user_role"]
    caller = UserAPIKeyAuth(user_role=None if role is None else LitellmUserRoles(role))
    return None, http_error(UNKNOWN_TEAM(step["team_id"], caller, step["status"]))


def answer(step: Step, proxy: ProxyServer) -> tuple[JsonValue, ErrorOut | None]:
    match step["function"]:
        case "validate":
            return validate_answer(step)
        case "redact":
            return redact_answer(step)
        case "resolve":
            return resolve_answer(step)
        case "audit":
            return audit_answer(step, proxy)
        case "unknown_team":
            return unknown_team_answer(step)


def run(input_value: Input) -> list[StepResult]:
    logger = logging.getLogger("LiteLLM Proxy")
    proxy = ProxyServer(PROXY_MODULE)
    saved_proxy = install_module(PROXY_MODULE, proxy)
    results: list[StepResult] = []
    previous_level = logger.level
    logger.setLevel(logging.WARNING)
    try:
        for step in input_value["steps"]:
            recorded = Warnings()
            logger.addHandler(recorded)
            try:
                value, error = answer(step, proxy)
            finally:
                logger.removeHandler(recorded)
            results.append({"value": value, "error": error, "warnings": recorded.fixed})
    finally:
        logger.setLevel(previous_level)
        restore_module(PROXY_MODULE, saved_proxy)
    return results
