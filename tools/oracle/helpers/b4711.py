import asyncio
from types import TracebackType
from typing import Final, Literal, Self, TypedDict

from litellm.proxy._experimental.mcp_server.db import (
    McpIdentifierConflict,
    approve_mcp_server,
    create_mcp_server_if_identifier_free,
    find_mcp_server_identifier_conflict,
    get_active_submitted_mcp_server_ids_for_user,
    get_all_mcp_servers,
    get_mcp_server,
    get_mcp_servers,
    get_mcp_submissions,
    reject_mcp_server,
    set_mcp_server_pinned_tools,
)
from litellm.proxy._types import LiteLLM_MCPServerTable, NewMCPServerRequest
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp_server.mcp_server_manager import PinnedMCPTool
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

JSON_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
JSON_BRANCHES: Final = TypeAdapter(list[dict[str, JsonValue]])
JSON_ITEMS: Final = TypeAdapter(list[JsonValue])
JSON_VALUE: Final = TypeAdapter(JsonValue)
STRING: Final = TypeAdapter(str)


class ServerView(TypedDict):
    server_id: str
    approval_status: str | None
    env: dict[str, str]


SERVER_VIEW: Final = TypeAdapter(ServerView)


class ReviewView(TypedDict):
    server_id: str
    approval_status: str | None
    review_notes: str | None
    updated_by: str | None
    reviewed_at: str | None


REVIEW_VIEW: Final = TypeAdapter(ReviewView)


class PinnedToolView(TypedDict):
    description: str
    input_schema: dict[str, JsonValue]


class PinnedView(TypedDict):
    pinned_tools: dict[str, PinnedToolView] | None


PINNED_VIEW: Final = TypeAdapter(PinnedView)
PINNED_INPUT: Final = TypeAdapter(dict[str, PinnedMCPTool] | None)


class ConflictView(TypedDict):
    field: Literal["alias", "server_name"]
    value: str
    server_id: str


CONFLICT_VIEW: Final = TypeAdapter(ConflictView)


class CapturedCreateError(Exception):
    pass


class FindConflict(TypedDict):
    function: Literal["find_conflict"]
    server_name: str | None
    alias: str | None
    exclude_server_id: str | None
    rows: list[JsonValue]


class CreateIfFree(TypedDict):
    function: Literal["create_if_free"]
    server_name: JsonValue
    alias: JsonValue
    rows: list[JsonValue]


class GetAll(TypedDict):
    function: Literal["get_all"]
    salt_key: str
    approval_status: str | None
    server_ids: list[str] | None
    rows: list[JsonValue]


class GetServer(TypedDict):
    function: Literal["get_server"]
    salt_key: str
    server_id: str
    rows: list[JsonValue]


class Review(TypedDict):
    function: Literal["review"]
    salt_key: str
    action: Literal["approve", "reject"]
    server_id: str
    touched_by: str
    review_notes: str | None
    rows: list[JsonValue]


class PinnedTools(TypedDict):
    function: Literal["pinned_tools"]
    salt_key: str
    server_id: str
    pinned_tools: dict[str, JsonValue] | None
    touched_by: str
    rows: list[JsonValue]


class Submissions(TypedDict):
    function: Literal["submissions"]
    salt_key: str
    rows: list[JsonValue]


class ActiveSubmissions(TypedDict):
    function: Literal["active_submissions"]
    user_id: str
    rows: list[JsonValue]


Input = FindConflict | CreateIfFree | GetAll | GetServer | Review | PinnedTools | Submissions | ActiveSubmissions


def insensitive_equals(value: JsonValue, expected: JsonValue) -> bool:
    try:
        left = STRING.validate_python(value, strict=True)
        right = STRING.validate_python(expected, strict=True)
    except ValidationError:
        return False
    return left.lower() == right.lower()


def field_matches(value: JsonValue, condition: JsonValue) -> bool:
    try:
        operators = JSON_OBJECT.validate_python(condition, strict=True)
    except ValidationError:
        return value == condition
    if "equals" in operators:
        equals = operators["equals"]
        if operators.get("mode") == "insensitive":
            return insensitive_equals(value, equals)
        return value == equals
    if "not" in operators:
        negated = operators["not"]
        return value is not None and value != negated
    if "in" in operators:
        options = JSON_ITEMS.validate_python(operators["in"], strict=True)
        return value in options
    return False


def where_matches(values: dict[str, JsonValue], where: dict[str, JsonValue]) -> bool:
    for key, condition in where.items():
        if key in {"AND", "OR", "NOT"}:
            branches = JSON_BRANCHES.validate_python(condition, strict=True)
            matched = [where_matches(values, branch) for branch in branches]
            if key == "AND" and not all(matched):
                return False
            if key == "OR" and not any(matched):
                return False
            if key == "NOT" and any(matched):
                return False
        elif not field_matches(values.get(key), condition):
            return False
    return True


class FakeRow:
    def __init__(self, data: dict[str, JsonValue]) -> None:
        self.data = dict(data)
        self.server_id = STRING.validate_python(self.data["server_id"], strict=True)

    def model_dump(self, mode: Literal["python", "json"] = "python") -> dict[str, JsonValue]:
        if mode == "json":
            return JSON_OBJECT.validate_python(self.data)
        return self.data


def submitted_at_of(row: FakeRow) -> str:
    values = row.model_dump(mode="json")
    return STRING.validate_python(values.get("submitted_at"), strict=True)


class FakeTable:
    def __init__(self, rows: list[JsonValue]) -> None:
        self.rows = [FakeRow(JSON_OBJECT.validate_python(row, strict=True)) for row in rows]
        self.locks: list[int] = []
        self.created = False

    async def find_first(self, where: dict[str, JsonValue]) -> FakeRow | None:
        for row in self.rows:
            if where_matches(row.model_dump(mode="json"), where):
                return row
        return None

    async def find_unique(self, where: dict[str, JsonValue]) -> FakeRow | None:
        matches = await self.find_many(where=where, order=None, take=1)
        return matches[0] if matches else None

    async def find_many(
        self,
        where: dict[str, JsonValue] | None = None,
        order: dict[str, JsonValue] | None = None,
        take: int | None = None,
    ) -> list[FakeRow]:
        selected = [
            row
            for row in self.rows
            if where is None or where_matches(row.model_dump(mode="json"), where)
        ]
        if order is not None:
            direction = order.get("submitted_at")
            if direction != "desc":
                raise ValueError("the fake table sorts only submitted_at descending")
            selected.sort(key=submitted_at_of, reverse=True)
        return selected[:take] if take is not None else selected

    async def create(self, data: dict[str, JsonValue]) -> FakeRow:
        self.created = True
        raise CapturedCreateError(data)

    async def update(
        self, where: dict[str, JsonValue], data: dict[str, JsonValue]
    ) -> FakeRow | None:
        for row in self.rows:
            if where_matches(row.model_dump(mode="json"), where):
                row.data.update(data)
                return row
        return None


class FakeTransaction:
    def __init__(self, table: FakeTable) -> None:
        self.table = table

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        del exc_type, exc, traceback

    async def execute_raw(self, query: str, *args: int) -> int:
        del query
        self.table.locks.extend(args)
        return 0

    @property
    def litellm_mcpservertable(self) -> FakeTable:
        return self.table


class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeTable) -> None:
        self.table = table

    def tx(self) -> FakeTransaction:
        return FakeTransaction(self.table)

    @override
    def __getattr__(self, name: str) -> FakeTable:
        del name
        return self.table


class FakeClient(PrismaClient):
    def __init__(self, table: FakeTable) -> None:
        self.db = FakeDb(table)


def server_views(servers: list[LiteLLM_MCPServerTable]) -> list[JsonValue]:
    return [
        JSON_VALUE.validate_python(SERVER_VIEW.validate_json(server.model_dump_json()))
        for server in servers
    ]


def conflict_output(conflict: McpIdentifierConflict | None) -> JsonValue:
    if conflict is None:
        return None
    view = CONFLICT_VIEW.validate_python(
        {"field": conflict.field, "value": conflict.value, "server_id": conflict.server_id}
    )
    return JSON_VALUE.validate_python(view)


def conflict_of(written: LiteLLM_MCPServerTable | McpIdentifierConflict) -> JsonValue:
    conflict: JsonValue = None
    match written:
        case McpIdentifierConflict():
            conflict = conflict_output(written)
        case LiteLLM_MCPServerTable():
            conflict = None
    return conflict


def create_output(table: FakeTable, conflict: JsonValue) -> JsonValue:
    return JSON_VALUE.validate_python(
        {"conflict": conflict, "lock_keys": table.locks, "written": table.created}
    )


def find_conflict_call(value: FindConflict) -> JsonValue:
    client = FakeClient(FakeTable(value["rows"]))
    conflict = asyncio.run(
        find_mcp_server_identifier_conflict(
            client,
            server_name=value["server_name"],
            alias=value["alias"],
            exclude_server_id=value["exclude_server_id"],
        )
    )
    return JSON_VALUE.validate_python({"conflict": conflict_output(conflict)})


def create_if_free_call(value: CreateIfFree) -> JsonValue:
    request = NewMCPServerRequest.model_construct(
        server_name=value["server_name"],
        alias=value["alias"],
        transport="sse",
    )
    table = FakeTable(value["rows"])
    try:
        written = asyncio.run(
            create_mcp_server_if_identifier_free(FakeClient(table), request, "tester")
        )
    except CapturedCreateError:
        return create_output(table, None)
    return create_output(table, conflict_of(written))


def get_all_call(value: GetAll) -> JsonValue:
    client = FakeClient(FakeTable(value["rows"]))
    server_ids = value["server_ids"]
    if server_ids is None:
        servers = asyncio.run(get_all_mcp_servers(client, value["approval_status"]))
    else:
        servers = asyncio.run(get_mcp_servers(client, server_ids))
    return JSON_VALUE.validate_python({"servers": server_views(servers)})


def get_server_call(value: GetServer) -> JsonValue:
    server = asyncio.run(get_mcp_server(FakeClient(FakeTable(value["rows"])), value["server_id"]))
    if server is None:
        return JSON_VALUE.validate_python({"server": None})
    view = SERVER_VIEW.validate_json(server.model_dump_json())
    return JSON_VALUE.validate_python({"server": view})


def review_call(value: Review) -> JsonValue:
    client = FakeClient(FakeTable(value["rows"]))
    if value["action"] == "approve":
        server = asyncio.run(approve_mcp_server(client, value["server_id"], value["touched_by"]))
    else:
        server = asyncio.run(
            reject_mcp_server(
                client, value["server_id"], value["touched_by"], value["review_notes"]
            )
        )
    view = REVIEW_VIEW.validate_json(server.model_dump_json())
    return JSON_VALUE.validate_python(
        {
            "server": {
                "server_id": view["server_id"],
                "approval_status": view["approval_status"],
                "review_notes": view["review_notes"],
                "updated_by": view["updated_by"],
                "reviewed_at_present": view["reviewed_at"] is not None,
            }
        }
    )


def pinned_tools_call(value: PinnedTools) -> JsonValue:
    pinned = PINNED_INPUT.validate_python(value["pinned_tools"])
    server = asyncio.run(
        set_mcp_server_pinned_tools(
            FakeClient(FakeTable(value["rows"])),
            value["server_id"],
            pinned,
            value["touched_by"],
        )
    )
    if server is None:
        return JSON_VALUE.validate_python({"found": False, "pinned_tools": None})
    view = PINNED_VIEW.validate_json(server.model_dump_json())
    return JSON_VALUE.validate_python({"found": True, "pinned_tools": view["pinned_tools"]})


def submissions_call(value: Submissions) -> JsonValue:
    summary = asyncio.run(get_mcp_submissions(FakeClient(FakeTable(value["rows"]))))
    items = [item.server_id for item in summary.items]
    return JSON_VALUE.validate_python(
        {
            "total": summary.total,
            "pending_review": summary.pending_review,
            "active": summary.active,
            "rejected": summary.rejected,
            "first_server_id": items[0] if items else None,
            "last_server_id": items[-1] if items else None,
        }
    )


def active_submissions_call(value: ActiveSubmissions) -> JsonValue:
    server_ids = asyncio.run(
        get_active_submitted_mcp_server_ids_for_user(
            FakeClient(FakeTable(value["rows"])), value["user_id"]
        )
    )
    return JSON_VALUE.validate_python({"server_ids": server_ids})


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "find_conflict":
            answer = find_conflict_call(input_value)
        case "create_if_free":
            answer = create_if_free_call(input_value)
        case "get_all":
            answer = get_all_call(input_value)
        case "get_server":
            answer = get_server_call(input_value)
        case "review":
            answer = review_call(input_value)
        case "pinned_tools":
            answer = pinned_tools_call(input_value)
        case "submissions":
            answer = submissions_call(input_value)
        case "active_submissions":
            answer = active_submissions_call(input_value)
    return answer
