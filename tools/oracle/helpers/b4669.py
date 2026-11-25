import asyncio
from typing import Final, Literal, Protocol, TypedDict

from litellm.models.access_group import LiteLLM_AccessGroupTable
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.models.team import LiteLLM_TeamTableCachedObj, Member
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import MCPRequestHandler
from litellm.proxy._experimental.mcp_server.mcp_server_manager import global_mcp_server_manager
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import BaseModel, TypeAdapter


class ServerRecord(TypedDict):
    server_id: str
    name: str
    alias: str | None
    server_name: str | None
    transport: str
    access_groups: list[str] | None


class PermissionRecord(TypedDict):
    mcp_servers: list[str] | None
    mcp_access_groups: list[str] | None
    mcp_tool_permissions: dict[str, list[str]] | None
    mcp_toolsets: list[str] | None


class AccessGroupRecord(TypedDict):
    access_group_id: str
    access_mcp_server_ids: list[str]


class Fixtures(TypedDict):
    servers: list[ServerRecord]
    access_groups: list[AccessGroupRecord]
    toolset_grants: dict[str, list[str]]


class KeyCall(Fixtures):
    function: Literal["key"]
    permission: PermissionRecord
    access_group_ids: list[str]


class TeamCall(Fixtures):
    function: Literal["team"]
    team_id: str
    blocked: bool
    team_access_group_ids: list[str]
    permission: PermissionRecord | None


class UserTeamsCall(TypedDict):
    function: Literal["user_teams"]
    user_id: str
    teams: list[str]


Input = KeyCall | TeamCall | UserTeamsCall
CachedValue = BaseModel | dict[str, list[str]]


class CacheSeed(Protocol):
    def set_cache(
        self, key: str, value: CachedValue, *, model_type: type[BaseModel] | None = None
    ) -> None: ...


# The non-fresh reads answer from the cache, so this client only keeps the no-database branches from firing.
class StubClient(PrismaClient):
    def __init__(self) -> None:
        pass


SERVERS: Final[TypeAdapter[list[MCPServer]]] = TypeAdapter(list[MCPServer])


def run(input_value: Input) -> list[str]:
    saved_client = proxy_server.prisma_client
    saved_cache = proxy_server.user_api_key_cache
    saved_config = global_mcp_server_manager.config_mcp_servers
    saved_registry = global_mcp_server_manager.registry
    cache = UserApiKeyCache()
    proxy_server.prisma_client = StubClient()
    proxy_server.user_api_key_cache = cache
    try:
        return asyncio.run(dispatch(input_value, cache))
    finally:
        proxy_server.prisma_client = saved_client
        proxy_server.user_api_key_cache = saved_cache
        global_mcp_server_manager.config_mcp_servers = saved_config
        global_mcp_server_manager.registry = saved_registry


async def dispatch(input_value: Input, cache: CacheSeed) -> list[str]:
    match input_value["function"]:
        case "key":
            return await key_servers(input_value, cache)
        case "team":
            return await team_servers(input_value, cache)
        case "user_teams":
            return await user_team_ids(input_value, cache)


# a caller with no team, org, user, end user or agent, so the resolver answers the key level and its group grants
async def key_servers(call: KeyCall, cache: CacheSeed) -> list[str]:
    seed_fixtures(call, call["permission"], cache)
    auth = UserAPIKeyAuth(
        object_permission=permission_row(call["permission"]),
        access_group_ids=call["access_group_ids"],
    )
    return sorted(await MCPRequestHandler.get_allowed_mcp_servers(auth))


# a keyless caller pinned to one team, so the resolver answers that team's grant
async def team_servers(call: TeamCall, cache: CacheSeed) -> list[str]:
    seed_fixtures(call, call["permission"], cache)
    permission = call["permission"]
    team = LiteLLM_TeamTableCachedObj(
        team_id=call["team_id"],
        blocked=call["blocked"],
        access_group_ids=call["team_access_group_ids"],
        object_permission=None if permission is None else permission_row(permission),
    )
    cache.set_cache(
        f"team_id:{call['team_id']}", team, model_type=LiteLLM_TeamTableCachedObj
    )
    auth = UserAPIKeyAuth(team_id=call["team_id"])
    return sorted(await MCPRequestHandler.get_allowed_mcp_servers(auth))


# every listed team keeps the user on its roster, so each team the user's record resolves to becomes a source
async def user_team_ids(call: UserTeamsCall, cache: CacheSeed) -> list[str]:
    user_id = call["user_id"]
    user = LiteLLM_UserTable(user_id=user_id, teams=call["teams"])
    cache.set_cache(user_id, user, model_type=LiteLLM_UserTable)
    for team_id in call["teams"]:
        team = LiteLLM_TeamTableCachedObj(
            team_id=team_id, members_with_roles=[Member(user_id=user_id, role="user")]
        )
        cache.set_cache(
            f"team_id:{team_id}", team, model_type=LiteLLM_TeamTableCachedObj
        )
    sources = await MCPRequestHandler.admitted_subject_sources(
        UserAPIKeyAuth(user_id=user_id)
    )
    return [source.team_id for source in sources[1:] if source.team_id is not None]


def seed_fixtures(
    fixtures: Fixtures, permission: PermissionRecord | None, cache: CacheSeed
) -> None:
    servers = SERVERS.validate_python(fixtures["servers"])
    global_mcp_server_manager.config_mcp_servers = {
        server.server_id: server for server in servers
    }
    global_mcp_server_manager.registry = {}
    for group in fixtures["access_groups"]:
        group_id = group["access_group_id"]
        row = LiteLLM_AccessGroupTable(
            access_group_id=group_id,
            access_group_name=group_id,
            access_mcp_server_ids=group["access_mcp_server_ids"],
        )
        cache.set_cache(
            f"access_group_id:{group_id}", row, model_type=LiteLLM_AccessGroupTable
        )
    toolset_ids = None if permission is None else permission["mcp_toolsets"]
    if toolset_ids:
        cache.set_cache(
            "toolset_perms:" + ",".join(sorted(toolset_ids)), fixtures["toolset_grants"]
        )


def permission_row(permission: PermissionRecord) -> LiteLLM_ObjectPermissionTable:
    return LiteLLM_ObjectPermissionTable(
        object_permission_id="op-case",
        mcp_servers=permission["mcp_servers"],
        mcp_access_groups=permission["mcp_access_groups"],
        mcp_tool_permissions=permission["mcp_tool_permissions"],
        mcp_toolsets=permission["mcp_toolsets"],
    )
