import asyncio
import sys
from collections import UserDict
from copy import deepcopy
from datetime import datetime, timezone
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
JSON_COLUMNS: Final = (
    "credentials",
    "env_vars",
    "mcp_info",
    "tool_name_to_display_name",
    "tool_name_to_description",
)
STRING: Final[TypeAdapter[str]] = TypeAdapter(str)
JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
WINDOW: Final[TypeAdapter[Where]] = TypeAdapter(Where)
STAMP: Final[TypeAdapter[datetime]] = TypeAdapter(datetime)


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


ENV_VAR_VIEW: Final[TypeAdapter[EnvVarView]] = TypeAdapter(EnvVarView)


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


class Answer(TypedDict):
    server_id: str | None
    conflict: ConflictView | None
    rows: list[ServerView]
    invalidated: list[str]
    deleted_children: list[str]


class Call(TypedDict):
    function: Literal["create", "create_if_free", "create_draft", "get_draft", "update", "delete"]
    salt_key: str
    touched_by: str
    payload: Where
    rows: list[Where]
    server_id: str
    ttl_seconds: int
    users: list[str]


class FakeRow(UserDict[str, JsonValue | datetime]):
    @property
    def server_id(self) -> str:
        return STRING.validate_python(self["server_id"], strict=True)

    def __getattr__(self, name: str) -> JsonValue | datetime:
        try:
            return self[name]
        except KeyError as error:
            raise AttributeError(name) from error

    def __setattr__(self, name: str, value: JsonValue | datetime) -> None:
        if name == "data":
            super().__setattr__(name, value)
        else:
            self[name] = value

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
        self,
        kind: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        pass

    async def execute_raw(self, _query: str, *_keys: int) -> int:
        return 0

    async def create(self, data: Where) -> FakeRow:
        stored = FakeRow(parsed_columns(data))
        stored["updated_at"] = datetime.now(timezone.utc)
        self.rows.append(stored)
        return FakeRow(deepcopy(stored.data))

    async def update(self, where: Where, data: Where) -> FakeRow | None:
        found = await self.find_first(where)
        if found is not None:
            for column, value in parsed_columns(data).items():
                found[column] = value
        return None if found is None else FakeRow(deepcopy(found.data))

    async def delete(self, where: Where) -> FakeRow | None:
        found = await self.find_first(where)
        if found is not None:
            self.rows.remove(found)
        return None if found is None else FakeRow(deepcopy(found.data))

    async def delete_many(self, where: Where) -> int:
        self.deleted.append(self.label)
        removed = [row for row in self.rows if where_matches(row, where)]
        self.rows = [row for row in self.rows if not where_matches(row, where)]
        return len(removed)

    async def find_many(
        self,
        where: Where | None = None,
        take: int | None = None,
    ) -> list[FakeRow]:
        matches = self.rows if where is None else [
            row for row in self.rows if where_matches(row, where)
        ]
        return matches if take is None else matches[:take]

    async def find_first(self, where: Where) -> FakeRow | None:
        return next(iter(await self.find_many(where=where, take=1)), None)

    async def find_unique(self, where: Where) -> FakeRow | None:
        return await self.find_first(where)


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
        self.fake_db.litellm_mcpservertable.rows = [seed_row(row) for row in rows]
        self.fake_db.litellm_mcpusercredentials.rows = [
            FakeRow({"user_id": user, "server_id": server_id}) for user in users
        ]

    async def record_invalidation(self, user_id: str, server_id: str) -> None:
        self.invalidated.append((user_id, server_id))


def parsed_columns(data: Where) -> Where:
    parsed = dict(data)
    for column in JSON_COLUMNS:
        value = parsed.get(column)
        if value is not None:
            parsed[column] = JSON_VALUE.validate_json(STRING.validate_python(value))
    return parsed


def seed_row(row: Where) -> FakeRow:
    stored = FakeRow(row)
    stamp = stored.get("updated_at")
    if stamp is not None:
        stored["updated_at"] = STAMP.validate_python(stamp)
    return stored


def field_matches(value: JsonValue | datetime, condition: JsonValue | datetime) -> bool:
    try:
        operators = WINDOW.validate_python(condition)
    except ValidationError:
        return value == condition
    if "gte" in operators:
        return STAMP.validate_python(value) >= STAMP.validate_python(operators["gte"])
    if "in" in operators:
        return value in TypeAdapter(list[JsonValue]).validate_python(operators["in"])
    if "not" in operators:
        return value is not None and value != operators["not"]
    if operators.get("mode") == "insensitive":
        wanted = STRING.validate_python(operators["equals"]).lower()
        return value is not None and STRING.validate_python(value).lower() == wanted
    return value == operators.get("equals")


def where_matches(values: FakeRow, where: Where) -> bool:
    for key, condition in where.items():
        if key in {"AND", "OR", "NOT"}:
            branches = TypeAdapter(list[Where]).validate_python(condition)
            nested = [where_matches(values, branch) for branch in branches]
            if key == "AND":
                matched = all(nested)
            elif key == "OR":
                matched = any(nested)
            else:
                matched = not any(nested)
        else:
            matched = field_matches(values.get(key), condition)
        if not matched:
            return False
    return True


def conflict_view(conflict: mcp_db.McpIdentifierConflict | None) -> ConflictView | None:
    if conflict is None:
        return None
    return {"field": conflict.field, "value": conflict.value, "server_id": conflict.server_id}


def server_view(server: LiteLLM_MCPServerTable) -> ServerView:
    credentials = deepcopy(server.credentials)
    env_vars = server.env_vars
    views: list[EnvVarView] | None = (
        None
        if env_vars is None
        else [ENV_VAR_VIEW.validate_json(entry.model_dump_json()) for entry in env_vars]
    )
    return {
        "server_id": server.server_id,
        "server_name": server.server_name,
        "alias": server.alias,
        "description": server.description,
        "auth_type": server.auth_type,
        "approval_status": server.approval_status,
        "created_by": server.created_by,
        "updated_by": server.updated_by,
        "is_byok": server.is_byok,
        "credentials": None if credentials is None else mcp_db.decrypt_credentials(credentials),
        "env_vars": views,
        "issuer": server.issuer,
        "token_url": server.token_url,
        "oauth2_flow": server.oauth2_flow,
        "token_exchange_endpoint": server.token_exchange_endpoint,
        "audience": server.audience,
    }


def row_views(client: FakeClient) -> list[ServerView]:
    views = []
    for row in client.fake_db.litellm_mcpservertable.rows:
        server = LiteLLM_MCPServerTable.model_validate(row.model_dump())
        mcp_db.decrypt_global_env_var_values(server.env_vars)
        views.append(server_view(server))
    return views


def run(call: Call) -> Answer:
    sys.modules[PROXY_SERVER] = ProxyServerStub()
    client = FakeClient(call["rows"], call["users"], call["server_id"])
    payload = call["payload"]
    answer: Answer = {
        "server_id": None,
        "conflict": None,
        "rows": [],
        "invalidated": [],
        "deleted_children": [],
    }
    match call["function"]:
        case "create":
            request = NewMCPServerRequest.model_validate(payload)
            created = asyncio.run(mcp_db.create_mcp_server(client, request, call["touched_by"]))
            answer["server_id"] = created.server_id
        case "create_if_free":
            request = NewMCPServerRequest.model_validate(payload)
            created = asyncio.run(
                mcp_db.create_mcp_server_if_identifier_free(client, request, call["touched_by"])
            )
            match created:
                case mcp_db.McpIdentifierConflict() as conflict:
                    answer["conflict"] = conflict_view(conflict)
                case LiteLLM_MCPServerTable() as server:
                    answer["server_id"] = server.server_id
        case "create_draft":
            request = NewMCPServerRequest.model_validate(payload)
            created = asyncio.run(
                mcp_db.create_draft_mcp_server(
                    client, request, call["touched_by"], call["ttl_seconds"], call["server_id"]
                )
            )
            answer["server_id"] = created.server_id
        case "get_draft":
            found = asyncio.run(
                mcp_db.get_draft_mcp_server(client, call["server_id"], call["ttl_seconds"])
            )
            answer["server_id"] = None if found is None else found.server_id
        case "update":
            request = UpdateMCPServerRequest.model_validate(payload)
            updated = asyncio.run(mcp_db.update_mcp_server(client, request, call["touched_by"]))
            if updated is None:
                answer["server_id"] = None
            else:
                match updated:
                    case mcp_db.McpIdentifierConflict() as conflict:
                        answer["conflict"] = conflict_view(conflict)
                    case LiteLLM_MCPServerTable() as server:
                        answer["server_id"] = server.server_id
        case "delete":
            deleted = asyncio.run(
                mcp_db.delete_mcp_server(client, call["server_id"], client.record_invalidation)
            )
            answer["server_id"] = None if deleted is None else deleted.server_id
    answer["rows"] = row_views(client)
    answer["invalidated"] = [user for user, _server in client.invalidated]
    answer["deleted_children"] = client.fake_db.deleted
    return answer
