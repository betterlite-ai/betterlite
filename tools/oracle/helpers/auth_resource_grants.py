import asyncio
from typing import Literal, NotRequired, TypedDict

from litellm.models.managed_files import LiteLLM_ManagedVectorStoresTable
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    can_key_call_search_tool,
    can_team_call_search_tool,
    get_managed_vector_store_rows_by_uuids,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue
from typing_extensions import override


class PermissionFields(TypedDict):
    object_permission_id: str
    search_tools: NotRequired[list[str] | None]


class KeyFields(TypedDict):
    object_permission_id: NotRequired[str | None]
    permission: NotRequired[PermissionFields | None]


class TeamFields(TypedDict):
    team_id: str
    object_permission_id: NotRequired[str | None]
    permission: NotRequired[PermissionFields | None]


class SearchKeyCall(TypedDict):
    function: Literal["search_key"]
    search_tool_name: str
    key: KeyFields


class SearchTeamCall(TypedDict):
    function: Literal["search_team"]
    search_tool_name: str
    team: TeamFields | None


class RowFixture(TypedDict):
    id: str
    model_dump: dict[str, JsonValue]


class ManagedRowsCall(TypedDict):
    function: Literal["managed_rows"]
    rows: list[RowFixture]
    rounds: list[list[str]]


Input = SearchKeyCall | SearchTeamCall | ManagedRowsCall


class Denial(TypedDict):
    status: int
    error_type: str
    param: str | None
    code: str


class Answer(TypedDict):
    allowed: bool
    error: Denial | None


class Query(TypedDict):
    uuids: list[str]
    take: int


class Round(TypedDict):
    vector_store_ids: list[str]
    queries: list[Query]


class FakeVectorStoreTable:
    def __init__(
        self, rows: list[tuple[str, LiteLLM_ManagedVectorStoresTable]], queries: list[Query]
    ) -> None:
        self._rows = rows
        self._queries = queries

    async def find_many(
        self, *, where: dict[str, dict[str, list[str]]], take: int
    ) -> list[LiteLLM_ManagedVectorStoresTable]:
        uuids = where["vector_store_id"]["in"]
        self._queries.append({"uuids": list(uuids), "take": take})
        return [row for identifier, row in self._rows if identifier in uuids]


class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeVectorStoreTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> FakeVectorStoreTable:
        del name
        return self.table


class FakePrismaClient(PrismaClient):
    def __init__(self, table: FakeVectorStoreTable) -> None:
        self.db = FakeDb(table)


def run(input_value: Input) -> Answer | list[Round]:
    match input_value["function"]:
        case "search_key":
            return search_key(input_value)
        case "search_team":
            return search_team(input_value)
        case "managed_rows":
            return asyncio.run(resolve_rounds(input_value))


def search_key(call: SearchKeyCall) -> Answer:
    fields = call["key"]
    token = UserAPIKeyAuth(
        object_permission_id=fields.get("object_permission_id"),
        object_permission=permission_of(fields.get("permission")),
    )
    try:
        asyncio.run(can_key_call_search_tool(call["search_tool_name"], token))
    except ProxyException as denial:
        return {"allowed": False, "error": denial_of(denial)}
    return {"allowed": True, "error": None}


def search_team(call: SearchTeamCall) -> Answer:
    fields = call["team"]
    team: LiteLLM_TeamTable | None = None
    if fields is not None:
        team = LiteLLM_TeamTable(
            team_id=fields["team_id"],
            object_permission_id=fields.get("object_permission_id"),
            object_permission=permission_of(fields.get("permission")),
        )
    try:
        asyncio.run(can_team_call_search_tool(call["search_tool_name"], team))
    except ProxyException as denial:
        return {"allowed": False, "error": denial_of(denial)}
    return {"allowed": True, "error": None}


async def resolve_rounds(call: ManagedRowsCall) -> list[Round]:
    queries: list[Query] = []
    rows: list[tuple[str, LiteLLM_ManagedVectorStoresTable]] = []
    for fixture in call["rows"]:
        row = LiteLLM_ManagedVectorStoresTable.model_validate(fixture["model_dump"])
        rows.append((fixture["id"], row))
    client = FakePrismaClient(FakeVectorStoreTable(rows, queries))
    cache = UserApiKeyCache()
    rounds: list[Round] = []
    for uuids in call["rounds"]:
        queries.clear()
        found = await get_managed_vector_store_rows_by_uuids(
            uuids=uuids,
            prisma_client=client,
            user_api_key_cache=cache,
        )
        rounds.append(
            {
                "vector_store_ids": [row.vector_store_id for row in found],
                "queries": list(queries),
            }
        )
    return rounds


def permission_of(fields: PermissionFields | None) -> LiteLLM_ObjectPermissionTable | None:
    if fields is None:
        return None
    return LiteLLM_ObjectPermissionTable(
        object_permission_id=fields["object_permission_id"],
        search_tools=fields.get("search_tools"),
    )


def denial_of(denial: ProxyException) -> Denial:
    return {
        "status": int(denial.code),
        "error_type": denial.type,
        "param": denial.param,
        "code": denial.code,
    }
