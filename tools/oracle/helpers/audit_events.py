import asyncio
import os
import sys
from collections.abc import Callable
from datetime import datetime
from types import ModuleType
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm.integrations.custom_logger import CustomLogger
from litellm.proxy._types import (
    AUDIT_ACTIONS,
    LiteLLM_AuditLogs,
    LitellmTableNames,
    UserAPIKeyAuth,
)
from litellm.proxy.management_helpers import audit_logs
from litellm.proxy.management_helpers.audit_logs import (
    get_audit_log_changed_by,
    is_audit_logging_enabled,
    reset_audit_log_callback_cache,
)
from litellm.types.utils import StandardAuditLogPayload
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

AUDIT_LOGS_ENV: Final = "LITELLM_STORE_AUDIT_LOGS"
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final = TypeAdapter(JsonValue)
# LiteLLM's CALLBACK_TYPES holds a bare Callable of Any, so this names the same union
CallbackEntry = CustomLogger | str | Callable[[], None]
ACTION: Final = TypeAdapter(AUDIT_ACTIONS)


class PolicyCall(TypedDict):
    aspect: Literal["policy"]
    explicit: bool | None
    store: bool | None
    env: str | None


class IdentityCall(TypedDict):
    aspect: Literal["identity"]
    litellm_changed_by: str | None
    user_id: str | None
    metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None
    admin_name: str | None


class AuditRecord(TypedDict):
    id: str
    updated_at: str
    changed_by: JsonValue
    changed_by_api_key: str | None
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


class RecordCall(TypedDict):
    aspect: Literal["record"]
    record: AuditRecord


class InstanceCallback(TypedDict):
    kind: Literal["instance"]
    label: str
    fails: bool


class NamedCallback(TypedDict):
    kind: Literal["named"]
    name: str


class DispatchCall(TypedDict):
    aspect: Literal["dispatch"]
    record: AuditRecord
    callbacks: list[InstanceCallback | NamedCallback]


class SinkCall(TypedDict):
    aspect: Literal["sink_disabled"]
    record: AuditRecord
    object_id: str
    action: str
    table_name: str
    before_value: str | None
    after_value: str | None
    user_id: str | None
    api_key: str | None


Call = PolicyCall | IdentityCall | RecordCall | DispatchCall | SinkCall


class Stored(TypedDict):
    id: str
    updated_at: str
    changed_by: str | None
    changed_by_api_key: str | None
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


STORED: Final = TypeAdapter(Stored)


class ProxyServer(ModuleType):
    premium_user: bool = False
    prisma_client: None = None


class AuditLogWrites(Protocol):
    async def create_audit_log_for_update(self, request_data: LiteLLM_AuditLogs) -> None: ...

    async def create_object_audit_log(
        self,
        *args: *tuple[
            str, AUDIT_ACTIONS, str | None, UserAPIKeyAuth,
            str | None, LitellmTableNames, str | None, str | None,
        ],
    ) -> None: ...


AUDIT_LOG_WRITES: AuditLogWrites = audit_logs
CALLER: TypeAdapter[UserAPIKeyAuth] = TypeAdapter(UserAPIKeyAuth)


class Recorder(CustomLogger):
    def __init__(self, label: str, *, fails: bool) -> None:
        self.label = label
        self.fails = fails
        self.delivered: list[Stored] = []

    @override
    async def async_log_audit_log_event(self, audit_log: StandardAuditLogPayload) -> None:
        if self.fails:
            raise RuntimeError("the audit log callback failed")
        self.delivered.append(STORED.validate_python(audit_log))


def install_proxy(*, premium: bool) -> ModuleType | None:
    module = ProxyServer(PROXY_MODULE)
    module.premium_user = premium
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = module
    return saved


def restore_proxy(saved: ModuleType | None) -> None:
    if saved is None:
        del sys.modules[PROXY_MODULE]
        return
    sys.modules[PROXY_MODULE] = saved


def parsed(text: str | None) -> JsonValue:
    if text is None:
        return None
    return JSON.validate_json(text)


def audit_model(record: AuditRecord) -> LiteLLM_AuditLogs:
    return LiteLLM_AuditLogs(
        id=record["id"], updated_at=datetime.fromisoformat(record["updated_at"]),
        changed_by=record["changed_by"], changed_by_api_key=record["changed_by_api_key"],
        action=ACTION.validate_python(record["action"]), object_id=record["object_id"],
        table_name=LitellmTableNames(record["table_name"]),
        before_value=record["before_value"],
        updated_values=record["updated_values"],
    )


def policy_answer(call: PolicyCall) -> JsonValue:
    install_proxy(premium=True)
    litellm.store_audit_logs = call["store"]
    if call["env"] is None:
        os.environ.pop(AUDIT_LOGS_ENV, None)
    else:
        os.environ[AUDIT_LOGS_ENV] = call["env"]
    return {"enabled": is_audit_logging_enabled(call["explicit"])}


def identity_answer(call: IdentityCall) -> JsonValue:
    values: dict[str, JsonValue] = {"user_id": call["user_id"]}
    if call["metadata"] is not None:
        values["metadata"] = call["metadata"]
    if call["team_metadata"] is not None:
        values["team_metadata"] = call["team_metadata"]
    caller = CALLER.validate_python(values)
    changed_by = get_audit_log_changed_by(
        litellm_changed_by=call["litellm_changed_by"],
        user_api_key_dict=caller,
        litellm_proxy_admin_name=call["admin_name"],
    )
    return {"changed_by": changed_by}


def stored_json(view: Stored) -> JsonValue:
    return {
        "id": view["id"], "updated_at": view["updated_at"],
        "changed_by": view["changed_by"], "changed_by_api_key": view["changed_by_api_key"],
        "action": view["action"], "table_name": view["table_name"],
        "object_id": view["object_id"], "before_value": parsed(view["before_value"]),
        "updated_values": parsed(view["updated_values"]),
    }


def record_answer(call: RecordCall) -> JsonValue:
    model = audit_model(call["record"])
    return stored_json(STORED.validate_json(model.model_dump_json()))


def delivery_view(recorders: list[Recorder]) -> JsonValue:
    return [
        {"callback": recorder.label, "payload": stored_json(payload)}
        for recorder in recorders
        for payload in recorder.delivered
    ]


def dispatch_entries(call: DispatchCall) -> tuple[list[Recorder], list[CallbackEntry]]:
    recorders: list[Recorder] = []
    callbacks: list[CallbackEntry] = []
    for entry in call["callbacks"]:
        if entry["kind"] == "instance":
            recorder = Recorder(entry["label"], fails=entry["fails"])
            recorders.append(recorder)
            callbacks.append(recorder)
        else:
            callbacks.append(entry["name"])
    return recorders, callbacks


async def dispatch_answer(call: DispatchCall) -> JsonValue:
    recorders, callbacks = dispatch_entries(call)
    litellm.audit_log_callbacks = callbacks
    reset_audit_log_callback_cache()
    saved = install_proxy(premium=True)
    model = audit_model(call["record"])
    try:
        await AUDIT_LOG_WRITES.create_audit_log_for_update(request_data=model)
        await asyncio.sleep(0)
        await asyncio.sleep(0)
    finally:
        restore_proxy(saved)
    return {"delivered": delivery_view(recorders)}


async def sink_answer(call: SinkCall) -> JsonValue:
    recorder = Recorder("sink", fails=False)
    callbacks: list[CallbackEntry] = [recorder]
    litellm.audit_log_callbacks = callbacks
    litellm.store_audit_logs = None
    os.environ.pop(AUDIT_LOGS_ENV, None)
    reset_audit_log_callback_cache()
    saved = install_proxy(premium=False)
    model = audit_model(call["record"])
    caller = UserAPIKeyAuth(user_id=call["user_id"], api_key=call["api_key"])
    try:
        await AUDIT_LOG_WRITES.create_audit_log_for_update(request_data=model)
        await AUDIT_LOG_WRITES.create_object_audit_log(
            call["object_id"],
            ACTION.validate_python(call["action"]),
            None,
            caller,
            None,
            LitellmTableNames(call["table_name"]),
            call["before_value"],
            call["after_value"],
        )
        await asyncio.sleep(0)
    finally:
        restore_proxy(saved)
    return {"delivered": delivery_view([recorder]), "stored": [], "calls": 2}


def run(input_value: Call) -> JsonValue:
    match input_value["aspect"]:
        case "policy":
            return policy_answer(input_value)
        case "identity":
            return identity_answer(input_value)
        case "record":
            return record_answer(input_value)
        case "dispatch":
            return asyncio.run(dispatch_answer(input_value))
        case "sink_disabled":
            return asyncio.run(sink_answer(input_value))
