import asyncio
from collections.abc import Mapping, Sequence
from typing import Final, TypedDict

from litellm.proxy.management_endpoints.router_weights import (
    validate_router_settings_weights,
)
from litellm.repositories.prisma_protocols import TableActions
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException
from typing_extensions import override


class DeploymentInput(TypedDict):
    model_name: str
    model_info: JsonValue


class Input(TypedDict):
    model_group: str
    deployment_id: str
    team_id: str | None
    stored: DeploymentInput | None
    configured: DeploymentInput | None


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)


class Row:
    def __init__(self, model_id: str, deployment: DeploymentInput) -> None:
        self.model_id = model_id
        self.model_name = deployment["model_name"]
        self.model_info = deployment["model_info"]


class Table(TableActions[Row]):
    def __init__(self, rows: Sequence[Row]) -> None:
        self.rows = tuple(rows)

    # LiteLLM's TableActions takes a Mapping with object values, so the override mirrors it
    @override
    async def find_many(
        self,
        take: int | None = None,
        skip: int | None = None,
        where: Mapping[str, object] | None = None,  # ast-grep-ignore: no-loose-types
        cursor: Mapping[str, object] | None = None,  # ast-grep-ignore: no-loose-types
        include: Mapping[str, object] | None = None,  # ast-grep-ignore: no-loose-types
        order: (
            Mapping[str, object]  # ast-grep-ignore: no-loose-types
            | Sequence[Mapping[str, object]]  # ast-grep-ignore: no-loose-types
            | None
        ) = None,
        distinct: Sequence[str] | None = None,
    ) -> Sequence[Row]:
        del take, skip, where, cursor, include, order, distinct
        return self.rows


class Db:
    def __init__(self, table: Table) -> None:
        self.litellm_proxymodeltable = table


class Client:
    def __init__(self, table: Table) -> None:
        self.db = Db(table)


class Deployment:
    def __init__(self, deployment: DeploymentInput) -> None:
        self.model_name = deployment["model_name"]
        self.model_info = deployment["model_info"]


class Router:
    def __init__(self, configured: DeploymentInput | None) -> None:
        self.configured = configured

    def get_deployment(self, model_id: str) -> Deployment | None:
        del model_id
        if self.configured is None:
            return None
        return Deployment(self.configured)


def stored_rows(request: Input) -> list[Row]:
    stored = request["stored"]
    if stored is None:
        return []
    return [Row(request["deployment_id"], stored)]


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    request = INPUT.validate_python(input_value)
    group_weights: dict[str, float] = {request["deployment_id"]: 1.0}
    weights: dict[str, dict[str, float]] = {request["model_group"]: group_weights}
    settings: dict[str, dict[str, dict[str, float]]] = {"weights": weights}
    try:
        asyncio.run(
            validate_router_settings_weights(
                settings,
                team_id=request["team_id"],
                prisma_client=Client(Table(stored_rows(request))),
                llm_router=Router(request["configured"]),
            )
        )
    except StarletteHTTPException as raised:
        return {"status": raised.status_code, "detail": raised.detail}
    return {"status": None, "detail": None}
