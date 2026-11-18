import asyncio
import sys
import types
from collections.abc import Sequence
from typing import TypedDict

from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.tool_registry_writer import ToolPolicyRegistry
from litellm.proxy.utils import PrismaClient
from litellm.types.tool_management import LiteLLM_ToolTableRow
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

JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Query(TypedDict):
    tool_names: list[str]
    key_permission_id: str | None
    team_permission_id: str | None


class Input(TypedDict):
    tools: list[dict[str, JsonValue]]
    permissions: list[dict[str, JsonValue]]
    queries: list[Query]
    output_tools: list[str]


class QueryAnswer(TypedDict):
    tool_names: list[str]
    key_permission_id: str | None
    team_permission_id: str | None
    effective: dict[str, str]


class Answer(TypedDict):
    initialized: bool
    queries: list[QueryAnswer]
    output_policies: dict[str, str]


class FakeToolTable:
    def __init__(self, rows: Sequence[LiteLLM_ToolTableRow]) -> None:
        self._rows = rows

    async def find_many(self) -> Sequence[LiteLLM_ToolTableRow]:
        await asyncio.sleep(0)
        return self._rows


class FakePermissionTable:
    def __init__(self, rows: Sequence[LiteLLM_ObjectPermissionTable]) -> None:
        self._rows = rows

    async def find_many(self) -> Sequence[LiteLLM_ObjectPermissionTable]:
        await asyncio.sleep(0)
        return self._rows


class FakeDb(PrismaWrapper):
    def __init__(self, tools: FakeToolTable, permissions: FakePermissionTable) -> None:
        self.litellm_tooltable = tools
        self.litellm_objectpermissiontable = permissions


class FakeClient(PrismaClient):
    def __init__(self, input_value: Input) -> None:
        self.db = FakeDb(
            FakeToolTable(
                [LiteLLM_ToolTableRow.model_validate(tool) for tool in input_value["tools"]]
            ),
            FakePermissionTable(
                [
                    LiteLLM_ObjectPermissionTable.model_validate(permission)
                    for permission in input_value["permissions"]
                ]
            ),
        )


def run(input_value: Input) -> JsonValue:
    registry = ToolPolicyRegistry()
    asyncio.run(registry.sync_tool_policy_from_db(FakeClient(input_value)))
    answer = Answer(
        initialized=registry.is_initialized(),
        queries=[
            QueryAnswer(
                tool_names=query["tool_names"],
                key_permission_id=query["key_permission_id"],
                team_permission_id=query["team_permission_id"],
                effective=registry.get_effective_policies(
                    query["tool_names"],
                    object_permission_id=query["key_permission_id"],
                    team_object_permission_id=query["team_permission_id"],
                ),
            )
            for query in input_value["queries"]
        ],
        output_policies={
            name: registry.get_output_policy(name) for name in input_value["output_tools"]
        },
    )
    return JSON_VALUE.validate_python(answer)
