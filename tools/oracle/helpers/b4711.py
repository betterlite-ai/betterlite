import asyncio
from collections import UserDict
from contextlib import suppress
from types import TracebackType
from typing import Final, Literal, Self, TypedDict

from litellm.proxy._experimental.mcp_server import db as mcp_db
from litellm.proxy._types import LiteLLM_MCPServerTable, NewMCPServerRequest
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp_server.mcp_server_manager import PinnedMCPTool
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

Where = dict[str, JsonValue]


class ConflictView(TypedDict):
    field: Literal["server_name", "alias"]
    value: str
    server_id: str


class CreateOutput(TypedDict):
    conflict: ConflictView | None
    lock_keys: list[int]
    written: bool


class ServerView(TypedDict):
    server_id: str
    approval_status: str | None
    env: dict[str, str]


class ReviewView(TypedDict):
    server_id: str
    approval_status: str | None
    review_notes: str | None
    updated_by: str | None


class Reviewed(TypedDict):
    reviewed_at: str | None


class ReviewOutput(ReviewView):
    reviewed_at_present: bool


class PinnedView(TypedDict):
    pinned_tools: dict[str, Where] | None


class SubmissionCounts(TypedDict):
    total: int
    pending_review: int
    active: int
    rejected: int


class SubmissionsOutput(SubmissionCounts):
    server_ids: list[str]


class Rows(TypedDict):
    rows: list[Where]


class FindConflict(Rows):
    function: Literal["find_conflict"]
    server_name: str | None
    alias: str | None
    exclude_server_id: str | None


class CreateIfFree(Rows):
    function: Literal["create_if_free"]
    server_name: str | None
    alias: str | None


class GetAll(Rows):
    function: Literal["get_all"]
    approval_status: str | None
    server_ids: list[str] | None


class GetServer(Rows):
    function: Literal["get_server"]
    server_id: str


class Review(Rows):
    function: Literal["review"]
    action: Literal["approve", "reject"]
    server_id: str
    touched_by: str
    review_notes: str | None


class PinnedTools(Rows):
    function: Literal["pinned_tools"]
    server_id: str
    pinned_tools: Where | None
    touched_by: str


class Submissions(Rows):
    function: Literal["submissions"]


class ActiveSubmissions(Rows):
    function: Literal["active_submissions"]
    user_id: str


Reads = FindConflict | GetAll | GetServer | ActiveSubmissions
Input = Reads | CreateIfFree | Review | PinnedTools | Submissions
Views = ConflictView | ServerView | list[ServerView] | PinnedView
Answer = Views | CreateOutput | ReviewOutput | SubmissionsOutput | list[str] | None
STRING: Final = TypeAdapter(str)
SERVER_VIEW: Final = TypeAdapter(ServerView)
PINNED_VIEW: Final = TypeAdapter(PinnedView)
PINNED_INPUT: Final = TypeAdapter(dict[str, PinnedMCPTool] | None)


class FakeRow(UserDict[str, JsonValue]):
    @property
    def server_id(self) -> str:
        return STRING.validate_python(self["server_id"], strict=True)

    def model_dump(self) -> Where:
        return dict(self)


def field_matches(value: JsonValue, condition: JsonValue) -> bool:
    try:
        operators = TypeAdapter(Where).validate_python(condition)
    except ValidationError:
        return value == condition
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


class FakeTable(PrismaWrapper):
    def __init__(self, rows: list[Where]) -> None:
        self.rows = [FakeRow(row) for row in rows]
        self.locks: list[int] = []
        self.created = False

    def tx(self) -> Self:
        return self

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

    async def execute_raw(self, _query: str, *args: int) -> int:
        self.locks.extend(args)
        return 0

    # only get_mcp_submissions orders: newest submitted_at first, ties in table order
    async def find_many(
        self, where: Where, order: Where | None = None, take: int | None = None
    ) -> list[FakeRow]:
        selected = [row for row in self.rows if where_matches(row, where)]
        if order is not None:
            stamps = [STRING.validate_python(row["submitted_at"]) for row in selected]
            keyed = [(stamp, -index) for index, stamp in enumerate(stamps)]
            selected = [selected[-negated] for _, negated in sorted(keyed, reverse=True)]
        return selected if take is None else selected[:take]

    async def find_first(self, where: Where) -> FakeRow | None:
        return next(iter(await self.find_many(where=where, take=1)), None)

    find_unique = find_first

    async def create(self, data: Where) -> FakeRow:
        self.created = True
        raise LookupError(data)

    async def update(self, where: Where, data: Where) -> FakeRow | None:
        found = await self.find_first(where)
        if found is not None:
            found.update(data)
        return found


class FakeClient(PrismaClient):
    def __init__(self, rows: list[Where]) -> None:
        self.db = self.fake_table = FakeTable(rows)


def conflict_view(conflict: mcp_db.McpIdentifierConflict | None) -> ConflictView | None:
    if conflict is None:
        return None
    return {"field": conflict.field, "value": conflict.value, "server_id": conflict.server_id}


def create_call(client: FakeClient, call: CreateIfFree) -> CreateOutput:
    req = NewMCPServerRequest.model_construct(server_name=call["server_name"], alias=call["alias"])
    conflict: mcp_db.McpIdentifierConflict | None = None
    with suppress(LookupError):
        match asyncio.run(mcp_db.create_mcp_server_if_identifier_free(client, req, "tester")):
            case mcp_db.McpIdentifierConflict() as found:
                conflict = found
            case LiteLLM_MCPServerTable():
                pass
    table = client.fake_table
    return {"conflict": conflict_view(conflict), "lock_keys": table.locks, "written": table.created}


def review_call(client: FakeClient, value: Review) -> ReviewOutput:
    if value["action"] == "approve":
        review = mcp_db.approve_mcp_server(client, value["server_id"], value["touched_by"])
    else:
        review = mcp_db.reject_mcp_server(
            client, value["server_id"], value["touched_by"], value["review_notes"]
        )
    dump = asyncio.run(review).model_dump_json()
    reviewed_at = TypeAdapter(Reviewed).validate_json(dump)["reviewed_at"]
    view = TypeAdapter(ReviewView).validate_json(dump)
    return {**view, "reviewed_at_present": reviewed_at is not None}


def run(call: Input) -> Answer:
    answer: Answer = None
    client = FakeClient(call["rows"])
    match call["function"]:
        case "find_conflict":
            found = mcp_db.find_mcp_server_identifier_conflict(
                client,
                server_name=call["server_name"],
                alias=call["alias"],
                exclude_server_id=call["exclude_server_id"],
            )
            answer = conflict_view(asyncio.run(found))
        case "create_if_free":
            answer = create_call(client, call)
        case "get_all":
            ids = call["server_ids"]
            if ids is None:
                reads = mcp_db.get_all_mcp_servers(client, call["approval_status"])
            else:
                reads = mcp_db.get_mcp_servers(client, ids)
            answer = [SERVER_VIEW.validate_json(s.model_dump_json()) for s in asyncio.run(reads)]
        case "get_server":
            server = asyncio.run(mcp_db.get_mcp_server(client, call["server_id"]))
            answer = None if server is None else SERVER_VIEW.validate_json(server.model_dump_json())
        case "review":
            answer = review_call(client, call)
        case "pinned_tools":
            tools = PINNED_INPUT.validate_python(call["pinned_tools"])
            pin = mcp_db.set_mcp_server_pinned_tools(
                client, call["server_id"], tools, call["touched_by"]
            )
            server = asyncio.run(pin)
            answer = None if server is None else PINNED_VIEW.validate_json(server.model_dump_json())
        case "submissions":
            summary = asyncio.run(mcp_db.get_mcp_submissions(client))
            counts = TypeAdapter(SubmissionCounts).validate_json(summary.model_dump_json())
            answer = SubmissionsOutput(**counts, server_ids=[i.server_id for i in summary.items])
        case "active_submissions":
            active = mcp_db.get_active_submitted_mcp_server_ids_for_user(client, call["user_id"])
            answer = asyncio.run(active)
    return answer
