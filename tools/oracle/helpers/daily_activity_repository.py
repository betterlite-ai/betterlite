import asyncio
import typing
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

from litellm.repositories.daily_activity_repository import DailyActivityRepository
from litellm.types.repositories.daily_activity import (
    DailyActivityScope,
    DailyActivityTable,
    ExportType,
    KeyMetadataRow,
    SpendLogsWindow,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
BATCH_SIZE: Final = 1000
DELETED_AT: Final = datetime(2026, 1, 1, tzinfo=UTC)


@dataclass(slots=True)
class TokenRecord:
    token: str
    key_alias: str | None
    team_id: str | None
    user_id: str | None
    metadata: dict[str, JsonValue] | None


@dataclass(slots=True)
class DeletedTokenRecord(TokenRecord):
    deleted_at: datetime


class FakeTable:
    def __init__(self, rows: Sequence[TokenRecord]) -> None:
        self.rows = rows

    async def find_many(
        self, _where: Mapping[str, object] | None = None, **_extra: object
    ) -> Sequence[TokenRecord]:
        return list(
            self.rows
        )


class FakeDatabase:
    def __init__(
        self,
        active: Sequence[TokenRecord],
        deleted: Sequence[DeletedTokenRecord],
        batches: Sequence[Sequence[Mapping[str, object]]],
    ) -> None:
        self.litellm_verificationtoken = FakeTable(active)
        self.litellm_deletedverificationtoken = FakeTable(deleted)
        self.batches = list(batches)
        self.queries: list[tuple[str, tuple[object, ...]]] = []

    async def query_raw(
        self, query: str, *values: object
    ) -> Sequence[Mapping[str, object]] | None:
        self.queries.append((query, values))
        if not self.batches:
            return None
        return self.batches.pop(0)

    @property
    def db(self) -> "FakeDatabase":
        return self


class PassthroughReads:
    async def recover_key_metadata(
        self,
        resolved: Mapping[str, KeyMetadataRow],
        _api_keys: frozenset[str],
        _window: SpendLogsWindow | None,
    ) -> Mapping[str, KeyMetadataRow]:
        return dict(
            resolved
        )


class FakeRepository(DailyActivityRepository):
    @typing.no_type_check
    def __init__(self, database: FakeDatabase) -> None:
        self._prisma_client = database
        self._proxy_reads = PassthroughReads()


class TagsCase(TypedDict):
    function: Literal["metadata_tags"]
    value: JsonValue


class RowCase(TypedDict):
    function: Literal["metadata_row"]
    token: str
    key_alias: str | None
    team_id: str | None
    user_id: str | None
    metadata: dict[str, JsonValue] | None
    key_exists: bool


class CursorCase(TypedDict):
    function: Literal["next_cursor"]
    export_type: Literal["daily", "daily_with_keys", "daily_with_models", "daily_with_users"]
    last: dict[str, JsonValue]


Call = TagsCase | RowCase | CursorCase


class Input(TypedDict):
    cases: dict[str, Call]


async def resolved_row(record: TokenRecord, *, key_exists: bool) -> KeyMetadataRow:
    active: list[TokenRecord] = [record] if key_exists else []
    archived: list[DeletedTokenRecord] = []
    if not key_exists:
        archived.append(
            DeletedTokenRecord(
                record.token,
                record.key_alias,
                record.team_id,
                record.user_id,
                record.metadata,
                DELETED_AT,
            )
        )
    database = FakeDatabase(active=active, deleted=archived, batches=[])
    repository = FakeRepository(database)
    resolved = await repository.key_metadata(frozenset({record.token}), None)
    return resolved[record.token]


async def tags_answer(case: TagsCase) -> JsonValue:
    record = TokenRecord("key", None, None, None, {"tags": case["value"]})
    row = await resolved_row(record, key_exists=True)
    return JSON.validate_python({"tags": list(row.tags)})


async def row_answer(case: RowCase) -> JsonValue:
    record = TokenRecord(
        token=case["token"],
        key_alias=case["key_alias"],
        team_id=case["team_id"],
        user_id=case["user_id"],
        metadata=case["metadata"],
    )
    row = await resolved_row(record, key_exists=case["key_exists"])
    return JSON.validate_python(
        {
            "row": {
                "api_key": row.api_key,
                "key_alias": row.key_alias,
                "team_id": row.team_id,
                "user_id": row.user_id,
                "user_email": row.user_email,
                "key_exists": row.key_exists,
                "tags": list(row.tags),
            }
        }
    )


async def cursor_answer(case: CursorCase) -> JsonValue:
    batch: list[dict[str, JsonValue]] = [dict(case["last"]) for _ in range(BATCH_SIZE)]
    database = FakeDatabase(active=[], deleted=[], batches=[batch, []])
    repository = FakeRepository(database)
    scope = DailyActivityScope(
        table=DailyActivityTable.USER,
        entity_id_field="user_id",
        entity_ids=None,
        exclude_entity_ids=(),
        api_keys=None,
        start_date="2026-08-01",
        end_date="2026-08-05",
        model=None,
        timezone_offset_minutes=None,
        include_current_utc_day=False,
    )
    rows = repository.export_rows(scope, export_type=ExportType(case["export_type"]))
    for _ in range(BATCH_SIZE):
        await rows.__anext__()
    remainder = await anext(rows, None)
    if remainder is not None:
        raise ValueError("the export produced more than one batch")
    cursor_query = database.queries[-1]
    return JSON.validate_python(
        {
            "cursor": {
                "date": cursor_query[1][-4],
                "entity_id": cursor_query[1][-3],
                "group_key": cursor_query[1][-2],
            }
        }
    )


async def run_async(input_value: Input) -> JsonValue:
    answers: dict[str, JsonValue] = {}
    for name, case in input_value["cases"].items():
        if case["function"] == "metadata_tags":
            answers[name] = await tags_answer(case)
        elif case["function"] == "metadata_row":
            answers[name] = await row_answer(case)
        else:
            answers[name] = await cursor_answer(case)
    return answers


def run(input_value: Input) -> JsonValue:
    return JSON.validate_python(asyncio.run(run_async(input_value)))
