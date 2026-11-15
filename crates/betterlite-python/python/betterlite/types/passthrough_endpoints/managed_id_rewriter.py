from collections.abc import Mapping, Sequence
from datetime import datetime
from typing import Literal, Protocol, TypeAlias, TypedDict, TypeVar, runtime_checkable

from pydantic import JsonValue

from betterlite.types.litellm_params_state import ForeignObject

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


@runtime_checkable
class ManagedFileIdReader(Protocol):
    async def get_unified_file_id(
        self, file_id: str, litellm_parent_otel_span: ForeignObject | None = None,
    ) -> ManagedFileRow | None: ...


@runtime_checkable
class ManagedFileIdWriter(Protocol):
    async def store_unified_file_id(
        self,
        file_id: str,
        file_object: ForeignObject | None,
        litellm_parent_otel_span: ForeignObject | None,
        model_mappings: dict[str, str],
        user_api_key_dict: ForeignObject,
    ) -> None: ...


class ManagedListResponse(TypedDict):
    object: Literal["list"]
    data: list[dict[str, JsonValue]]
    first_id: str | None
    last_id: str | None
    has_more: bool
