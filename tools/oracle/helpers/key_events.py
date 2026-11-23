import asyncio
import logging
from collections.abc import Coroutine
from datetime import datetime, timezone
from typing import Final, Literal, NamedTuple, TypeAlias, TypedDict
from uuid import UUID

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy import proxy_server
from litellm.proxy._types import (
    GenerateKeyRequest,
    GenerateKeyResponse,
    KeyRequest,
    LiteLLM_AuditLogs,
    LiteLLM_VerificationToken,
    RegenerateKeyRequest,
    UpdateKeyRequest,
    UserAPIKeyAuth,
    WebhookEvent,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.hooks import key_management_event_hooks as hooks
from litellm.proxy.hooks.key_management_event_hooks import KeyManagementEventHooks as Hooks
from litellm.proxy.management_helpers import audit_logs
from litellm.proxy.utils import PrismaClient
from litellm.secret_managers.base_secret_manager import BaseSecretManager
from litellm.types.secret_managers.main import KeyManagementSettings
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

HOOKS_MODULE: Final = "key_management_event_hooks.py"
JSON: Final = TypeAdapter(JsonValue)
GENERATE_REQUEST: Final = TypeAdapter(GenerateKeyRequest)
GENERATE_RESPONSE: Final = TypeAdapter(GenerateKeyResponse)
UPDATE_REQUEST: Final = TypeAdapter(UpdateKeyRequest)
REGENERATE_REQUEST: Final = TypeAdapter(RegenerateKeyRequest)
KEY_REQUEST: Final = TypeAdapter(KeyRequest)
KEY: Final = TypeAdapter(LiteLLM_VerificationToken)
KEYS: Final = TypeAdapter(list[LiteLLM_VerificationToken])
Params: TypeAlias = dict[str, JsonValue] | None


class Caller(TypedDict):
    user_id: str | None
    api_key: str | None
    token: str | None
    metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue] | None


class Settings(TypedDict):
    store_virtual_keys: bool | None
    prefix_for_stored_virtual_keys: str
    description: str | None
    tags: dict[str, str] | None


class Call(TypedDict):
    event: Literal["generated", "updated", "rotated", "deleted"]
    data: JsonValue
    response: dict[str, JsonValue]
    row: JsonValue
    rows: list[JsonValue]
    caller: Caller
    changed_by: str | None
    admin_name: str
    audit: bool | None
    now: str
    id: str
    settings: Settings
    client: bool
    runtime: Literal["absent", "no_database", "ready"]
    teams: dict[str, JsonValue]
    secrets: dict[str, str]
    failing: list[str]
    alerting: list[str]


class AuditFields(TypedDict):
    id: str
    updated_at: str
    changed_by: JsonValue
    changed_by_api_key: str | None
    action: str
    table_name: str
    object_id: str
    before_value: str | None
    updated_values: str | None


AUDIT: Final = TypeAdapter(AuditFields)


class Board(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.effects: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno == logging.WARNING and record.pathname.endswith(HOOKS_MODULE):
            self.effects.append(["warn"])

    def email(self, webhook_event: WebhookEvent) -> Coroutine[None, None, bool]:
        view: dict[str, JsonValue] = {
            "event": webhook_event.event,
            "event_group": webhook_event.event_group.value,
            "event_message": webhook_event.event_message,
            "token": webhook_event.token,
            "spend": webhook_event.spend,
            "max_budget": webhook_event.max_budget,
            "user_id": webhook_event.user_id,
            "team_id": webhook_event.team_id,
            "key_alias": webhook_event.key_alias,
        }
        self.effects.append(["email", view])
        return asyncio.sleep(0, result=True)

    def audit(self, request_data: LiteLLM_AuditLogs) -> Coroutine[None, None, None]:
        self.effects.append(["audit", audit_view(request_data)])
        return asyncio.sleep(0)


class Manager(BaseSecretManager):
    def __init__(self, board: Board, call: Call) -> None:
        self.board = board
        self.secrets = call["secrets"]
        self.failing = call["failing"]

    def answer(self, effect: list[JsonValue], name: str) -> dict[str, JsonValue]:
        self.board.effects.append(effect)
        if name in self.failing:
            raise RuntimeError(name)
        return {}

    @override
    async def async_read_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L30-L35
        self, secret_name: str, optional_params: Params = None, _timeout: float | None = None
    ) -> str | None:
        self.answer(["read", secret_name, optional_params], secret_name)
        return self.secrets.get(secret_name)

    @override
    def sync_read_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L48-L54
        self, secret_name: str, optional_params: Params = None, _timeout: float | None = None
    ) -> str | None:
        self.answer(["read", secret_name, optional_params], secret_name)
        return self.secrets.get(secret_name)

    @override
    async def async_write_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L68-L76
        self, secret_name: str, secret_value: str, description: str | None = None,
        optional_params: Params = None, _timeout: float | None = None, tags: Params = None,
    ) -> dict[str, JsonValue]:
        effect: list[JsonValue] = ["write", secret_name, secret_value, description, tags]
        return self.answer([*effect, optional_params], secret_name)

    @override
    async def async_delete_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L94-L100
        self, secret_name: str, recovery_window_in_days: int | None = 7,
        optional_params: Params = None, _timeout: float | None = None,
    ) -> dict[str, JsonValue]:
        effect: list[JsonValue] = ["delete", secret_name, recovery_window_in_days, optional_params]
        return self.answer(effect, secret_name)

    @override
    async def async_rotate_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L114-L120
        self, current_secret_name: str, new_secret_name: str, new_secret_value: str,
        optional_params: Params = None, _timeout: float | None = None,
    ) -> dict[str, JsonValue]:
        effect: list[JsonValue] = ["rotate", current_secret_name, new_secret_name, new_secret_value]
        return self.answer([*effect, optional_params], current_secret_name)


class Teams(InMemoryCache):
    def __init__(self, board: Board, call: Call) -> None:
        super().__init__()
        self.board = board
        self.metadata = call["teams"]

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        team_id = key.removeprefix("team_id:")
        self.board.effects.append(["lookup", team_id])
        team = self.metadata.get(team_id)
        return None if team is None else {"team_id": team_id, "metadata": team}


class Database(PrismaClient):
    def __init__(self) -> None: ...


class Stubs(NamedTuple):
    moment: datetime
    identifier: UUID

    def now(self, tz: timezone | None = None) -> datetime:
        return self.moment if tz is None else self.moment.astimezone(tz)

    def uuid4(self) -> UUID: return self.identifier


def audit_view(record: LiteLLM_AuditLogs) -> JsonValue:
    fields = AUDIT.validate_json(record.model_dump_json())
    before = fields["before_value"]
    updated = fields["updated_values"]
    return {
        "id": fields["id"],
        "updated_at": fields["updated_at"],
        "changed_by": fields["changed_by"],
        "changed_by_api_key": fields["changed_by_api_key"],
        "action": fields["action"],
        "table_name": fields["table_name"],
        "object_id": fields["object_id"],
        "before_value": JSON.validate_json(before if before is not None else "null"),
        "updated_values": JSON.validate_json(updated if updated is not None else "null"),
    }


def caller(identity: Caller) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        user_id=identity["user_id"],
        api_key=identity["api_key"],
        token=identity["token"],
        metadata=identity["metadata"],
        team_metadata=identity["team_metadata"],
    )


def run(input_value: Call) -> dict[str, JsonValue]:
    board = Board()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.setLevel(logging.DEBUG)
    logger.handlers = [board]
    settings = input_value["settings"]
    litellm._key_management_settings = KeyManagementSettings(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1309
        store_virtual_keys=settings["store_virtual_keys"],
        prefix_for_stored_virtual_keys=settings["prefix_for_stored_virtual_keys"],
        description=settings["description"],
        tags=settings["tags"],
    )
    litellm.store_audit_logs = input_value["audit"]
    litellm.secret_manager_client = Manager(board, input_value) if input_value["client"] else None
    proxy_server.prisma_client = Database() if input_value["runtime"] == "ready" else None
    proxy_server.user_api_key_cache = UserApiKeyCache(in_memory_cache=Teams(board, input_value))
    proxy_server.general_settings = {"alerting": input_value["alerting"]}
    proxy_server.litellm_proxy_admin_name = input_value["admin_name"]
    stubs = Stubs(datetime.fromisoformat(input_value["now"]), UUID(input_value["id"]))
    hooks.datetime = stubs  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage, reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L71
    hooks.uuid = stubs  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage, reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L69
    audit_logs.create_audit_log_for_update = board.audit
    alerting = proxy_server.proxy_logging_obj.slack_alerting_instance
    alerting.send_key_created_or_user_invited_email = board.email  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/SlackAlerting/slack_alerting.py#L1274
    return asyncio.run(answer(input_value, board))


async def answer(call: Call, board: Board) -> dict[str, JsonValue]:
    raised = False
    try:
        await perform(call)
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L92-L93
        raised = True
    return {"raised": raised, "trace": board.effects}


async def perform(call: Call) -> None:
    match call["event"]:
        case "generated":
            await Hooks.async_key_generated_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L36-L94
                data=GENERATE_REQUEST.validate_python(call["data"]),
                response=GENERATE_RESPONSE.validate_python(call["response"]),
                user_api_key_dict=caller(call["caller"]),
                litellm_changed_by=call["changed_by"],
            )
        case "updated":
            await Hooks.async_key_updated_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L97-L154
                data=UPDATE_REQUEST.validate_python(call["data"]),
                existing_key_row=KEY.validate_python(call["row"]),
                response=call["response"],
                user_api_key_dict=caller(call["caller"]),
                litellm_changed_by=call["changed_by"],
            )
        case "rotated":
            raw = call["data"]
            await Hooks.async_key_rotated_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L157-L224
                data=REGENERATE_REQUEST.validate_python(raw) if raw is not None else None,
                existing_key_row=KEY.validate_python(call["row"]),
                response=GENERATE_RESPONSE.validate_python(call["response"]),
                user_api_key_dict=caller(call["caller"]),
                litellm_changed_by=call["changed_by"],
            )
        case "deleted":
            await Hooks.async_key_deleted_hook(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L227-L245
                data=KEY_REQUEST.validate_python(call["data"]),
                keys_being_deleted=KEYS.validate_python(call["rows"]),
                response=call["response"],
                user_api_key_dict=caller(call["caller"]),
                litellm_changed_by=call["changed_by"],
            )
