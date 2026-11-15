import asyncio
import os
import sys
from types import ModuleType
from typing import Final, Literal, TypedDict

import litellm
from litellm import CALLBACK_TYPES
from litellm.integrations.custom_logger import CustomLogger
from litellm.proxy._types import (
    AUDIT_ACTIONS,
    LiteLLM_AuditLogs,
    LitellmTableNames,
    UserAPIKeyAuth,
)
from litellm.proxy.management_helpers.audit_logs import (
    create_audit_log_for_update,
    create_object_audit_log,
    get_audit_log_changed_by,
    is_audit_logging_enabled,
    reset_audit_log_callback_cache,
)
from litellm.types.utils import StandardAuditLogPayload
from pydantic import JsonValue, TypeAdapter

AUDIT_LOGS_ENV: Final = "LITELLM_STORE_AUDIT_LOGS"
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final = TypeAdapter(JsonValue)
ACTION_VALUES: Final[dict[str, AUDIT_ACTIONS]] = {
    "created": "created",
    "updated": "updated",
    "deleted": "deleted",
    "blocked": "blocked",
    "unblocked": "unblocked",
    "rotated": "rotated",
    "kill_switch_fired": "kill_switch_fired",
}


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


CallbackCall = InstanceCallback | NamedCallback


class DispatchCall(TypedDict):
    aspect: Literal["dispatch"]
    record: AuditRecord
    callbacks: list[CallbackCall]


class Caller(TypedDict):
    user_id: str | None
    api_key: str | None


class ObjectCall(TypedDict):
    object_id: str
    action: str
    table_name: str
    before_value: str | None
    after_value: str | None


class SinkCall(TypedDict):
    aspect: Literal["sink_disabled"]
    record: AuditRecord
    object: ObjectCall
    caller: Caller


Call = PolicyCall | IdentityCall | RecordCall | DispatchCall | SinkCall


class RecordView(TypedDict):
    id: str
    updated_at: str
    changed_by: str | None
    changed_by_api_key: str | None
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


class PayloadView(TypedDict):
    id: str
    updated_at: str
    changed_by: str
    changed_by_api_key: str
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


RECORD_VIEW: Final = TypeAdapter(RecordView)
PAYLOAD_VIEW: Final = TypeAdapter(PayloadView)


class ProxyServer(ModuleType):
    premium_user: bool = False
    prisma_client: None = None


class Recorder(CustomLogger):
    def __init__(self, label: str, *, fails: bool) -> None:
        super().__init__()
        self.label = label
        self.fails = fails
        self.delivered: list[PayloadView] = []

    async def async_log_audit_log_event(self, audit_log: StandardAuditLogPayload) -> None:
        if self.fails:
            raise RuntimeError("the audit log callback failed")
        self.delivered.append(PAYLOAD_VIEW.validate_python(audit_log))


def install_proxy(premium: bool) -> ModuleType | None:
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


def policy_answer(call: PolicyCall) -> JsonValue:
    install_proxy(premium=True)
    litellm.store_audit_logs = call["store"]
    if call["env"] is None:
        os.environ.pop(AUDIT_LOGS_ENV, None)
    else:
        os.environ[AUDIT_LOGS_ENV] = call["env"]
    answer: dict[str, JsonValue] = {"enabled": is_audit_logging_enabled(call["explicit"])}
    return answer


def identity_answer(call: IdentityCall) -> JsonValue:
    values: dict[str, JsonValue] = {}
    if call["user_id"] is not None:
        values["user_id"] = call["user_id"]
    if call["metadata"] is not None:
        values["metadata"] = call["metadata"]
    if call["team_metadata"] is not None:
        values["team_metadata"] = call["team_metadata"]
    changed_by = get_audit_log_changed_by(
        litellm_changed_by=call["litellm_changed_by"],
        user_api_key_dict=UserAPIKeyAuth.model_validate(values),
        litellm_proxy_admin_name=call["admin_name"],
    )
    answer: dict[str, JsonValue] = {"changed_by": changed_by}
    return answer


def audit_model(record: AuditRecord) -> LiteLLM_AuditLogs:
    values: dict[str, JsonValue] = {
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
    return LiteLLM_AuditLogs.model_validate(values)


def record_answer(call: RecordCall) -> JsonValue:
    model = audit_model(call["record"])
    view = RECORD_VIEW.validate_json(model.model_dump_json())
    answer: dict[str, JsonValue] = {
        "id": view["id"],
        "updated_at": view["updated_at"],
        "changed_by": view["changed_by"],
        "changed_by_api_key": view["changed_by_api_key"],
        "action": view["action"],
        "table_name": view["table_name"],
        "object_id": view["object_id"],
        "before_value": parsed(view["before_value"]),
        "updated_values": parsed(view["updated_values"]),
    }
    return answer


def payload_value(payload: PayloadView) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "id": payload["id"],
        "updated_at": payload["updated_at"],
        "changed_by": payload["changed_by"],
        "changed_by_api_key": payload["changed_by_api_key"],
        "action": payload["action"],
        "table_name": payload["table_name"],
        "object_id": payload["object_id"],
        "before_value": parsed(payload["before_value"]),
        "updated_values": parsed(payload["updated_values"]),
    }
    return answer


def delivery_view(recorders: list[Recorder]) -> JsonValue:
    delivered: list[JsonValue] = []
    for recorder in recorders:
        for payload in recorder.delivered:
            entry: dict[str, JsonValue] = {
                "callback": recorder.label,
                "payload": payload_value(payload),
            }
            delivered.append(entry)
    return delivered


def dispatch_entries(call: DispatchCall) -> tuple[list[Recorder], list[CALLBACK_TYPES]]:
    recorders: list[Recorder] = []
    callbacks: list[CALLBACK_TYPES] = []
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
        await create_audit_log_for_update(request_data=model)
        await asyncio.sleep(0)
        await asyncio.sleep(0)
    finally:
        restore_proxy(saved)
    return delivery_view(recorders)


async def sink_answer(call: SinkCall) -> JsonValue:
    recorder = Recorder("sink", fails=False)
    callbacks: list[CALLBACK_TYPES] = [recorder]
    litellm.audit_log_callbacks = callbacks
    litellm.store_audit_logs = None
    os.environ.pop(AUDIT_LOGS_ENV, None)
    reset_audit_log_callback_cache()
    saved = install_proxy(premium=False)
    model = audit_model(call["record"])
    caller: dict[str, JsonValue] = {
        "user_id": call["caller"]["user_id"],
        "api_key": call["caller"]["api_key"],
    }
    try:
        await create_audit_log_for_update(request_data=model)
        await create_object_audit_log(
            object_id=call["object"]["object_id"],
            action=ACTION_VALUES[call["object"]["action"]],
            litellm_changed_by=None,
            user_api_key_dict=UserAPIKeyAuth.model_validate(caller),
            litellm_proxy_admin_name=None,
            table_name=LitellmTableNames(call["object"]["table_name"]),
            before_value=call["object"]["before_value"],
            after_value=call["object"]["after_value"],
        )
        await asyncio.sleep(0)
    finally:
        restore_proxy(saved)
    answer: dict[str, JsonValue] = {
        "delivered": delivery_view([recorder]),
        "stored": [],
        "calls": 2,
    }
    return answer


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
