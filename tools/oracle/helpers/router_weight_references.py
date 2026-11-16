import asyncio
from collections.abc import Mapping, Sequence
from typing import Final, TypeAlias, TypedDict

from litellm.proxy.management_endpoints.router_weights import validate_router_settings_weights
from litellm.repositories.prisma_protocols import TableActions
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException
from typing_extensions import override

Filters: TypeAlias = Mapping[str, object]  # ast-grep-ignore: no-loose-types
OrderBy: TypeAlias = Filters | Sequence[Filters] | None
UNIMPLEMENTED: Final = "the router weight helper reads rows only through find_many"


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


# LiteLLM types the client as this protocol, so the fake answers every member it can be asked for
class Table(TableActions[Row]):
    def __init__(self, rows: Sequence[Row]) -> None:
        self.rows = tuple(rows)

    @override
    async def find_many(
        self,
        take: int | None = None,
        skip: int | None = None,
        where: Filters | None = None,
        cursor: Filters | None = None,
        include: Filters | None = None,
        order: OrderBy = None,
        distinct: Sequence[str] | None = None,
    ) -> Sequence[Row]:
        del take, skip, where, cursor, include, order, distinct
        return self.rows

    @override
    async def find_unique(self, where: Filters, include: Filters | None = None) -> Row | None:
        del where, include
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def find_first(
        self,
        skip: int | None = None,
        where: Filters | None = None,
        cursor: Filters | None = None,
        include: Filters | None = None,
        order: OrderBy = None,
        distinct: Sequence[str] | None = None,
    ) -> Row | None:
        del skip, where, cursor, include, order, distinct
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def create(self, data: Filters, include: Filters | None = None) -> Row:
        del data, include
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def create_many(
        self, data: Sequence[Filters], *, skip_duplicates: bool | None = None
    ) -> int:
        del data, skip_duplicates
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def upsert(
        self, where: Filters, data: Filters, include: Filters | None = None
    ) -> Row:
        del where, data, include
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def update(
        self, data: Filters, where: Filters, include: Filters | None = None
    ) -> Row | None:
        del data, where, include
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def update_many(self, data: Filters, where: Filters) -> int:
        del data, where
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def delete(self, where: Filters, include: Filters | None = None) -> Row | None:
        del where, include
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def delete_many(self, where: Filters | None = None) -> int:
        del where
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def count(
        self,
        select: None = None,
        take: int | None = None,
        skip: int | None = None,
        where: Filters | None = None,
        cursor: Filters | None = None,
    ) -> int:
        del select, take, skip, where, cursor
        raise NotImplementedError(UNIMPLEMENTED)

    @override
    async def group_by(
        self,
        by: Sequence[str],
        *,
        where: Filters | None = None,
        take: int | None = None,
        skip: int | None = None,
        order: OrderBy = None,
        having: Filters | None = None,
        count: bool | Filters | None = None,
        sum: bool | Filters | None = None,
        avg: bool | Filters | None = None,
        min: bool | Filters | None = None,
        max: bool | Filters | None = None,
    ) -> Sequence[Filters]:
        del by, where, take, skip, order, having, count, sum, avg, min, max
        raise NotImplementedError(UNIMPLEMENTED)


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


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    request = INPUT.validate_python(input_value)
    rows: list[Row] = []
    stored = request["stored"]
    if stored is not None:
        rows.append(Row(request["deployment_id"], stored))
    group_weights: dict[str, float] = {request["deployment_id"]: 1.0}
    weights: dict[str, dict[str, float]] = {request["model_group"]: group_weights}
    settings: dict[str, dict[str, dict[str, float]]] = {"weights": weights}
    try:
        asyncio.run(
            validate_router_settings_weights(
                settings,
                team_id=request["team_id"],
                prisma_client=Client(Table(rows)),
                llm_router=Router(request["configured"]),
            )
        )
    except StarletteHTTPException as raised:
        rejected: dict[str, JsonValue] = {"status": raised.status_code, "detail": raised.detail}
        return rejected
    accepted: dict[str, JsonValue] = {"status": None, "detail": None}
    return accepted
