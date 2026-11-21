import asyncio
import sys
from collections.abc import Generator, Mapping, Sequence
from contextlib import contextmanager
from types import ModuleType
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from fastapi import HTTPException
from litellm.models.access_group import LiteLLM_AccessGroupTable
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints.auth.agent_permission_handler import (
    AgentRequestHandler,
    RestrictedAgentAccess,
    UnrestrictedAgentAccess,
)
from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.proxy.auth import auth_checks
from litellm.proxy.auth.auth_checks import TeamNotFoundError
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.repositories.prisma_protocols import DatabaseClient
from opentelemetry.trace import Span
from pydantic import ConfigDict, JsonValue, TypeAdapter

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final = TypeAdapter(JsonValue)
AGENT_GROUPS: Final = TypeAdapter(dict[str, list[str]])


class GroupSpec(TypedDict):
    agents: list[str]
    agent_access_groups: list[str]
    access_group_ids: list[str]


class KeySpec(TypedDict):
    team_id: str | None
    agents: list[str]
    agent_access_groups: list[str]
    access_group_ids: list[str]


class DbAgent(TypedDict):
    agent_id: str
    agent_access_groups: list[str]


class Input(TypedDict):
    operation: Literal["grants"]
    method: Literal["key", "key_team"]
    strict: bool
    database: bool
    key: KeySpec | None
    team: GroupSpec | None
    db_agents: list[DbAgent]
    groups: dict[str, list[str]]
    config_agents: list[dict[str, JsonValue]]
    fail: Literal["table", "team"] | None


AgentAccess: TypeAlias = UnrestrictedAgentAccess | RestrictedAgentAccess

# every argument type the two lookups take, so the fakes accept their keyword calls
LookupArgument: TypeAlias = (
    str | bool | None | Span | PrismaClient | DatabaseClient | UserApiKeyCache | ProxyLogging
)


@runtime_checkable
class ConfigRegistry(Protocol):
    def load_agents_from_config(
        self, agent_config: Sequence[Mapping[str, JsonValue]] | None = None
    ) -> None: ...


REGISTRY: Final = TypeAdapter(ConfigRegistry, config=ConfigDict(arbitrary_types_allowed=True))


class AgentRow:
    def __init__(self, agent_id: str, agent_access_groups: list[str]) -> None:
        self.agent_id = agent_id
        self.agent_access_groups = agent_access_groups


class AgentTable:
    def __init__(self, rows: list[AgentRow], *, fail: bool) -> None:
        self.rows = rows
        self.fail = fail

    async def find_many(self, *, where: Mapping[str, JsonValue]) -> list[AgentRow]:
        await asyncio.sleep(0)
        if self.fail:
            raise RuntimeError("the agents table read failed")
        wanted = AGENT_GROUPS.validate_python(where["agent_access_groups"])["hasSome"]
        return [
            row
            for row in self.rows
            if any(group in row.agent_access_groups for group in wanted)
        ]


class FakeDatabase:
    def __init__(self, agents: AgentTable) -> None:
        self.litellm_agentstable = agents


class FakePrisma:
    def __init__(self, agents: AgentTable) -> None:
        self.db = FakeDatabase(agents)
        self.writer_db = FakeDatabase(agents)


class ProxyServer(ModuleType):
    prisma_client: FakePrisma | None = None
    user_api_key_cache: UserApiKeyCache | None = None
    proxy_logging_obj: ProxyLogging | None = None


class FakeTeamLookup:
    def __init__(self, team: GroupSpec | None, *, fail: bool) -> None:
        self.team = team
        self.fail = fail

    async def __call__(
        self,
        team_id: str,
        *_args: LookupArgument,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_TeamTableCachedObj:
        await asyncio.sleep(0)
        team = self.team
        if team is None or self.fail:
            raise TeamNotFoundError(team_id=team_id)
        permission = LiteLLM_ObjectPermissionTable(
            object_permission_id="team-permission",
            agents=team["agents"],
            agent_access_groups=team["agent_access_groups"],
        )
        return LiteLLM_TeamTableCachedObj(
            team_id=team_id,
            object_permission=permission,
            access_group_ids=team["access_group_ids"],
        )


class FakeAccessLookup:
    def __init__(self, groups: dict[str, list[str]], *, fail: bool) -> None:
        self.groups = groups
        self.fail = fail

    async def __call__(
        self,
        access_group_id: str,
        *_args: LookupArgument,
        **_kwargs: LookupArgument,
    ) -> LiteLLM_AccessGroupTable:
        await asyncio.sleep(0)
        members = self.groups.get(access_group_id)
        if members is None or self.fail:
            detail = {"error": f"Access group doesn't exist in db. Access group={access_group_id}."}
            raise HTTPException(status_code=404, detail=detail)
        return LiteLLM_AccessGroupTable(
            access_group_id=access_group_id,
            access_group_name=access_group_id,
            access_agent_ids=members,
        )


def key_auth(spec: KeySpec | None) -> UserAPIKeyAuth | None:
    if spec is None:
        return None
    permission = LiteLLM_ObjectPermissionTable(
        object_permission_id="key-permission",
        agents=spec["agents"],
        agent_access_groups=spec["agent_access_groups"],
    )
    return UserAPIKeyAuth(
        team_id=spec["team_id"],
        object_permission=permission,
        access_group_ids=spec["access_group_ids"],
    )


@contextmanager
def patched(
    call: Input, lookup: FakeTeamLookup, access: FakeAccessLookup
) -> Generator[None]:
    rows = [
        AgentRow(row["agent_id"], row["agent_access_groups"]) for row in call["db_agents"]
    ]
    table = AgentTable(rows, fail=call["fail"] == "table")
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = FakePrisma(table) if call["database"] else None
    proxy_server.user_api_key_cache = UserApiKeyCache() if call["database"] else None
    saved_module = sys.modules.get(PROXY_MODULE)
    saved_lookup = auth_checks.get_team_object
    saved_access = auth_checks.get_access_object
    sys.modules[PROXY_MODULE] = proxy_server
    auth_checks.get_team_object = lookup
    auth_checks.get_access_object = access
    try:
        yield
    finally:
        auth_checks.get_team_object = saved_lookup
        auth_checks.get_access_object = saved_access
        if saved_module is None:
            del sys.modules[PROXY_MODULE]
        else:
            sys.modules[PROXY_MODULE] = saved_module


async def method_access(call: Input, key: UserAPIKeyAuth | None) -> AgentAccess:
    if call["method"] == "key":
        return await AgentRequestHandler.get_allowed_agents_for_key(
            key, strict=call["strict"]
        )
    return await AgentRequestHandler.resolve_key_team_agent_access(
        key, strict=call["strict"]
    )


def run(input_value: Input) -> JsonValue:
    REGISTRY.validate_python(global_agent_registry).load_agents_from_config(
        input_value["config_agents"]
    )
    lookup = FakeTeamLookup(input_value["team"], fail=input_value["fail"] == "team")
    access = FakeAccessLookup(input_value["groups"], fail=False)
    with patched(input_value, lookup, access):
        key = key_auth(input_value["key"])
        result = asyncio.run(method_access(input_value, key))
    match result:
        case UnrestrictedAgentAccess():
            return "unrestricted"
        case RestrictedAgentAccess(agent_ids=agent_ids):
            return JSON.validate_python(sorted(agent_ids))
