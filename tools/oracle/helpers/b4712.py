import asyncio
import sys
from collections import UserDict
from collections.abc import Callable, Coroutine, Mapping
from copy import deepcopy
from datetime import UTC, datetime
from types import ModuleType, TracebackType
from typing import Final, Literal, Self, TypedDict

from litellm.models.mcp_server import LiteLLM_MCPServerTable
from litellm.proxy._experimental.mcp_server import db as mcp_db
from litellm.proxy._types import NewMCPServerRequest, UpdateMCPServerRequest
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp import MCPCredentials
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

Where = dict[str, JsonValue | datetime]
PROXY_SERVER: Final = "litellm.proxy.proxy_server"
JSON_COLUMNS: Final = ("credentials", "env_vars", "mcp_info")
JSON_MAP_COLUMNS: Final = ("tool_name_to_display_name", "tool_name_to_description")
STRING: Final[TypeAdapter[str]] = TypeAdapter(str)
JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
WINDOW: Final[TypeAdapter[Where]] = TypeAdapter(Where)
STAMP: Final[TypeAdapter[datetime]] = TypeAdapter(datetime)
BRANCHES: Final[TypeAdapter[list[Where]]] = TypeAdapter(list[Where])


class ProxyServerStub(ModuleType):
    def __init__(self) -> None:
        super().__init__(PROXY_SERVER)
        self.redis_usage_cache: None = None
        self.master_key: None = None


class ConflictView(TypedDict):
    field: Literal["server_name", "alias"]
    value: str
    server_id: str


class EnvVarView(TypedDict):
    name: str
    value: str
    scope: str
    description: str | None


class ServerView(TypedDict):
    server_id: str
    server_name: str | None
    alias: str | None
    description: str | None
    auth_type: str | None
    approval_status: str | None
    created_by: str | None
    updated_by: str | None
    is_byok: bool
    credentials: MCPCredentials | None
    env_vars: list[EnvVarView] | None
    issuer: str | None
    token_url: str | None
    oauth2_flow: str | None
    token_exchange_endpoint: str | None
    audience: str | None


SERVER_VIEW: Final[TypeAdapter[ServerView]] = TypeAdapter(ServerView)


class Answer(TypedDict):
    server_id: str | None
    conflict: ConflictView | None
    rows: list[ServerView]
    invalidated: list[str]
    deleted_children: list[str]


Outcome = tuple[str | None, ConflictView | None]
Function = Literal["create", "create_if_free", "create_draft", "get_draft", "update", "delete"]


class Call(TypedDict):
    function: Function
    salt_key: str
    touched_by: str
    payload: Where
    rows: list[Where]
    server_id: str
    ttl_seconds: int
    users: list[str]


class FakeRow(UserDict[str, JsonValue | datetime]):
    def __getattr__(self, name: str) -> JsonValue | datetime:
        try:
            return self[name]
        except KeyError as error:
            raise AttributeError(name) from error

    def model_dump(self) -> Where:
        return dict(self.data)


class FakeTable(PrismaWrapper):
    def __init__(self, label: str, deleted: list[str]) -> None:
        self.label = label
        self.deleted = deleted
        self.rows: list[FakeRow] = []

    @override
    def __getattr__(self, name: str) -> Self:
        del name
        return self

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self, kind: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None:
        pass

    async def execute_raw(self, _query: str, *_keys: int) -> int:
        return 0

    async def create(self, data: Where) -> FakeRow:
        stored = FakeRow(parsed_columns(data))
        stored["updated_at"] = datetime.now(UTC)
        self.rows.append(stored)
        return FakeRow(deepcopy(stored.data))

    async def update(self, where: Where, data: Where) -> FakeRow | None:
        found = await self.find_first(where)
        if found is not None:
            found.update(parsed_columns(data))
        return None if found is None else FakeRow(deepcopy(found.data))

    async def delete(self, where: Where) -> FakeRow | None:
        found = await self.find_first(where)
        self.rows = [row for row in self.rows if row is not found]
        return None if found is None else FakeRow(deepcopy(found.data))

    async def delete_many(self, where: Where) -> int:
        self.deleted.append(self.label)
        removed = await self.find_many(where=where)
        self.rows = [row for row in self.rows if all(row is not gone for gone in removed)]
        return len(removed)

    async def find_many(self, where: Where | None = None, take: int | None = None) -> list[FakeRow]:
        clauses: list[tuple[str, JsonValue | datetime]] = [] if where is None else [*where.items()]
        matches = [row for row in self.rows if all(field_matches(row, *part) for part in clauses)]
        return matches if take is None else matches[:take]

    async def find_first(self, where: Where) -> FakeRow | None:
        return next(iter(await self.find_many(where=where, take=1)), None)

    find_unique = find_first


class FakeDb(PrismaWrapper):
    def __init__(self) -> None:
        self.deleted: list[str] = []
        self.litellm_mcpservertable = FakeTable("server", self.deleted)
        self.litellm_mcpusercredentials = FakeTable("credential", self.deleted)
        self.litellm_mcpuserenvvars = FakeTable("env var", self.deleted)
        self.litellm_mcpserveroauthclient = FakeTable("OAuth client", self.deleted)

    def tx(self) -> FakeTable:
        return self.litellm_mcpservertable


class FakeClient(PrismaClient):
    def __init__(self, rows: list[Where], users: list[str], server_id: str) -> None:
        self.db = self.fake_db = FakeDb()
        self.invalidated: list[tuple[str, str]] = []
        seeded = [FakeRow(row) for row in rows]
        for stored in seeded:
            stamp = stored.get("updated_at")
            if stamp is not None:
                stored["updated_at"] = STAMP.validate_python(stamp)
        self.fake_db.litellm_mcpservertable.rows = seeded
        self.fake_db.litellm_mcpusercredentials.rows = [
            FakeRow({"user_id": user, "server_id": server_id}) for user in users
        ]

    async def record_invalidation(self, user_id: str, server_id: str) -> None:
        self.invalidated.append((user_id, server_id))


def parsed_columns(data: Where) -> Where:
    parsed = dict(data)
    for column in (*JSON_COLUMNS, *JSON_MAP_COLUMNS):
        value = parsed.get(column)
        if value is not None:
            parsed[column] = JSON_VALUE.validate_json(STRING.validate_python(value))
    return parsed


# The where shapes LiteLLM's MCP server writes send: AND/OR/NOT, equality, gte, not, insensitive.
def field_matches(values: FakeRow, key: str, condition: JsonValue | datetime) -> bool:
    if key in {"AND", "OR", "NOT"}:
        branches = [list(branch.items()) for branch in BRANCHES.validate_python(condition)]
        nested = [all(field_matches(values, *part) for part in branch) for branch in branches]
        return {"AND": all(nested), "OR": any(nested), "NOT": not any(nested)}[key]
    value = values.get(key)
    try:
        operators = WINDOW.validate_python(condition)
    except ValidationError:
        return value == condition
    if "gte" in operators:
        return STAMP.validate_python(value) >= STAMP.validate_python(operators["gte"])
    if "not" in operators:
        return value is not None and value != operators["not"]
    wanted = STRING.validate_python(operators["equals"]).lower()
    return value is not None and STRING.validate_python(value).lower() == wanted


def written(outcome: LiteLLM_MCPServerTable | mcp_db.McpIdentifierConflict | None) -> Outcome:
    if outcome is None:
        return None, None
    match outcome:
        case mcp_db.McpIdentifierConflict(field=field, value=value, server_id=server_id):
            return None, {"field": field, "value": value, "server_id": server_id}
        case LiteLLM_MCPServerTable(server_id=server_id):
            return server_id, None


def row_views(client: FakeClient) -> list[ServerView]:
    views: list[ServerView] = []
    for row in client.fake_db.litellm_mcpservertable.rows:
        server = LiteLLM_MCPServerTable.model_validate(row.model_dump())
        mcp_db.decrypt_global_env_var_values(server.env_vars)
        view = SERVER_VIEW.validate_json(server.model_dump_json())
        if server.credentials is not None:
            view["credentials"] = mcp_db.decrypt_credentials(deepcopy(server.credentials))
        views.append(view)
    return views


async def create(client: FakeClient, call: Call) -> Outcome:
    request = NewMCPServerRequest.model_validate(call["payload"])
    return (await mcp_db.create_mcp_server(client, request, call["touched_by"])).server_id, None


async def create_if_free(client: FakeClient, call: Call) -> Outcome:
    request = NewMCPServerRequest.model_validate(call["payload"])
    created = await mcp_db.create_mcp_server_if_identifier_free(client, request, call["touched_by"])
    return written(created)


async def create_draft(client: FakeClient, call: Call) -> Outcome:
    request = NewMCPServerRequest.model_validate(call["payload"])
    ttl, draft_id = call["ttl_seconds"], call["server_id"]
    draft = await mcp_db.create_draft_mcp_server(client, request, call["touched_by"], ttl, draft_id)
    return draft.server_id, None


async def get_draft(client: FakeClient, call: Call) -> Outcome:
    ttl = call["ttl_seconds"]
    return written(await mcp_db.get_draft_mcp_server(client, call["server_id"], ttl))


async def update(client: FakeClient, call: Call) -> Outcome:
    changes = UpdateMCPServerRequest.model_validate(call["payload"])
    return written(await mcp_db.update_mcp_server(client, changes, call["touched_by"]))


async def delete(client: FakeClient, call: Call) -> Outcome:
    deleted = await mcp_db.delete_mcp_server(client, call["server_id"], client.record_invalidation)
    return (None if deleted is None else deleted.server_id), None


# One handler per LiteLLM operation a case can name.
HANDLERS: Final[Mapping[Function, Callable[[FakeClient, Call], Coroutine[None, None, Outcome]]]] = {
    "create": create, "create_if_free": create_if_free, "create_draft": create_draft,
    "get_draft": get_draft, "update": update, "delete": delete,
}


def run(call: Call) -> Answer:
    sys.modules[PROXY_SERVER] = ProxyServerStub()
    client = FakeClient(call["rows"], call["users"], call["server_id"])
    server_id, conflict = asyncio.run(HANDLERS[call["function"]](client, call))
    return {
        "server_id": server_id,
        "conflict": conflict,
        "rows": row_views(client),
        "invalidated": [user for user, _server in client.invalidated],
        "deleted_children": client.fake_db.deleted,
    }
