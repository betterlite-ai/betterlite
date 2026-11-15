import asyncio
import json
import sys
from collections.abc import Mapping
from datetime import UTC, datetime
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

from litellm.proxy._experimental.mcp_server.toolset_db import (
    MCPToolsetRow,
    MCPToolsetRowData,
    create_mcp_toolset,
    delete_mcp_toolset,
    get_mcp_toolset,
    get_mcp_toolset_by_name,
    list_mcp_toolsets,
    update_mcp_toolset,
)
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp_server.mcp_toolset import (
    MCPToolset,
    MCPToolsetTool,
    NewMCPToolsetRequest,
    UpdateMCPToolsetRequest,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class RecordNotFoundError(Exception):
    pass


class PrismaErrorsModule(ModuleType):
    def __init__(self) -> None:
        super().__init__("prisma.errors")
        self.RecordNotFoundError = RecordNotFoundError


PRISMA_ERRORS: Final = PrismaErrorsModule()
sys.modules["prisma"] = ModuleType("prisma")
sys.modules["prisma.errors"] = PRISMA_ERRORS


class DatabaseUnavailableError(Exception):
    pass


class IdWhere(TypedDict):
    toolset_id: str


class NameWhere(TypedDict):
    toolset_name: str


class CreatedRow(TypedDict):
    toolset_id: str
    toolset_name: str
    tools: str
    created_by: str
    updated_by: str
    description: NotRequired[str]


class UpdatedRow(TypedDict):
    updated_by: str
    toolset_name: NotRequired[str]
    description: NotRequired[str | None]
    tools: NotRequired[str]


class RowInput(TypedDict):
    toolset_id: str
    toolset_name: str
    description: NotRequired[str | None]
    tools: NotRequired[list[MCPToolsetTool] | None]


class StoredRow(TypedDict):
    toolset_id: str
    toolset_name: str
    description: str | None
    tools: str
    created_at: datetime
    created_by: str | None
    updated_at: datetime
    updated_by: str | None


class Call(TypedDict):
    timestamp: str
    rows: list[RowInput]


class CreateCall(Call):
    operation: Literal["create"]
    touched_by: str
    request: dict[str, JsonValue]


class GetCall(Call):
    operation: Literal["get"]
    toolset_id: str


class GetByNameCall(Call):
    operation: Literal["get_by_name"]
    toolset_name: str


class ListCall(Call):
    operation: Literal["list"]
    toolset_ids: list[str] | None
    use_writer: bool
    fail: bool


class UpdateCall(Call):
    operation: Literal["update"]
    touched_by: str
    request: dict[str, JsonValue]


class DeleteCall(Call):
    operation: Literal["delete"]
    toolset_id: str


Input = CreateCall | GetCall | GetByNameCall | ListCall | UpdateCall | DeleteCall


class FakeRow(MCPToolsetRow):
    def __init__(self, row: StoredRow) -> None:
        self.row = row

    @override
    def model_dump(self) -> MCPToolsetRowData:
        return TypeAdapter(MCPToolsetRowData).validate_python(self.row)


class State:
    def __init__(self, rows: list[StoredRow], timestamp: datetime) -> None:
        self.rows = rows
        self.timestamp = timestamp
        self.seen: list[str] = []


class FakeTable:
    def __init__(self, state: State, label: str, *, fail: bool) -> None:
        self.state = state
        self.label = label
        self.fail = fail

    async def create(self, data: Mapping[str, JsonValue]) -> FakeRow:
        fields = TypeAdapter(CreatedRow).validate_python(data)
        row: StoredRow = {
            "toolset_id": fields["toolset_id"],
            "toolset_name": fields["toolset_name"],
            "description": fields.get("description"),
            "tools": fields["tools"],
            "created_at": self.state.timestamp,
            "created_by": fields["created_by"],
            "updated_at": self.state.timestamp,
            "updated_by": fields["updated_by"],
        }
        self.state.rows.append(row)
        return FakeRow(row)

    async def find_unique(self, where: Mapping[str, JsonValue]) -> FakeRow | None:
        wanted = TypeAdapter(IdWhere).validate_python(where)["toolset_id"]
        return next(
            (FakeRow(row) for row in self.state.rows if row["toolset_id"] == wanted), None
        )

    async def find_first(self, where: Mapping[str, JsonValue]) -> FakeRow | None:
        wanted = TypeAdapter(NameWhere).validate_python(where)["toolset_name"]
        return next(
            (FakeRow(row) for row in self.state.rows if row["toolset_name"] == wanted), None
        )

    async def find_many(self, where: Mapping[str, JsonValue]) -> list[FakeRow]:
        self.state.seen.append(self.label)
        if self.fail:
            raise DatabaseUnavailableError("the toolset table read failed")
        if where.get("toolset_id") is None:
            return [FakeRow(row) for row in self.state.rows]
        identifiers = TypeAdapter(dict[str, list[str]]).validate_python(where["toolset_id"])
        return [FakeRow(row) for row in self.state.rows if row["toolset_id"] in identifiers["in"]]

    async def update(
        self, where: Mapping[str, JsonValue], data: Mapping[str, JsonValue]
    ) -> FakeRow:
        toolset_id = TypeAdapter(IdWhere).validate_python(where)["toolset_id"]
        fields = TypeAdapter(UpdatedRow).validate_python(data)
        for row in self.state.rows:
            if row["toolset_id"] != toolset_id:
                continue
            if "toolset_name" in fields:
                row["toolset_name"] = fields["toolset_name"]
            if "description" in fields:
                row["description"] = fields["description"]
            if "tools" in fields:
                row["tools"] = fields["tools"]
            row["updated_by"] = fields["updated_by"]
            row["updated_at"] = self.state.timestamp
            return FakeRow(row)
        raise RecordNotFoundError(f"the toolset {toolset_id} does not exist")

    async def delete(self, where: Mapping[str, JsonValue]) -> FakeRow:
        toolset_id = TypeAdapter(IdWhere).validate_python(where)["toolset_id"]
        for index, row in enumerate(self.state.rows):
            if row["toolset_id"] == toolset_id:
                del self.state.rows[index]
                return FakeRow(row)
        raise RecordNotFoundError(f"the toolset {toolset_id} does not exist")


class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> FakeTable:
        if name != "litellm_mcptoolsettable":
            raise AttributeError(name)
        return self.table


class FakeClient(PrismaClient):
    def __init__(self, state: State, *, fail: bool) -> None:
        self.db = FakeDb(FakeTable(state, "reader", fail=fail))
        self.writer = FakeDb(FakeTable(state, "writer", fail=fail))

    @property
    @override
    def writer_db(self) -> FakeDb:
        return self.writer


def stored_row(row: RowInput, timestamp: datetime) -> StoredRow:
    tools = row.get("tools")
    return {
        "toolset_id": row["toolset_id"],
        "toolset_name": row["toolset_name"],
        "description": row.get("description"),
        "tools": "[]" if tools is None else json.dumps(tools),
        "created_at": timestamp,
        "created_by": None,
        "updated_at": timestamp,
        "updated_by": None,
    }


def toolset_value(toolset: MCPToolset | None) -> JsonValue:
    if toolset is None:
        return None
    return TypeAdapter(JsonValue).validate_json(toolset.model_dump_json())


async def dispatch(call: Input, state: State) -> JsonValue:
    match call["operation"]:
        case "create":
            client = FakeClient(state, fail=False)
            request = TypeAdapter(NewMCPToolsetRequest).validate_python(call["request"])
            created = await create_mcp_toolset(client, request, call["touched_by"])
            return toolset_value(created)
        case "get":
            found = await get_mcp_toolset(FakeClient(state, fail=False), call["toolset_id"])
            return toolset_value(found)
        case "get_by_name":
            client = FakeClient(state, fail=False)
            named = await get_mcp_toolset_by_name(client, call["toolset_name"])
            return toolset_value(named)
        case "list":
            client = FakeClient(state, fail=call["fail"])
            toolsets = await list_mcp_toolsets(
                client, call["toolset_ids"], use_writer=call["use_writer"]
            )
            return {
                "toolsets": [toolset_value(toolset) for toolset in toolsets],
                "store": state.seen[0],
            }
        case "update":
            client = FakeClient(state, fail=False)
            changes = TypeAdapter(UpdateMCPToolsetRequest).validate_python(call["request"])
            updated = await update_mcp_toolset(client, changes, call["touched_by"])
            return toolset_value(updated)
        case "delete":
            removed = await delete_mcp_toolset(FakeClient(state, fail=False), call["toolset_id"])
            return toolset_value(removed)


def run(input_value: Input) -> JsonValue:
    parsed = TypeAdapter(datetime).validate_python(input_value["timestamp"])
    timestamp = parsed.replace(tzinfo=UTC)
    state = State([stored_row(row, timestamp) for row in input_value["rows"]], timestamp)
    return asyncio.run(dispatch(input_value, state))
