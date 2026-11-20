import asyncio
from collections.abc import Sequence
from typing import Final, Literal, NotRequired, TypedDict, cast

from litellm.models.team import LiteLLM_TeamTable, LiteLLM_TeamTableCachedObj
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.management_endpoints import model_management_endpoints
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from litellm.types.router import Deployment, LiteLLM_Params
from pydantic import JsonValue, TypeAdapter


class Tag(TypedDict):
    function: str


class AliasRowInput(TypedDict):
    id: int
    team_id: NotRequired[str | None]
    aliases: NotRequired[dict[str, str] | None]


class DeploymentRowInput(TypedDict):
    model_name: str
    model_info: JsonValue


class AliasCall(TypedDict):
    function: Literal["delete_team_model_alias"]
    public_model_name: str
    aliases: list[AliasRowInput]


class TeamDeploymentsCall(TypedDict):
    function: Literal["get_team_deployments"]
    team_id: str
    deployments: list[DeploymentRowInput]


class TeamPublicNamesCall(TypedDict):
    function: Literal["get_team_public_model_names"]
    team_id: str
    deployments: list[DeploymentRowInput]


class RemoveCall(TypedDict):
    function: Literal["remove_unbacked_team_models"]
    model_name: str
    team_id: NotRequired[str | None]
    team_public_model_name: NotRequired[str | None]
    aliases: list[AliasRowInput]
    deployments: list[DeploymentRowInput]
    served_names: NotRequired[list[str] | None]
    team_models: NotRequired[list[str] | None]


class Input(TypedDict):
    calls: list[JsonValue]


class AliasState(TypedDict):
    id: int
    aliases: dict[str, str] | None


class CacheState(TypedDict):
    team_id: str
    models: list[str]


JSON: Final = TypeAdapter(JsonValue)
TAG: Final = TypeAdapter(Tag)
ALIAS_MAP: Final = TypeAdapter(dict[str, str])
ALIAS_CALL: Final = TypeAdapter(AliasCall)
DEPLOYMENTS_CALL: Final = TypeAdapter(TeamDeploymentsCall)
PUBLIC_NAMES_CALL: Final = TypeAdapter(TeamPublicNamesCall)
REMOVE_CALL: Final = TypeAdapter(RemoveCall)


class FakeTeamRelation:
    team_id: str

    def __init__(self, team_id: str) -> None:
        self.team_id = team_id


class FakeAliasRow:
    def __init__(self, row: AliasRowInput) -> None:
        self.id = row["id"]
        self.model_aliases = row.get("aliases")
        team_id = row.get("team_id")
        self.team = None if team_id is None else FakeTeamRelation(team_id)


class FakeAliasTable:
    rows: list[FakeAliasRow]

    def __init__(self, rows: list[AliasRowInput]) -> None:
        self.rows = [FakeAliasRow(row) for row in rows]

    async def find_many(self, *, include: dict[str, bool]) -> list[FakeAliasRow]:
        del include
        return list(self.rows)

    async def update(self, *, where: dict[str, int], data: dict[str, str]) -> FakeAliasRow:
        found = next(row for row in self.rows if row.id == where["id"])
        found.model_aliases = ALIAS_MAP.validate_json(data["model_aliases"])
        return found


class FakeDeploymentRow:
    def __init__(self, row: DeploymentRowInput) -> None:
        self.model_name = row["model_name"]
        self.model_info = row["model_info"]


class FakeDeploymentTable:
    rows: list[FakeDeploymentRow]

    def __init__(self, rows: list[DeploymentRowInput]) -> None:
        self.rows = [FakeDeploymentRow(row) for row in rows]

    async def find_many(self, *, where: dict[str, dict[str, str]]) -> list[FakeDeploymentRow]:
        prefix = where["model_name"]["startswith"]
        return [row for row in self.rows if row.model_name.startswith(prefix)]


class FakeTeamTable:
    row: LiteLLM_TeamTable | None

    def __init__(self, team_id: str | None, models: list[str] | None) -> None:
        if team_id is None or models is None:
            self.row = None
        else:
            self.row = LiteLLM_TeamTable(team_id=team_id, models=list(models))

    async def find_unique(self, *, where: dict[str, str]) -> LiteLLM_TeamTable | None:
        del where
        return self.row

    async def update(
        self,
        *,
        where: dict[str, str],
        data: dict[str, list[str]],
        include: dict[str, bool],
    ) -> LiteLLM_TeamTable:
        del where, include
        if self.row is None:
            raise LookupError("the team row is absent")
        self.row.models = list(data["models"])
        return self.row


class FakeDatabase(PrismaWrapper):
    def __init__(
        self,
        aliases: FakeAliasTable,
        deployments: FakeDeploymentTable,
        teams: FakeTeamTable,
    ) -> None:
        self.litellm_modeltable = aliases
        self.litellm_proxymodeltable = deployments
        self.litellm_teamtable = teams


class FakePrismaClient(PrismaClient):
    def __init__(self, database: FakeDatabase) -> None:
        self.db = database


class FakeRouter(Router):
    def __init__(self, names: Sequence[str]) -> None:
        self.model_name_to_deployment_indices = {name: [0] for name in names}


def cache_summary(cache: UserApiKeyCache, team_id: str | None) -> CacheState | None:
    if team_id is None:
        return None
    cached = cache.get_cache(key=f"team_id:{team_id}", model_type=LiteLLM_TeamTableCachedObj)
    if cached is None:
        return None
    return CacheState(team_id=cached.team_id, models=cached.models)


def alias_answer(call: AliasCall) -> JsonValue:
    aliases = FakeAliasTable(call["aliases"])
    client = FakePrismaClient(
        FakeDatabase(aliases, FakeDeploymentTable([]), FakeTeamTable(None, None))
    )
    removed = asyncio.run(
        model_management_endpoints.delete_team_model_alias(
            public_model_name=call["public_model_name"],
            prisma_client=client,
        )
    )
    report = {
        "removed": [{"team_id": team_id, "alias": alias} for team_id, alias in removed],
        "aliases": [
            AliasState(id=row.id, aliases=row.model_aliases) for row in aliases.rows
        ],
    }
    return JSON.validate_python(report)


def deployments_answer(call: TeamDeploymentsCall) -> JsonValue:
    deployments = FakeDeploymentTable(call["deployments"])
    client = FakePrismaClient(
        FakeDatabase(FakeAliasTable([]), deployments, FakeTeamTable(None, None))
    )
    rows = asyncio.run(
        model_management_endpoints._get_team_deployments(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L1734-L1766
            team_id=call["team_id"],
            prisma_client=client,
        )
    )
    return JSON.validate_python({"deployments": [row.model_name for row in rows]})


def public_names_answer(call: TeamPublicNamesCall) -> JsonValue:
    deployments = FakeDeploymentTable(call["deployments"])
    client = FakePrismaClient(
        FakeDatabase(FakeAliasTable([]), deployments, FakeTeamTable(None, None))
    )
    names = asyncio.run(
        model_management_endpoints._get_team_public_model_names(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L1812-L1831
            team_id=call["team_id"],
            prisma_client=client,
        )
    )
    return JSON.validate_python({"names": sorted(names)})


def remove_answer(call: RemoveCall) -> JsonValue:
    aliases = FakeAliasTable(call["aliases"])
    deployments = FakeDeploymentTable(call["deployments"])
    teams = FakeTeamTable(call.get("team_id"), call.get("team_models"))
    client = FakePrismaClient(FakeDatabase(aliases, deployments, teams))
    cache = UserApiKeyCache()
    served = call.get("served_names")
    router = None if served is None else FakeRouter(served)
    model = Deployment(
        model_name=call["model_name"],
        litellm_params=LiteLLM_Params(model="openai/gpt-4o"),
        model_info={
            "team_id": call.get("team_id"),
            "team_public_model_name": call.get("team_public_model_name"),
        },
    )
    asyncio.run(
        model_management_endpoints._remove_unbacked_team_models(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L1834-L1907
            model_params=model,
            prisma_client=client,
            user_api_key_cache=cache,
            proxy_logging_obj=cast(ProxyLogging, None),  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/model_management_endpoints.py#L1834-L1844
            llm_router=router,
        )
    )
    report = {
        "team_models": None if teams.row is None else list(teams.row.models),
        "cache": cache_summary(cache, call.get("team_id")),
        "aliases": [
            AliasState(id=row.id, aliases=row.model_aliases) for row in aliases.rows
        ],
    }
    return JSON.validate_python(report)


def answer(call: JsonValue) -> JsonValue:
    function = TAG.validate_python(call)["function"]
    if function == "delete_team_model_alias":
        return alias_answer(ALIAS_CALL.validate_python(call))
    if function == "get_team_deployments":
        return deployments_answer(DEPLOYMENTS_CALL.validate_python(call))
    if function == "get_team_public_model_names":
        return public_names_answer(PUBLIC_NAMES_CALL.validate_python(call))
    if function == "remove_unbacked_team_models":
        return remove_answer(REMOVE_CALL.validate_python(call))
    raise ValueError(f"unknown function {function}")


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
