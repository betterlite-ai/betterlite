import asyncio
from dataclasses import dataclass
from typing import TypedDict

import litellm
from litellm.proxy import proxy_server
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter


class CallerRecord(TypedDict):
    user_id: str | None
    team_id: str | None
    admin_view: bool


class UserRecord(TypedDict):
    user_id: str
    teams: list[str]


class Input(TypedDict):
    function: str
    model_info: dict[str, JsonValue]
    allowed_team_ids: list[str] | None
    team_models: list[str]
    access_groups: dict[str, list[str]]
    caller: CallerRecord | None
    user: UserRecord | None


@dataclass
class UserRow:
    user_id: str
    teams: list[str]


class UserTable:
    def __init__(self, rows: dict[str, UserRow]) -> None:
        self.rows: dict[str, UserRow] = rows

    async def find_unique(self, where: dict[str, str]) -> UserRow | None:
        await asyncio.sleep(0)
        return self.rows.get(where["user_id"])


class FakeDatabase(PrismaWrapper):
    def __init__(self, usertable: UserTable) -> None:
        self.litellm_usertable = usertable


class FakePrismaClient(PrismaClient):
    def __init__(self, database: FakeDatabase) -> None:
        self.db = database


def build_prisma(input_value: Input) -> FakePrismaClient:
    user = input_value["user"]
    users: dict[str, UserRow] = {}
    if user is not None:
        users[user["user_id"]] = UserRow(user_id=user["user_id"], teams=[*user["teams"]])
    return FakePrismaClient(FakeDatabase(UserTable(users)))


def build_caller(input_value: Input) -> UserAPIKeyAuth | None:
    record = input_value["caller"]
    if record is None:
        return None
    role = LitellmUserRoles.PROXY_ADMIN if record["admin_view"] else None
    return UserAPIKeyAuth(
        user_id=record["user_id"],
        team_id=record["team_id"],
        user_role=role,
    )


def byok_answer(input_value: Input) -> JsonValue:
    allowed = input_value["allowed_team_ids"]
    outside = proxy_server._byok_row_outside_caller_teams(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14650-L14661
        model_info_dict=input_value["model_info"],
        allowed_team_ids=None if allowed is None else set(allowed),
    )
    payload: dict[str, JsonValue] = {"outside": outside}
    return payload


def resolve_answer(input_value: Input) -> JsonValue:
    resolved = proxy_server._team_models_resolve_to_names(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L15068-L15076
        team_models=input_value["team_models"],
        access_groups=input_value["access_groups"],
    )
    names: list[JsonValue] = [name for name in resolved]
    payload: dict[str, JsonValue] = {"names": names}
    return payload


async def caller_scope_answer(input_value: Input) -> JsonValue:
    caller = build_caller(input_value)
    prisma: PrismaClient | None = None
    if caller is not None:
        prisma = build_prisma(input_value)
    scope = await proxy_server._get_caller_byok_team_scope(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14613-L14647
        user_api_key_dict=caller,
        prisma_client=prisma,
    )
    ids: list[JsonValue] | None = None
    if scope is not None:
        ids = [team for team in sorted(scope)]
    payload: dict[str, JsonValue] = {"allowed_team_ids": ids}
    return payload


async def answer(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "byok_row_outside_caller_teams":
        return byok_answer(input_value)
    if function == "team_models_resolve_to_names":
        return resolve_answer(input_value)
    if function == "caller_byok_team_scope":
        return await caller_scope_answer(input_value)
    raise ValueError(f"unsupported model_team_scope function {function}")


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    litellm.suppress_debug_info = True
    return asyncio.run(answer(TypeAdapter(Input).validate_python(input_value)))
