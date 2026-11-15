import asyncio
from typing import TypedDict

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.repositories.config_repository import ConfigParam, ConfigRepository
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class Input(TypedDict):
    raw: str


class FakeRow:
    def __init__(self, raw: str) -> None:
        self.param_name = "param_name"
        self.param_value = raw


class FakeTable:
    def __init__(self, row: FakeRow) -> None:
        self.row = row

    async def find_unique(self, *, where: dict[str, str]) -> FakeRow:
        del where
        return self.row


class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> FakeTable:
        del name
        return self.table


class FakeClient(PrismaClient):
    def __init__(self, table: FakeTable) -> None:
        self.db = FakeDb(table)


# get_param decodes a stored string through the pin's own config-value conversion.
def run(input_value: Input) -> JsonValue:
    table = FakeTable(FakeRow(input_value["raw"]))
    repository = ConfigRepository(FakeClient(table))
    param: ConfigParam | None = asyncio.run(repository.get_param("param_name"))
    if param is None:
        raise ValueError("the fake config table returned no row")
    return TypeAdapter(JsonValue).validate_python(param.param_value)
