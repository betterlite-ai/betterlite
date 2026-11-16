import asyncio
from collections import UserDict
from types import TracebackType
from typing import Final, Literal, Self, TypedDict

from litellm.proxy._experimental.mcp_server import db as mcp_db
from litellm.proxy.common_utils.encrypt_decrypt_utils import decrypt_value_helper
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

KEY: Final = "mcp_user_env_vars"
COMPOSITE_KEY: Final = "user_id_server_id"
Where = dict[str, JsonValue]

STRING: Final = TypeAdapter(str)
WHERE: Final = TypeAdapter(Where)
TEXTS: Final = TypeAdapter(dict[str, str])
JSON: Final = TypeAdapter(JsonValue)
IN_VALUES: Final = TypeAdapter(list[JsonValue])


class RowView(TypedDict):
    user_id: str
    server_id: str
    values: JsonValue


class Answer(TypedDict):
    result: dict[str, str] | dict[str, dict[str, str]] | None
    rows: list[RowView]
    events: list[str]
    locks: list[int]


class Rows(TypedDict):
    salt_key: str
    rows: list[Where]


class Get(Rows):
    function: Literal["get"]
    user_id: str
    server_id: str


class Bulk(Rows):
    function: Literal["bulk"]
    user_id: str
    server_ids: list[str]


class Merge(Rows):
    function: Literal["merge"]
    user_id: str
    server_id: str
    updates: dict[str, str]
    allowed_names: list[str]


class Delete(Rows):
    function: Literal["delete"]
    user_id: str
    server_id: str


Input = Get | Bulk | Merge | Delete


class FakeRow(UserDict[str, JsonValue]):
    @property
    def values_b64(self) -> str:
        return STRING.validate_python(self["values_b64"], strict=True)

    @property
    def server_id(self) -> str:
        return STRING.validate_python(self["server_id"], strict=True)


def field_matches(value: JsonValue, condition: JsonValue) -> bool:
    try:
        operators = WHERE.validate_python(condition)
    except ValidationError:
        return value == condition
    if "in" in operators:
        return value in IN_VALUES.validate_python(operators["in"])
    return value == operators.get("equals")


def row_matches(row: FakeRow, where: Where) -> bool:
    for key, condition in where.items():
        if key == COMPOSITE_KEY:
            composite = TEXTS.validate_python(condition)
            matched = row["user_id"] == composite["user_id"] and row["server_id"] == composite["server_id"]
        else:
            matched = field_matches(row.get(key), condition)
        if not matched:
            return False
    return True


class FakeTable(PrismaWrapper):
    def __init__(self, rows: list[Where]) -> None:
        self.rows = [FakeRow(row) for row in rows]
        self.events: list[str] = []
        self.locks: list[int] = []

    def tx(self) -> Self:
        self.events.append("tx")
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
        del kind, exc, tb

    async def execute_raw(self, _query: str, *args: int) -> int:
        self.events.append("lock")
        self.locks.extend(args)
        return 0

    def selected(self, where: Where) -> list[FakeRow]:
        return [row for row in self.rows if row_matches(row, where)]

    async def find_unique(self, where: Where) -> FakeRow | None:
        self.events.append("find_unique")
        return next(iter(self.selected(where)), None)

    async def find_many(self, where: Where) -> list[FakeRow]:
        self.events.append("find_many")
        return self.selected(where)

    async def upsert(self, where: Where, data: Where) -> FakeRow:
        self.events.append("upsert")
        create = TEXTS.validate_python(data["create"])
        update = TEXTS.validate_python(data["update"])
        found = next(iter(self.selected(where)), None)
        if found is None:
            created: Where = {
                "user_id": create["user_id"],
                "server_id": create["server_id"],
                "values_b64": create["values_b64"],
            }
            row = FakeRow(created)
            self.rows.append(row)
            return row
        found.update(update)
        return found

    async def delete_many(self, where: Where) -> int:
        self.events.append("delete_many")
        kept = [row for row in self.rows if not row_matches(row, where)]
        removed = len(self.rows) - len(kept)
        self.rows = kept
        return removed


class FakeClient(PrismaClient):
    def __init__(self, rows: list[Where]) -> None:
        self.db = self.fake_table = FakeTable(rows)


def stored_value(stored: str) -> JsonValue:
    decrypted = decrypt_value_helper(
        value=stored,
        key=KEY,
        exception_type="debug",
        return_original_value=False,
    )
    if decrypted is None:
        return None
    parsed: JsonValue
    try:
        parsed = JSON.validate_json(decrypted)
    except ValidationError:
        return decrypted
    return parsed


def row_view(row: FakeRow) -> RowView:
    return {
        "user_id": STRING.validate_python(row["user_id"], strict=True),
        "server_id": row.server_id,
        "values": stored_value(row.values_b64),
    }


def run(call: Input) -> Answer:
    client = FakeClient(call["rows"])
    result: dict[str, str] | dict[str, dict[str, str]] | None = None
    match call["function"]:
        case "get":
            result = asyncio.run(mcp_db.get_user_env_vars(client, call["user_id"], call["server_id"]))
        case "bulk":
            result = asyncio.run(mcp_db.get_user_env_vars_bulk(client, call["user_id"], call["server_ids"]))
        case "merge":
            result = asyncio.run(
                mcp_db.merge_user_env_vars(
                    client,
                    call["user_id"],
                    call["server_id"],
                    call["updates"],
                    call["allowed_names"],
                )
            )
        case "delete":
            asyncio.run(mcp_db.delete_user_env_vars(client, call["user_id"], call["server_id"]))
    table = client.fake_table
    return {
        "result": result,
        "rows": [row_view(row) for row in table.rows],
        "events": table.events,
        "locks": table.locks,
    }
