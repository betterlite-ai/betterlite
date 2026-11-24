import asyncio
from dataclasses import dataclass
from typing import Final, Literal, NotRequired, TypedDict, cast

import litellm
from litellm.proxy import proxy_server
from litellm.proxy._types import LiteLLM_UserTable, LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.utils import PrismaClient
from litellm.router import Router
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import JsonValue, TypeAdapter


class UserRecord(TypedDict):
    user_id: str
    teams: list[str]
    models: list[str]


class TeamRecord(TypedDict):
    team_id: str
    models: list[str]
    access_group_ids: NotRequired[list[str] | None]


class AccessGroupRecord(TypedDict):
    access_group_id: str
    access_model_names: list[str]


class CallerRecord(TypedDict):
    user_id: str | None
    admin_view: bool
    models: list[str]


class Input(TypedDict):
    function: str
    model_list: list[dict[str, JsonValue]]
    model_group_alias: dict[str, str]
    all_models: list[dict[str, JsonValue]]
    grant: list[str]
    key_models: list[str]
    user: UserRecord | None
    creators: dict[str, str | None]
    teams: list[TeamRecord]
    user_teams: Literal["*"] | list[str] | None
    access_groups: list[AccessGroupRecord]
    caller: CallerRecord | None


INFO: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
ROWS: Final[TypeAdapter[list[dict[str, JsonValue]]]] = TypeAdapter(list[dict[str, JsonValue]])


@dataclass
class ProxyModelRow:
    created_by: str | None


@dataclass
class UserRow:
    user_id: str
    teams: list[str]
    models: list[str]


@dataclass
class TeamRow:
    team_id: str
    models: list[str]
    access_group_ids: list[str] | None

    def model_dump(self) -> dict[str, JsonValue]:
        models: list[JsonValue] = [*self.models]
        ids: list[JsonValue] = (
            [] if self.access_group_ids is None else [*self.access_group_ids]
        )
        return {"team_id": self.team_id, "models": models, "access_group_ids": ids}


@dataclass
class AccessGroupRow:
    access_group_id: str
    access_model_names: list[str]


class ProxyModelTable:
    def __init__(self, rows: dict[str, ProxyModelRow]) -> None:
        self.rows: dict[str, ProxyModelRow] = rows

    async def find_unique(self, where: dict[str, str]) -> ProxyModelRow | None:
        await asyncio.sleep(0)
        return self.rows.get(where["model_id"])


class UserTable:
    def __init__(self, rows: dict[str, UserRow]) -> None:
        self.rows: dict[str, UserRow] = rows

    async def find_unique(self, where: dict[str, str]) -> UserRow | None:
        await asyncio.sleep(0)
        return self.rows.get(where["user_id"])


class TeamTable:
    def __init__(self, rows: list[TeamRow]) -> None:
        self.rows: list[TeamRow] = rows

    async def find_many(
        self, where: dict[str, dict[str, list[str]]] | None = None
    ) -> list[TeamRow]:
        await asyncio.sleep(0)
        if where is None:
            return list(self.rows)
        wanted = set(where["team_id"]["in"])
        return [row for row in self.rows if row.team_id in wanted]


class AccessGroupTable:
    def __init__(self, rows: list[AccessGroupRow]) -> None:
        self.rows: list[AccessGroupRow] = rows

    async def find_many(self, where: dict[str, dict[str, list[str]]]) -> list[AccessGroupRow]:
        await asyncio.sleep(0)
        wanted = set(where["access_group_id"]["in"])
        return [row for row in self.rows if row.access_group_id in wanted]


@dataclass
class FakeDatabase:
    litellm_proxymodeltable: ProxyModelTable
    litellm_usertable: UserTable
    litellm_teamtable: TeamTable
    litellm_accessgrouptable: AccessGroupTable


@dataclass
class FakePrisma:
    db: FakeDatabase


def build_router(input_value: Input) -> Router:
    aliases: dict[str, str | RouterModelGroupAliasItem] = {}
    aliases.update(input_value["model_group_alias"])
    return Router(model_list=input_value["model_list"], model_group_alias=aliases)


def build_prisma(input_value: Input) -> FakePrisma:
    user = input_value["user"]
    users: dict[str, UserRow] = {}
    if user is not None:
        users[user["user_id"]] = UserRow(
            user_id=user["user_id"], teams=list(user["teams"]), models=list(user["models"])
        )
    teams = [
        TeamRow(
            team_id=record["team_id"],
            models=list(record["models"]),
            access_group_ids=record.get("access_group_ids"),
        )
        for record in input_value["teams"]
    ]
    groups = [
        AccessGroupRow(
            access_group_id=record["access_group_id"],
            access_model_names=list(record["access_model_names"]),
        )
        for record in input_value["access_groups"]
    ]
    return FakePrisma(
        db=FakeDatabase(
            litellm_proxymodeltable=ProxyModelTable(
                {
                    model_id: ProxyModelRow(created_by=created_by)
                    for model_id, created_by in input_value["creators"].items()
                }
            ),
            litellm_usertable=UserTable(users),
            litellm_teamtable=TeamTable(teams),
            litellm_accessgrouptable=AccessGroupTable(groups),
        )
    )


def build_caller(input_value: Input) -> UserAPIKeyAuth:
    record = input_value["caller"]
    if record is None:
        raise ValueError("this operation needs a caller record")
    role = LitellmUserRoles.PROXY_ADMIN if record["admin_view"] else None
    return UserAPIKeyAuth(
        user_id=record["user_id"],
        user_role=role,
        models=record["models"],
    )


def row_ids(value: JsonValue) -> list[JsonValue]:
    return [
        INFO.validate_python(row["model_info"]).get("id") if "model_info" in row else None
        for row in ROWS.validate_python(value)
    ]


def grant_answer(input_value: Input) -> JsonValue:
    found = proxy_server._resolve_model_grant_to_deployment_ids(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14407-L14430
        models=input_value["grant"], llm_router=build_router(input_value)
    )
    ids: list[JsonValue] = [*found]
    return {"ids": ids}


def direct_answer(input_value: Input) -> JsonValue:
    user = input_value["user"]
    if user is None:
        raise ValueError("the direct_access function needs a user row")
    user_models: list[str] = [*user["models"]]
    user_object = LiteLLM_UserTable(
        user_id=user["user_id"], teams=list(user["teams"]), models=user_models
    )
    found = proxy_server.get_direct_access_models(
        user_db_object=user_object,
        llm_router=build_router(input_value),
        key_models=input_value["key_models"],
    )
    ids: list[JsonValue] = [*found]
    return {"ids": ids}


async def team_answer(input_value: Input) -> JsonValue:
    user_teams = input_value["user_teams"]
    if user_teams is None:
        raise ValueError("the team_models function needs user_teams")
    found = await proxy_server.get_all_team_models(
        user_teams=user_teams,
        prisma_client=cast(PrismaClient, build_prisma(input_value)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14359-L14404
        llm_router=build_router(input_value),
    )
    teams: dict[str, JsonValue] = {}
    for model_id in sorted(found):
        names: list[JsonValue] = [*sorted(found[model_id])]
        teams[model_id] = names
    return {"team_models": teams}


async def non_admin_answer(input_value: Input) -> JsonValue:
    models = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14208-L14251
        JsonValue,
        await proxy_server.non_admin_all_models(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14208-L14251
            all_models=input_value["all_models"],
            llm_router=build_router(input_value),
            user_api_key_dict=build_caller(input_value),
            prisma_client=cast(PrismaClient, build_prisma(input_value)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14208-L14251
        ),
    )
    return {"ids": row_ids(models)}


def filter_answer(input_value: Input) -> JsonValue:
    models = input_value["all_models"]
    kept = cast(JsonValue, proxy_server._filter_models_to_user_accessible(models))  # noqa: SLF001  # pyright: ignore[reportAny, reportPrivateUsage, reportUnknownArgumentType, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14456-L14463
    return {"ids": row_ids(kept)}


async def answer(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "grant_resolution":
        return grant_answer(input_value)
    if function == "direct_access":
        return direct_answer(input_value)
    if function == "team_models":
        return await team_answer(input_value)
    if function == "non_admin":
        return await non_admin_answer(input_value)
    if function == "filter":
        return filter_answer(input_value)
    raise ValueError(f"unsupported model_info_scope function {function}")


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    litellm.suppress_debug_info = True
    return asyncio.run(answer(TypeAdapter(Input).validate_python(input_value)))
