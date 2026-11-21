import asyncio
from datetime import datetime
from typing import Final, Literal, TypeAlias, TypedDict, final

import litellm
from litellm.integrations.additional_logging_utils import AdditionalLoggingUtils
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.logging_callback_manager import LoggingCallbackManager
from litellm.proxy import proxy_server
from litellm.proxy._types import LiteLLM_UserTable, LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.spend_tracking.spend_management_endpoints import ui_view_request_response_for_request_id
from litellm.proxy.utils import PrismaClient
from litellm.types.integrations.base_health_check import IntegrationHealthCheckStatus
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

DETAIL: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])
UserRole = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]
Bind: TypeAlias = str | tuple[str, ...] | None
TeamIds = TypedDict("TeamIds", {"in": list[str]})


class CallerInput(TypedDict):
    user_id: str | None
    user_role: UserRole | None


class MemberInput(TypedDict):
    user_id: str | None
    role: Literal["admin", "user"]


class TeamInput(TypedDict):
    team_id: str
    members_with_roles: list[MemberInput]
    team_member_permissions: list[str] | None


class UserInput(TypedDict):
    user_id: str
    teams: list[str]


class OwnerRow(TypedDict):
    user: str | None
    team_id: str | None


class SpendRow(TypedDict):
    request_id: str
    user: str | None
    team_id: str | None


class Step(TypedDict):
    caller: CallerInput
    lookup_id: str
    users: list[UserInput]
    teams: list[TeamInput]
    fail_teams: bool
    owners: list[OwnerRow]
    rows: list[SpendRow]
    payload: dict[str, JsonValue] | None


class Input(TypedDict):
    steps: list[Step]


class Read(TypedDict):
    read: Literal["query", "team", "teams"]
    sql: str | None
    binds: list[JsonValue]


class StepAnswer(TypedDict):
    reads: list[Read]
    storage_key: str | None
    outcome: Literal["payload", "row", "none", "http", "error"]
    status: int | None
    detail: dict[str, str] | None


@final
class UniqueTeam(TypedDict):
    team_id: str


@final
class UniqueUser(TypedDict):
    user_id: str


class ManyTeams(TypedDict):
    team_id: TeamIds


class ScriptedFailureError(RuntimeError):
    pass


class TeamRow(BaseModel):
    model_config = ConfigDict(extra="allow")


class Board:
    def __init__(self, step: Step) -> None:
        self.step = step
        self.reads: list[Read] = []
        self.storage_key: str | None = None


# the team and user tables; a user is only ever served from the cache the step fills
class Table:
    def __init__(self, board: Board) -> None:
        self.board = board

    async def find_unique(
        self, *, where: UniqueTeam | UniqueUser, include: dict[str, bool] | None = None
    ) -> TeamRow | None:
        del include
        if "user_id" in where:
            return None
        found = await self.find_many(where={"team_id": {"in": [where["team_id"]]}}, read="team")
        return found.pop() if found else None

    async def find_many(self, *, where: ManyTeams, read: Literal["team", "teams"] = "teams") -> list[TeamRow]:
        team_ids = where["team_id"]["in"]
        binds: list[JsonValue] = [*team_ids] if read == "team" else [[*team_ids]]
        self.board.reads.append({"read": read, "sql": None, "binds": binds})
        if self.board.step["fail_teams"]:
            raise ScriptedFailureError(read)
        teams = self.board.step["teams"]
        return [TeamRow.model_validate(team) for team in teams if team["team_id"] in team_ids]


class Db(PrismaWrapper):
    def __init__(self, board: Board) -> None:
        self.board = board
        self.litellm_teamtable = Table(board)
        self.litellm_usertable = Table(board)

    async def query_raw(self, sql: str, *args: Bind) -> list[OwnerRow] | list[SpendRow]:
        binds: list[JsonValue] = []
        for arg in args:
            match arg:
                case tuple():
                    binds.append([*arg])
                case str() | None:
                    binds.append(arg)
        self.board.reads.append({"read": "query", "sql": " ".join(sql.split()), "binds": binds})
        if "SELECT DISTINCT" in sql:
            return self.board.step["owners"]
        return self.board.step["rows"]


class Client(PrismaClient):
    def __init__(self, board: Board) -> None:
        self.db = Db(board)


# the cold-storage logger the detail route asks, recording the key it is asked for
class Recorder(CustomLogger, AdditionalLoggingUtils):
    def __init__(self, board: Board) -> None:
        super().__init__()
        self.board = board

    @override
    async def async_health_check(self) -> IntegrationHealthCheckStatus:
        return {"status": "healthy", "error_message": None}

    @override
    async def get_request_response_payload(
        self, request_id: str, start_time_utc: datetime | None, end_time_utc: datetime | None
    ) -> dict[str, JsonValue] | None:
        del start_time_utc, end_time_utc
        self.board.storage_key = request_id
        return self.board.step["payload"]


# the callback manager the detail route asks for its cold-storage loggers
class Loggers(LoggingCallbackManager):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    @override
    def get_active_additional_logging_utils_from_custom_logger(self) -> set[AdditionalLoggingUtils]:
        return {self.recorder}


def run(input_value: Input) -> list[StepAnswer]:
    client = proxy_server.prisma_client
    cache = proxy_server.user_api_key_cache
    loggers = litellm.logging_callback_manager
    try:
        return [asyncio.run(run_step(step)) for step in input_value["steps"]]
    finally:
        proxy_server.prisma_client = client
        proxy_server.user_api_key_cache = cache
        litellm.logging_callback_manager = loggers


async def run_step(step: Step) -> StepAnswer:
    board = Board(step)
    cache = UserApiKeyCache(default_in_memory_ttl=60.0)
    for user in step["users"]:
        stored = LiteLLM_UserTable(user_id=user["user_id"], teams=user["teams"])
        await cache.async_set_cache(key=user["user_id"], value=stored)
    proxy_server.prisma_client = Client(board)
    proxy_server.user_api_key_cache = cache
    litellm.logging_callback_manager = Loggers(Recorder(board))
    return await view(step, board)


async def view(step: Step, board: Board) -> StepAnswer:
    role = step["caller"]["user_role"]
    caller = UserAPIKeyAuth(
        user_id=step["caller"]["user_id"],
        user_role=None if role is None else LitellmUserRoles(role),
    )
    answer: StepAnswer = {
        "reads": board.reads,
        "storage_key": None,
        "outcome": "none",
        "status": None,
        "detail": None,
    }
    try:
        found: JsonValue | None = await ui_view_request_response_for_request_id(
            request_id=step["lookup_id"], start_date=None, end_date=None, user_api_key_dict=caller
        )
    except HTTPException as raised:
        answer["outcome"] = "http"
        answer["status"] = raised.status_code
        answer["detail"] = DETAIL.validate_python(raised.detail)
    except ScriptedFailureError:
        answer["outcome"] = "error"
    else:
        served = board.storage_key is not None and step["payload"] is not None
        answer["outcome"] = "payload" if served else "none" if found is None else "row"
    answer["storage_key"] = board.storage_key
    return answer
