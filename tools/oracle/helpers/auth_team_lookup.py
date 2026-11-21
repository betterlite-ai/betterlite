import asyncio
import logging
import time
from collections.abc import Mapping
from typing import Final, Literal, NoReturn, TypedDict

from fastapi import Request
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.constants import DEFAULT_IN_MEMORY_TTL
from litellm.models.access_group import LiteLLM_AccessGroupTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy._types import NewTeamRequest, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.proxy.auth.auth_checks import (
    TeamNotFoundError,
    _update_last_db_access_time,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2487-L2488
    get_access_object,
    get_object_permission,
    get_team_object,
    get_team_object_by_alias,
    last_db_access_time,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints import team_endpoints
from litellm.proxy.management_endpoints.team_endpoints import new_team
from litellm.proxy.utils import PrismaClient
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DETAIL: Final[TypeAdapter[str | dict[str, str]]] = TypeAdapter(str | dict[str, str])
LOGGING_FUNCTIONS: Final = frozenset(
    ("_get_team_object_from_user_api_key_cache", "get_access_object", "get_team_object_by_alias")
)


class Step(TypedDict):
    now: int
    fail: list[str]


class Call(TypedDict):
    row: str
    function: Literal["team", "alias", "access"]
    target: str
    flags: list[Literal["no_database", "cache_only", "db_only", "upsert"]]
    in_memory_ttl: int
    throttled_at: int | None
    cached_teams: dict[str, dict[str, JsonValue]]
    cached_groups: dict[str, dict[str, JsonValue]]
    teams: list[dict[str, JsonValue]]
    groups: list[dict[str, JsonValue]]
    permissions: list[dict[str, JsonValue]]
    created: dict[str, JsonValue] | None
    steps: list[Step]


class TeamView(TypedDict):
    team_id: str
    team_alias: str | None
    permission: bool
    model_table: bool
    refreshed: bool


class GroupView(TypedDict):
    access_group_id: str
    access_group_name: str


class Failure(TypedDict):
    kind: Literal["team_not_found", "http", "exception"]
    status: int | None
    detail: Literal["object", "text"] | None


class StepAnswer(TypedDict):
    value: TeamView | GroupView | None
    error: Failure | None
    events: list[str]
    logs: list[str]


class Answer(TypedDict):
    row: str
    steps: list[StepAnswer]


# what one step records, the log handler that records it and the clock LiteLLM reads through time.time
class Board(logging.Handler):
    def __init__(self, call: Call) -> None:
        super().__init__(logging.DEBUG)
        self.events: list[str] = []
        self.logs: list[str] = []
        self.fail: list[str] = []
        self.now = 0.0
        self.values: dict[str, JsonValue] = {**call["cached_teams"], **call["cached_groups"]}
        self.created = call["created"]

    def __call__(self) -> float:
        return self.now

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName in LOGGING_FUNCTIONS:
            self.logs.append(record.levelname)


class Memory(InMemoryCache):
    def __init__(self, board: Board) -> None:
        super().__init__()
        self.board = board

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        self.board.events.append(f"read {key}")
        return self.board.values.get(key)


# DualCache swallows a failing backend write, so a scripted write failure is raised above it
class KeyCache(UserApiKeyCache):
    def __init__(self, board: Board) -> None:
        super().__init__(in_memory_cache=Memory(board), default_in_memory_ttl=60.0)
        self.board = board

    @override
    async def async_set_cache(self, key: str, value: BaseModel, *, model_type: type[BaseModel], ttl: float) -> None:  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/user_api_key_cache.py#L215-L233
        del model_type
        if f"write {key}" in self.board.fail:
            raise RuntimeError(key)
        self.board.events.append(f"write {key} ttl={ttl:g}")
        self.board.values[key] = JSON_VALUE.validate_json(value.model_dump_json())


class Row(BaseModel):
    model_config = ConfigDict(extra="allow")


class Table:
    def __init__(self, board: Board, name: str, handle: str, rows: list[dict[str, JsonValue]]) -> None:
        self.board = board
        self.name = name
        self.handle = handle
        self.rows = rows

    async def find_unique(self, *, where: dict[str, str], include: Mapping[str, bool] | None = None) -> Row | None:
        found = await self.find_many(where=where, include=include)
        return found.pop() if found else None

    async def find_many(self, *, where: dict[str, str], include: Mapping[str, bool] | None = None) -> list[Row]:
        del include
        field, value = next(iter(where.items()))
        self.board.events.append(f"db {self.name} {self.handle} {value}")
        if self.name in self.board.fail:
            raise RuntimeError(self.name)
        return [Row.model_validate(row) for row in self.rows if row.get(field) == value]


class Db(PrismaWrapper):
    def __init__(self, board: Board, call: Call, handle: str) -> None:
        self.tables = {
            "litellm_teamtable": Table(board, "litellm_teamtable", handle, call["teams"]),
            "litellm_accessgrouptable": Table(board, "litellm_accessgrouptable", handle, call["groups"]),
            "litellm_objectpermissiontable": Table(board, "litellm_objectpermissiontable", handle, call["permissions"]),
        }

    @override
    def __getattr__(self, name: str) -> Table:
        try:
            return self.tables[name]
        except KeyError as error:
            raise AttributeError(name) from error


class Client(PrismaClient):
    def __init__(self, board: Board, call: Call) -> None:
        self.db = Db(board, call, "reader")
        self.writer = Db(board, call, "writer")

    @property
    @override
    def writer_db(self) -> Db:
        return self.writer


# stands in for team_endpoints.new_team, which _get_team_db_check imports when it runs
class Provisioner:
    def __init__(self, board: Board) -> None:
        self.board = board

    async def __call__(
        self, data: NewTeamRequest, http_request: Request, user_api_key_dict: UserAPIKeyAuth
    ) -> dict[str, JsonValue]:
        del http_request
        role = user_api_key_dict.user_role
        self.board.events.append(f"provision {data.team_id} {'' if role is None else role.value}")
        if "provision" in self.board.fail or self.board.created is None:
            raise RuntimeError("provision")
        return self.board.created


# get_object_permission catches its own read failures, so a failed load is scripted in its place
class Refusal:
    async def __call__(self, **_kwargs: str | bool | PrismaClient | UserApiKeyCache | None) -> NoReturn:
        raise RuntimeError("permission")


def run(input_value: Call) -> Answer:
    if input_value["in_memory_ttl"] != DEFAULT_IN_MEMORY_TTL:
        raise ValueError("in_memory_ttl must equal the DEFAULT_IN_MEMORY_TTL the case env sets")
    board = Board(input_value)
    logger = logging.getLogger("LiteLLM Proxy")
    level = logger.level
    handlers = logger.handlers
    clock = time.time
    last_db_access_time.clear()
    time.time = board
    logger.setLevel(logging.DEBUG)
    logger.handlers = [board]
    team_endpoints.new_team = Provisioner(board)  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3152-L3163
    try:
        return asyncio.run(answer(input_value, board))
    finally:
        team_endpoints.new_team = new_team  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L1330-L1337
        auth_checks.get_object_permission = get_object_permission  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3981
        logger.handlers = handlers
        logger.setLevel(level)
        time.time = clock
        last_db_access_time.clear()


async def answer(call: Call, board: Board) -> Answer:
    client = None if "no_database" in call["flags"] else Client(board, call)
    cache = KeyCache(board)
    if call["throttled_at"] is not None:
        board.now = float(call["throttled_at"])
        _update_last_db_access_time(f"team_id:{call['target']}", None, last_db_access_time)
    steps: list[StepAnswer] = []
    for step in call["steps"]:
        board.now = float(step["now"])
        board.fail = list(step["fail"])
        board.events = []
        board.logs = []
        auth_checks.get_object_permission = Refusal() if "permission" in board.fail else get_object_permission  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3981
        value: TeamView | GroupView | None = None
        error: Failure | None = None
        try:
            value = await lookup(call, client, cache)
        except TeamNotFoundError as raised:
            error = failure("team_not_found", raised)
        except HTTPException as raised:
            error = failure("http", raised)
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3275-L3276
            error = {"kind": "exception", "status": None, "detail": None}
        steps.append({"value": value, "error": error, "events": list(board.events), "logs": list(board.logs)})
    return {"row": call["row"], "steps": steps}


async def lookup(call: Call, client: Client | None, cache: KeyCache) -> TeamView | GroupView:
    target = call["target"]
    flags = call["flags"]
    cache_only = "cache_only" in flags
    db_only = "db_only" in flags
    match call["function"]:
        case "team":
            upsert = "upsert" in flags
            team = await get_team_object(
                target, client, cache, check_cache_only=cache_only, check_db_only=db_only, team_id_upsert=upsert
            )
            return team_view(team)
        case "alias":
            named: LiteLLM_TeamTableCachedObj = await get_team_object_by_alias(target, client, cache)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3419-L3420
            return team_view(named)
        case "access":
            group: LiteLLM_AccessGroupTable = await get_access_object(target, client, cache, check_db_only=db_only)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3348-L3349
            return {"access_group_id": group.access_group_id, "access_group_name": group.access_group_name}


def team_view(team: LiteLLM_TeamTableCachedObj) -> TeamView:
    return {
        "team_id": team.team_id,
        "team_alias": team.team_alias,
        "permission": team.object_permission is not None,
        "model_table": team.litellm_model_table is not None,
        "refreshed": team.last_refreshed_at is not None,
    }


# the exception class, status and detail shape; the message text is not compared
def failure(kind: Literal["team_not_found", "http"], error: HTTPException) -> Failure:
    match DETAIL.validate_python(error.detail):
        case str():
            return {"kind": kind, "status": error.status_code, "detail": "text"}
        case dict():
            return {"kind": kind, "status": error.status_code, "detail": "object"}
