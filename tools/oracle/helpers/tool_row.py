import asyncio
import sys
import types
from collections.abc import Mapping, Sequence
from typing import TypedDict

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.tool_registry_writer import list_tools
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter


class PrismaUnavailableError(Exception):
    pass


class PrismaErrorsModule(types.ModuleType):
    PrismaError = PrismaUnavailableError


class PrismaModule(types.ModuleType):
    errors: PrismaErrorsModule = PrismaErrorsModule("prisma.errors")


_PRISMA = PrismaModule("prisma")
sys.modules.setdefault("prisma", _PRISMA)
sys.modules.setdefault("prisma.errors", _PRISMA.errors)

ROW_VALUE: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class TokenRow(TypedDict):
    token: str
    user_id: str | None


class UserRow(TypedDict):
    user_id: str
    user_email: str | None
    user_alias: str | None


class Input(TypedDict):
    rows: list[dict[str, JsonValue]]
    tokens: list[TokenRow]
    users: list[UserRow]


class FakeToolTable:
    def __init__(self, rows: Sequence[dict[str, JsonValue]]) -> None:
        self.rows = rows

    async def find_many(
        self,
        *,
        where: Mapping[str, JsonValue],
        order: Mapping[str, str],
    ) -> Sequence[dict[str, JsonValue]]:
        del where, order
        await asyncio.sleep(0)
        return self.rows


class FakeTokenTable:
    def __init__(self, rows: Sequence[TokenRow]) -> None:
        self.rows = rows

    async def find_many(
        self,
        *,
        where: Mapping[str, Mapping[str, Sequence[str]]],
    ) -> Sequence[TokenRow]:
        await asyncio.sleep(0)
        wanted = where["token"]["in"]
        return [row for row in self.rows if row["token"] in wanted]


class FakeUserTable:
    def __init__(self, rows: Sequence[UserRow]) -> None:
        self.rows = rows

    async def find_many(
        self,
        *,
        where: Mapping[str, Mapping[str, Sequence[str]]],
    ) -> Sequence[UserRow]:
        await asyncio.sleep(0)
        wanted = where["user_id"]["in"]
        return [row for row in self.rows if row["user_id"] in wanted]


class FakeDb(PrismaWrapper):
    def __init__(
        self,
        tools: FakeToolTable,
        tokens: FakeTokenTable,
        users: FakeUserTable,
    ) -> None:
        self.litellm_tooltable = tools
        self.litellm_verificationtoken = tokens
        self.litellm_usertable = users


class FakeClient(PrismaClient):
    def __init__(self, input_value: Input) -> None:
        self.db = FakeDb(
            FakeToolTable(input_value["rows"]),
            FakeTokenTable(input_value["tokens"]),
            FakeUserTable(input_value["users"]),
        )


def run(input_value: Input) -> JsonValue:
    client = FakeClient(input_value)
    tools = asyncio.run(list_tools(client))
    rows: list[JsonValue] = [ROW_VALUE.validate_json(tool.model_dump_json()) for tool in tools]
    answer: JsonValue = {"tools": rows}
    return answer
