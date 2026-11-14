import asyncio
from collections.abc import Mapping, Sequence
from datetime import timedelta
from types import TracebackType
from typing import TypedDict

from litellm.litellm_core_utils.litellm_logging import is_valid_sha256_hash
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.spend_tracking.key_metadata_recovery import (
    KeyMetadataDict,
    attach_user_details,
    fill_missing_api_key_aliases,
    recover_cli_session_key_metadata,
    recover_double_hashed_key_metadata,
    recover_key_owner_from_daily_spend,
)
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter

ROW_VALUE: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class UserRow(TypedDict):
    user_id: str
    user_email: str | None
    teams: list[str]


class TokenRow(TypedDict):
    digest: str
    key_alias: str | None
    team_id: str | None
    user_id: str | None


class DailyRow(TypedDict):
    api_key: str
    first_owner: str | None
    last_owner: str | None


class Fields(TypedDict):
    api_key: str
    alias: str
    team_id: str
    user_email: str


class Input(TypedDict):
    digest_values: list[str]
    cli_keys: list[str]
    double_hashed_keys: list[str]
    daily_keys: list[str]
    users: list[UserRow]
    active_token_rows: list[TokenRow]
    deleted_token_rows: list[TokenRow]
    daily_owner_rows: list[DailyRow]
    attach_metadata: dict[str, KeyMetadataDict]
    export_rows: list[dict[str, JsonValue]]
    export_fields: Fields


class FakeTable:
    def __init__(self, users: list[LiteLLM_UserTable]) -> None:
        self.users = users

    async def find_many(
        self, *, where: Mapping[str, Mapping[str, Sequence[str]]]
    ) -> Sequence[LiteLLM_UserTable]:
        await asyncio.sleep(0)
        wanted = where["user_id"]["in"]
        return [user for user in self.users if user.user_id in wanted]


class FakeTransaction:
    def __init__(self, rows: Sequence[Mapping[str, JsonValue]]) -> None:
        self.rows = rows

    async def __aenter__(self) -> "FakeTransaction":
        await asyncio.sleep(0)
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        del exc_type, exc_value, traceback
        await asyncio.sleep(0)

    async def execute_raw(self, sql: str, *params: JsonValue) -> int:
        del sql, params
        await asyncio.sleep(0)
        return 0

    async def query_raw(self, sql: str, *params: JsonValue) -> Sequence[Mapping[str, JsonValue]]:
        del sql, params
        await asyncio.sleep(0)
        return self.rows


class FakeWrapper(PrismaWrapper):
    def __init__(
        self,
        table: FakeTable,
        active: Sequence[Mapping[str, JsonValue]],
        deleted: Sequence[Mapping[str, JsonValue]],
        daily: Sequence[Mapping[str, JsonValue]],
    ) -> None:
        self.litellm_usertable = table
        self.active = active
        self.deleted = deleted
        self.daily = daily

    def tx(self, *, timeout: timedelta) -> FakeTransaction:
        del timeout
        return FakeTransaction(self.daily)

    async def query_raw(self, sql: str, *params: JsonValue) -> Sequence[Mapping[str, JsonValue]]:
        del params
        await asyncio.sleep(0)
        if '"LiteLLM_DeletedVerificationToken"' in sql:
            return self.deleted
        return self.active


class FakePrismaClient(PrismaClient):
    def __init__(
        self,
        users: list[LiteLLM_UserTable],
        active: Sequence[Mapping[str, JsonValue]],
        deleted: Sequence[Mapping[str, JsonValue]],
        daily: Sequence[Mapping[str, JsonValue]],
    ) -> None:
        self.db = FakeWrapper(FakeTable(users), active, deleted, daily)


def dump_metadata(meta: KeyMetadataDict) -> dict[str, JsonValue]:
    dumped: dict[str, JsonValue] = {}
    key_alias = meta.get("key_alias")
    if key_alias is not None:
        dumped["key_alias"] = key_alias
    team_id = meta.get("team_id")
    if team_id is not None:
        dumped["team_id"] = team_id
    user_id = meta.get("user_id")
    if user_id is not None:
        dumped["user_id"] = user_id
    user_email = meta.get("user_email")
    if user_email is not None:
        dumped["user_email"] = user_email
    key_exists = meta.get("key_exists")
    if key_exists is not None:
        dumped["key_exists"] = key_exists
    return dumped


def dump_mapping(recovered: Mapping[str, KeyMetadataDict]) -> dict[str, JsonValue]:
    dumped: dict[str, JsonValue] = {}
    for key, meta in recovered.items():
        dumped[key] = dump_metadata(meta)
    return dumped


def fake_client(input_value: Input) -> FakePrismaClient:
    users = [LiteLLM_UserTable.model_validate(user) for user in input_value["users"]]
    return FakePrismaClient(
        users,
        input_value["active_token_rows"],
        input_value["deleted_token_rows"],
        input_value["daily_owner_rows"],
    )


async def collect(client: FakePrismaClient, input_value: Input) -> dict[str, JsonValue]:
    digest: list[JsonValue] = [
        is_valid_sha256_hash(value) for value in input_value["digest_values"]
    ]
    cli = await recover_cli_session_key_metadata(client, frozenset(input_value["cli_keys"]))
    double_hashed = await recover_double_hashed_key_metadata(
        client, frozenset(input_value["double_hashed_keys"])
    )
    daily_owners = await recover_key_owner_from_daily_spend(
        client, frozenset(input_value["daily_keys"])
    )
    attached = await attach_user_details(client, input_value["attach_metadata"])
    fields = input_value["export_fields"]
    export = await fill_missing_api_key_aliases(
        client,
        input_value["export_rows"],
        api_key_field=fields["api_key"],
        alias_field=fields["alias"],
        team_id_field=fields["team_id"],
        user_email_field=fields["user_email"],
    )
    owners: dict[str, JsonValue] = {}
    for key, owner in daily_owners.items():
        owners[key] = owner
    rows: list[JsonValue] = [ROW_VALUE.validate_python(row) for row in export]
    return {
        "digest": digest,
        "cli": dump_mapping(cli),
        "double_hashed": dump_mapping(double_hashed),
        "daily_owners": owners,
        "attached": dump_mapping(attached),
        "export": rows,
    }


def run(input_value: Input) -> JsonValue:
    client = fake_client(input_value)
    return asyncio.run(collect(client, input_value))
