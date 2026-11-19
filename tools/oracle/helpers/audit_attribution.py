import asyncio
import json
import sys
from collections.abc import Callable
from datetime import datetime
from types import ModuleType
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.integrations.custom_logger import CustomLogger
from litellm.proxy._types import (
    AUDIT_ACTIONS,
    LiteLLM_AuditLogs,
    LitellmTableNames,
    UserAPIKeyAuth,
)
from litellm.proxy.management_helpers import audit_logs
from litellm.proxy.management_helpers.audit_logs import get_audit_log_changed_by
from litellm.types.utils import StandardAuditLogPayload
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
ACTION: Final = TypeAdapter(AUDIT_ACTIONS)
CALLER: TypeAdapter[UserAPIKeyAuth] = TypeAdapter(UserAPIKeyAuth)
JSON: Final = TypeAdapter(JsonValue)
# LiteLLM's CALLBACK_TYPES holds a bare Callable of Any, so this names the same union
CallbackEntry = CustomLogger | str | Callable[[], None]


class IdentityInput(TypedDict):
    function: Literal["identity"]
    litellm_changed_by: str | None
    user_id: str | None
    metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None
    admin_name: str | None


class PayloadInput(TypedDict):
    function: Literal["payload"]
    id: str
    updated_at: str
    changed_by: str | None
    changed_by_api_key: str | None
    action: str
    table_name: str
    object_id: str
    before_value: JsonValue
    updated_values: JsonValue


Input: TypeAlias = IdentityInput | PayloadInput


class ProxyServer(ModuleType):
    premium_user: bool = False
    prisma_client: None = None


class AuditLogWrites(Protocol):
    async def create_audit_log_for_update(self, request_data: LiteLLM_AuditLogs) -> None: ...


AUDIT_LOG_WRITES: AuditLogWrites = audit_logs


class Recorder(CustomLogger):
    payload: StandardAuditLogPayload | None = None

    @override
    async def async_log_audit_log_event(self, audit_log: StandardAuditLogPayload) -> None:
        self.payload = audit_log


def install_proxy() -> ModuleType | None:
    module = ProxyServer(PROXY_MODULE)
    module.premium_user = True
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


def json_text(value: JsonValue) -> str | None:
    if value is None:
        return None
    return json.dumps(value)


def audit_model(call: PayloadInput) -> LiteLLM_AuditLogs:
    return LiteLLM_AuditLogs(
        id=call["id"],
        updated_at=datetime.fromisoformat(call["updated_at"]),
        changed_by=call["changed_by"],
        changed_by_api_key=call["changed_by_api_key"],
        action=ACTION.validate_python(call["action"]),
        object_id=call["object_id"],
        table_name=LitellmTableNames(call["table_name"]),
        before_value=json_text(call["before_value"]),
        updated_values=json_text(call["updated_values"]),
    )


def identity_answer(call: IdentityInput) -> JsonValue:
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


async def dispatch(model: LiteLLM_AuditLogs) -> None:
    await AUDIT_LOG_WRITES.create_audit_log_for_update(request_data=model)
    await asyncio.sleep(0)
    await asyncio.sleep(0)


def payload_answer(call: PayloadInput) -> JsonValue:
    recorder = Recorder()
    callbacks: list[CallbackEntry] = [recorder]
    litellm.audit_log_callbacks = callbacks
    litellm.store_audit_logs = True
    saved = install_proxy()
    model = audit_model(call)
    try:
        asyncio.run(dispatch(model))
    finally:
        restore_proxy(saved)
        litellm.audit_log_callbacks = []
    delivered = recorder.payload
    if delivered is None:
        raise ValueError("the audit callback received no payload")
    return {
        "id": delivered["id"],
        "updated_at": delivered["updated_at"],
        "changed_by": delivered["changed_by"],
        "changed_by_api_key": delivered["changed_by_api_key"],
        "action": delivered["action"],
        "table_name": delivered["table_name"],
        "object_id": delivered["object_id"],
        "before_value": parsed(delivered["before_value"]),
        "updated_values": parsed(delivered["updated_values"]),
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "identity":
        return identity_answer(input_value)
    return payload_answer(input_value)
