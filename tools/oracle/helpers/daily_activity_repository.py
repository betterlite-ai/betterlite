import asyncio
import sys
import types
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime, timedelta
from types import TracebackType
from typing import Final, Literal, Self, TypedDict

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints.common_daily_activity import daily_activity_repository
from litellm.proxy.spend_tracking.spend_event import ObjectMapping
from litellm.proxy.utils import PrismaClient
from litellm.types.repositories.daily_activity import (
    DailyActivityScope,
    DailyActivityTable,
    ExportType,
    KeyMetadataRow,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
BATCH_SIZE: Final = 1000
DELETED_AT: Final = datetime(2026, 1, 1, tzinfo=UTC)


class PrismaErrorsModule(types.ModuleType):
    PrismaError = RuntimeError


class PrismaModule(types.ModuleType):
    errors = PrismaErrorsModule("prisma.errors")


# the recovery path imports prisma.errors lazily; the oracle environment has no prisma package
sys.modules.setdefault("prisma", PrismaModule("prisma"))
sys.modules.setdefault("prisma.errors", PrismaModule.errors)


class TokenFields(TypedDict):
    token: str
    key_alias: str | None
    team_id: str | None
    user_id: str | None
    metadata: dict[str, JsonValue] | None


class TokenRecord:
    def __init__(self, fields: TokenFields) -> None:
        self.token = fields["token"]
        self.key_alias = fields["key_alias"]
        self.team_id = fields["team_id"]
        self.user_id = fields["user_id"]
        self.metadata: ObjectMapping | None = fields["metadata"]


class DeletedTokenRecord(TokenRecord):
    def __init__(self, fields: TokenFields, deleted_at: datetime) -> None:
        super().__init__(fields)
        self.deleted_at = deleted_at


class UserRecord:
    def __init__(self, user_id: str, user_email: str | None, teams: list[str]) -> None:
        self.user_id = user_id
        self.user_email = user_email
        self.teams = teams


class TokenTable:
    def __init__(self, rows: Sequence[TokenRecord]) -> None:
        self.rows = tuple(rows)

    async def find_many(
        self, *, where: Mapping[str, Mapping[str, Sequence[str]]]
    ) -> Sequence[TokenRecord]:
        wanted = where["token"]["in"]
        return [row for row in self.rows if row.token in wanted]


class UserTable:
    def __init__(self, rows: Sequence[UserRecord]) -> None:
        self.rows = tuple(rows)

    async def find_many(
        self, *, where: Mapping[str, Mapping[str, Sequence[str]]]
    ) -> Sequence[UserRecord]:
        wanted = where["user_id"]["in"]
        return [row for row in self.rows if row.user_id in wanted]


class FakeTransaction:
    def __init__(self) -> None:
        self.statements: list[str] = []
        self.queries: list[tuple[str, tuple[JsonValue, ...]]] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        del exc_type, exc_value, traceback

    async def execute_raw(self, sql: str) -> int:
        self.statements.append(sql)
        return 0

    async def query_raw(
        self, sql: str, *params: JsonValue
    ) -> Sequence[Mapping[str, JsonValue]]:
        self.queries.append((sql, tuple(JSON.validate_python(param) for param in params)))
        return ()


class FakeWrapper(PrismaWrapper):
    def __init__(
        self,
        active: Sequence[TokenRecord],
        deleted: Sequence[TokenRecord],
        batches: Sequence[Sequence[Mapping[str, JsonValue]]],
    ) -> None:
        self.batches = [list(batch) for batch in batches]
        self.queries: list[tuple[str, tuple[JsonValue, ...]]] = []
        self.timeouts: list[timedelta] = []
        self.transaction = FakeTransaction()
        self.litellm_verificationtoken = TokenTable(active)
        self.litellm_deletedverificationtoken = TokenTable(deleted)
        self.litellm_usertable = UserTable(())

    def tx(self, *, timeout: timedelta) -> FakeTransaction:
        self.timeouts.append(timeout)
        return self.transaction

    async def query_raw(
        self, sql: str, *params: JsonValue
    ) -> Sequence[Mapping[str, JsonValue]] | None:
        self.queries.append((sql, tuple(JSON.validate_python(param) for param in params)))
        if not self.batches:
            return None
        return self.batches.pop(0)


class FakePrismaClient(PrismaClient):
    def __init__(self, wrapper: FakeWrapper) -> None:
        self.db = wrapper


class TagsCase(TypedDict):
    function: Literal["metadata_tags"]
    value: JsonValue


class RowCase(TokenFields):
    function: Literal["metadata_row"]
    key_exists: bool


class CursorCase(TypedDict):
    function: Literal["next_cursor"]
    export_type: Literal["daily", "daily_with_keys", "daily_with_models", "daily_with_users"]
    last: dict[str, JsonValue]


Call = TagsCase | RowCase | CursorCase


class Input(TypedDict):
    cases: dict[str, Call]


async def token_metadata(fields: TokenFields, *, key_exists: bool) -> Mapping[str, KeyMetadataRow]:
    record = TokenRecord(fields)
    active: list[TokenRecord] = [record] if key_exists else []
    deleted: list[DeletedTokenRecord] = []
    if not key_exists:
        deleted.append(DeletedTokenRecord(fields, DELETED_AT))
    repository = daily_activity_repository(FakePrismaClient(FakeWrapper(active, deleted, [])))
    return await repository.key_metadata(frozenset({fields["token"]}), None)


async def tags_answer(case: TagsCase) -> JsonValue:
    fields: TokenFields = {
        "token": "key",
        "key_alias": None,
        "team_id": None,
        "user_id": None,
        "metadata": {"tags": case["value"]},
    }
    resolved = await token_metadata(fields, key_exists=True)
    row = resolved["key"]
    payload: ObjectMapping = {"tags": list(row.tags)}
    return JSON.validate_python(payload)


async def row_answer(case: RowCase) -> JsonValue:
    resolved = await token_metadata(case, key_exists=case["key_exists"])
    row = resolved[case["token"]]
    payload: ObjectMapping = {
        "row": {
            "api_key": row.api_key,
            "key_alias": row.key_alias,
            "team_id": row.team_id,
            "user_id": row.user_id,
            "user_email": row.user_email,
            "key_exists": row.key_exists,
            "tags": list(row.tags),
        },
    }
    return JSON.validate_python(payload)


async def cursor_answer(case: CursorCase) -> JsonValue:
    batch: list[dict[str, JsonValue]] = [case["last"] for _ in range(BATCH_SIZE)]
    wrapper = FakeWrapper([], [], [batch, []])
    repository = daily_activity_repository(FakePrismaClient(wrapper))
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
    if await anext(rows, None) is not None:
        raise ValueError("the export produced more than one batch")
    binds = wrapper.queries[-1][1]
    payload: ObjectMapping = {
        "cursor": {
            "date": binds[-4],
            "entity_id": binds[-3],
            "group_key": binds[-2],
        },
    }
    return JSON.validate_python(payload)


async def answers(cases: Mapping[str, Call]) -> dict[str, JsonValue]:
    resolved: dict[str, JsonValue] = {}
    for name, case in cases.items():
        if case["function"] == "metadata_tags":
            resolved[name] = await tags_answer(case)
        elif case["function"] == "metadata_row":
            resolved[name] = await row_answer(case)
        else:
            resolved[name] = await cursor_answer(case)
    return resolved


def run(input_value: Input) -> JsonValue:
    resolved = asyncio.run(answers(input_value["cases"]))
    return JSON.validate_python(resolved)
