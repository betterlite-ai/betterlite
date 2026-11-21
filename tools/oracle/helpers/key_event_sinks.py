import asyncio
import logging
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy import proxy_server
from litellm.proxy._types import LiteLLM_VerificationToken, WebhookEvent
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.hooks.key_management_event_hooks import KeyManagementEventHooks as Hooks
from litellm.proxy.utils import PrismaClient
from litellm.secret_managers.base_secret_manager import BaseSecretManager
from litellm.types.secret_managers.main import KeyManagementSettings
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

OWNED: Final = frozenset(
    (
        "_store_virtual_key_in_secret_manager",
        "_rotate_virtual_key_in_secret_manager",
        "_rename_virtual_key_in_secret_manager",
        "_delete_virtual_keys_from_secret_manager",
        "_get_secret_manager_optional_params",
        "_send_key_created_email",
        "_send_key_rotated_email",
    )
)
KEYS: Final[TypeAdapter[list[LiteLLM_VerificationToken]]] = TypeAdapter(
    list[LiteLLM_VerificationToken]
)
Params: TypeAlias = dict[str, JsonValue] | None


class Settings(TypedDict):
    store_virtual_keys: bool | None
    prefix_for_stored_virtual_keys: str
    description: str | None
    tags: dict[str, str] | None


class Store(TypedDict):
    action: Literal["store"]
    secret_name: str
    secret_token: str
    team_id: str | None


class Rotate(TypedDict):
    action: Literal["rotate"]
    current_secret_name: str
    new_secret_name: str
    new_secret_value: str
    team_id: str | None


class Rename(TypedDict):
    action: Literal["rename"]
    current_secret_name: str
    new_secret_name: str
    team_id: str | None


class Delete(TypedDict):
    action: Literal["delete"]
    keys: list[dict[str, JsonValue]]


class Created(TypedDict):
    action: Literal["key_created_email"]
    response: dict[str, JsonValue]


class Rotated(TypedDict):
    action: Literal["key_rotated_email"]


Step: TypeAlias = Store | Rotate | Rename | Delete | Created | Rotated


class Call(TypedDict):
    settings: Settings
    client: bool
    runtime: Literal["absent", "no_database", "ready"]
    teams: dict[str, JsonValue]
    secrets: dict[str, str]
    failing: list[str]
    alerting: list[str]
    steps: list[Step]


class StepAnswer(TypedDict):
    raised: bool
    effects: list[JsonValue]
    logs: list[str]


class Answer(TypedDict):
    steps: list[StepAnswer]


class Board(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.effects: list[JsonValue] = []
        self.logs: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName in OWNED:
            self.logs.append(record.levelname)

    async def email(self, webhook_event: WebhookEvent) -> bool:
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
        return True


class Manager(BaseSecretManager):
    def __init__(self, board: Board, call: Call) -> None:
        self.board = board
        self.secrets = call["secrets"]
        self.failing = call["failing"]

    def answer(
        self, effect: list[JsonValue], name: str, timeout: float | None
    ) -> dict[str, JsonValue]:
        del timeout
        self.board.effects.append(effect)
        if name in self.failing:
            raise RuntimeError(name)
        return {}

    @override
    async def async_read_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L30-L35
        self, secret_name: str, optional_params: Params = None, timeout: float | None = None
    ) -> str | None:
        return self.sync_read_secret(secret_name, optional_params, timeout)

    @override
    def sync_read_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L49-L54
        self, secret_name: str, optional_params: Params = None, timeout: float | None = None
    ) -> str | None:
        self.answer(["read", secret_name, optional_params], secret_name, timeout)
        return self.secrets.get(secret_name)

    @override
    async def async_write_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L68-L76
        self, secret_name: str, secret_value: str, description: str | None = None,
        optional_params: Params = None, timeout: float | None = None, tags: Params = None,
    ) -> dict[str, JsonValue]:
        effect: list[JsonValue] = ["write", secret_name, secret_value, description, tags]
        return self.answer([*effect, optional_params], secret_name, timeout)

    @override
    async def async_delete_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L94-L100
        self, secret_name: str, recovery_window_in_days: int | None = 7,
        optional_params: Params = None, timeout: float | None = None,
    ) -> dict[str, JsonValue]:
        effect: list[JsonValue] = ["delete", secret_name, recovery_window_in_days, optional_params]
        return self.answer(effect, secret_name, timeout)

    # one rotate effect keeps the base class's read, write and delete steps out of these cases
    @override
    async def async_rotate_secret(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L114-L120
        self, current_secret_name: str, new_secret_name: str, new_secret_value: str,
        optional_params: Params = None, timeout: float | None = None,
    ) -> dict[str, JsonValue]:
        effect: list[JsonValue] = ["rotate", current_secret_name, new_secret_name, new_secret_value]
        return self.answer([*effect, optional_params], current_secret_name, timeout)


# a team read from the cache never reaches the database, and a missing one fails on the stub below
class Teams(InMemoryCache):
    def __init__(self, board: Board, call: Call) -> None:
        super().__init__()
        self.board = board
        self.metadata = call["teams"]

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        team_id = key.removeprefix("team_id:")
        self.board.effects.append(["lookup", team_id])
        if team_id not in self.metadata:
            return None
        return {"team_id": team_id, "metadata": self.metadata[team_id]}


class Database(PrismaClient):
    def __init__(self) -> None:
        self.connected = True


def run(input_value: Call) -> Answer:
    board = Board()
    logger = logging.getLogger("LiteLLM Proxy")
    level = logger.level
    handlers = logger.handlers
    cache = proxy_server.user_api_key_cache
    alerting = proxy_server.proxy_logging_obj.slack_alerting_instance
    sender = alerting.send_key_created_or_user_invited_email
    logger.setLevel(logging.DEBUG)
    logger.handlers = [board]
    settings = input_value["settings"]
    litellm._key_management_settings = KeyManagementSettings(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1309
        store_virtual_keys=settings["store_virtual_keys"],
        prefix_for_stored_virtual_keys=settings["prefix_for_stored_virtual_keys"],
        description=settings["description"],
        tags=settings["tags"],
    )
    litellm.secret_manager_client = Manager(board, input_value) if input_value["client"] else None
    ready = input_value["runtime"] == "ready"
    proxy_server.prisma_client = Database() if ready else None
    proxy_server.user_api_key_cache = UserApiKeyCache(in_memory_cache=Teams(board, input_value))
    proxy_server.general_settings = {"alerting": input_value["alerting"]}
    alerting.send_key_created_or_user_invited_email = board.email  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/SlackAlerting/slack_alerting.py#L1274
    try:
        return asyncio.run(answer(input_value, board))
    finally:
        alerting.send_key_created_or_user_invited_email = sender  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/SlackAlerting/slack_alerting.py#L1274
        proxy_server.general_settings = {}
        proxy_server.user_api_key_cache = cache
        proxy_server.prisma_client = None
        litellm.secret_manager_client = None
        logger.handlers = handlers
        logger.setLevel(level)


async def answer(call: Call, board: Board) -> Answer:
    steps: list[StepAnswer] = []
    for step in call["steps"]:
        board.effects = []
        board.logs = []
        raised = False
        try:
            await perform(step)
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L92-L93
            raised = True
        steps.append({"raised": raised, "effects": board.effects, "logs": board.logs})
    return {"steps": steps}


async def perform(step: Step) -> None:
    if step["action"] == "store":
        await Hooks._store_virtual_key_in_secret_manager(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L289-L318
            step["secret_name"], step["secret_token"], step["team_id"]
        )
    elif step["action"] == "rotate":
        await Hooks._rotate_virtual_key_in_secret_manager(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L321-L345
            step["current_secret_name"],
            step["new_secret_name"],
            step["new_secret_value"],
            step["team_id"],
        )
    elif step["action"] == "rename":
        await Hooks._rename_virtual_key_in_secret_manager(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L359-L395
            step["current_secret_name"], step["new_secret_name"], step["team_id"]
        )
    elif step["action"] == "delete":
        keys = KEYS.validate_python(step["keys"])
        await Hooks._delete_virtual_keys_from_secret_manager(keys)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L405-L438
    elif step["action"] == "key_created_email":
        await Hooks._send_key_created_email(step["response"])  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L517-L589
        await asyncio.sleep(0)
    else:
        await Hooks._send_key_rotated_email({}, None)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/key_management_event_hooks.py#L592-L645
