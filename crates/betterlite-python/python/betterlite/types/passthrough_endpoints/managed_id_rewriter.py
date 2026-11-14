from collections.abc import Mapping, Sequence
from datetime import datetime
from typing import Literal, Protocol, TypeAlias, TypeVar

from pydantic import JsonValue

SortOrder: TypeAlias = Literal["asc", "desc"]
ResourceKind: TypeAlias = Literal["files", "batches"]

PrismaWhereValue: TypeAlias = (
    str | int | bool | datetime | None
    | Mapping[str, "PrismaWhereValue"]
    | Sequence["PrismaWhereValue"]
)
PrismaWhere: TypeAlias = Mapping[str, PrismaWhereValue]
PrismaOrder: TypeAlias = Mapping[str, SortOrder]
ManagedRowData: TypeAlias = Mapping[str, str | None]


class ManagedResourceRow(Protocol):
    created_by: str | None
    team_id: str | None
    created_at: datetime | None
    file_object: JsonValue


class ManagedFileRow(ManagedResourceRow, Protocol):
    unified_file_id: str


class ManagedObjectRow(ManagedResourceRow, Protocol):
    unified_object_id: str


RowT = TypeVar("RowT", bound=ManagedResourceRow)


class ManagedTable(Protocol[RowT]):
    async def find_first(self, *, where: PrismaWhere) -> RowT | None: ...

    async def find_many(
        self,
        *,
        where: PrismaWhere,
        order: PrismaOrder | Sequence[PrismaOrder] | None = None,
        take: int | None = None,
    ) -> list[RowT]: ...


class ManagedFileTable(ManagedTable[ManagedFileRow], Protocol): ...


class ManagedObjectTable(ManagedTable[ManagedObjectRow], Protocol):
    async def update(
        self, *, where: PrismaWhere, data: ManagedRowData,
    ) -> ManagedObjectRow | None: ...

    async def upsert(
        self, *, where: PrismaWhere, data: Mapping[str, ManagedRowData],
    ) -> ManagedObjectRow: ...
