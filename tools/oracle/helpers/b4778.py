import asyncio
from collections.abc import Mapping, Sequence
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.models.access_group import LiteLLM_AccessGroupTable
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.proxy.agent_endpoints.auth.agent_access_groups import (
    AgentAccessGroupCeiling,
    resolve_agent_access_group_ceiling,
    resolve_managed_agent_ceilings,
)
from litellm.proxy.agent_endpoints.auth.agent_caller import agent_caller_auth
from litellm.proxy.agent_endpoints.auth.agent_permission_handler import (
    AgentRequestHandler,
    RestrictedAgentAccess,
    UnrestrictedAgentAccess,
)
from litellm.types.agents import AgentCaller, AgentResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter


class GroupLists(TypedDict):
    access_model_names: list[str]
    access_mcp_server_ids: list[str]
    access_agent_ids: list[str]


class CeilingCall(TypedDict):
    operation: Literal["ceiling"]
    agent_id: str
    agent_access_group_ids: list[str]
    groups: dict[str, GroupLists | None]


class ManagedCall(TypedDict):
    operation: Literal["managed_ceilings"]
    access_group_ids: list[str]


class CallerIds(TypedDict):
    user_id: str | None
    team_id: str | None


class CallerAuthCall(TypedDict):
    operation: Literal["caller_auth"]
    agent_id: str | None
    models: list[str]
    agent_caller: CallerIds | None
    requires_fresh_policy: bool


class Unrestricted(TypedDict):
    kind: Literal["unrestricted"]


class Restricted(TypedDict):
    kind: Literal["restricted"]
    restricted_ids: list[str]
    restricted_by_legacy_name: list[str]
    restricted_by_stable_name: list[str]


Side: TypeAlias = Unrestricted | Restricted


class AccessCall(TypedDict):
    operation: Literal["intersect_access"]
    config_agents: list[dict[str, JsonValue]]
    key_access: Side
    team_access: Side


Input: TypeAlias = CeilingCall | ManagedCall | CallerAuthCall | AccessCall


class CeilingView(TypedDict):
    access_group_ids: list[str]
    models: list[str]
    mcp_server_ids: list[str]
    agent_ids: list[str]


class CallerFields(TypedDict):
    user_id: str | None
    team_id: str | None
    agent_id: str | None
    models: list[str]


class CallerView(CallerFields):
    requires_fresh_policy: bool


@runtime_checkable
class ConfigRegistry(Protocol):
    def load_agents_from_config(
        self, agent_config: Sequence[Mapping[str, JsonValue]] | None = None
    ) -> None: ...


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
CALLER_FIELDS: TypeAdapter[CallerFields] = TypeAdapter(CallerFields)
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)
REGISTRY: TypeAdapter[ConfigRegistry] = TypeAdapter(
    ConfigRegistry, config=ConfigDict(arbitrary_types_allowed=True)
)
KEY_AGENT: Final = "invoking-agent"


class CaseGroups:
    def __init__(self, call: CeilingCall) -> None:
        self.call = call

    async def access_group_ids(self, agent_id: str) -> tuple[str, ...]:
        if agent_id != self.call["agent_id"]:
            return ()
        return tuple(self.call["agent_access_group_ids"])

    async def access_group(self, group_id: str) -> LiteLLM_AccessGroupTable | None:
        lists = self.call["groups"][group_id]
        if lists is None:
            return None
        return LiteLLM_AccessGroupTable(
            access_group_id=group_id,
            access_group_name=group_id,
            access_model_names=lists["access_model_names"],
            access_mcp_server_ids=lists["access_mcp_server_ids"],
            access_agent_ids=lists["access_agent_ids"],
        )


class TeamCeiling:
    def __init__(self, team_ids: frozenset[str] | None) -> None:
        self.team_ids = team_ids

    async def resolve(self, agent_id: str) -> AgentAccessGroupCeiling | None:
        if agent_id != KEY_AGENT or self.team_ids is None:
            return None
        return AgentAccessGroupCeiling(
            access_group_ids=("team",),
            models=frozenset(),
            mcp_server_ids=frozenset(),
            agent_ids=self.team_ids,
        )


def ceiling_view(ceiling: AgentAccessGroupCeiling) -> CeilingView:
    return {
        "access_group_ids": list(ceiling.access_group_ids),
        "models": sorted(ceiling.models),
        "mcp_server_ids": sorted(ceiling.mcp_server_ids),
        "agent_ids": sorted(ceiling.agent_ids),
    }


def ceiling(call: CeilingCall) -> JsonValue:
    groups = CaseGroups(call)
    found = asyncio.run(
        resolve_agent_access_group_ceiling(
            call["agent_id"],
            load_access_group_ids=groups.access_group_ids,
            load_access_group=groups.access_group,
        )
    )
    if found is None:
        return None
    return JSON.validate_python(ceiling_view(found))


def managed_ceilings(call: ManagedCall) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "agent_id": "managed-agent",
        "agent_name": "managed",
        "agent_card_params": {},
        "access_group_ids": list(call["access_group_ids"]),
    }
    agent = AGENT.validate_python(fields)
    ceilings = asyncio.run(resolve_managed_agent_ceilings(agent))
    views: list[CeilingView] = [ceiling_view(found) for found in ceilings]
    return JSON.validate_python(views)


def caller_auth(call: CallerAuthCall) -> JsonValue:
    key = UserAPIKeyAuth(agent_id=call["agent_id"], models=call["models"])
    caller = call["agent_caller"]
    if caller is not None:
        key.agent_caller = AgentCaller(user_id=caller["user_id"], team_id=caller["team_id"])
    key.requires_fresh_policy = call["requires_fresh_policy"]
    record = agent_caller_auth(key)
    if record is None:
        return None
    fields = CALLER_FIELDS.validate_json(record.model_dump_json())
    view: CallerView = {
        "user_id": fields["user_id"],
        "team_id": fields["team_id"],
        "agent_id": fields["agent_id"],
        "models": fields["models"],
        "requires_fresh_policy": record.requires_fresh_policy,
    }
    return JSON.validate_python(view)


def stable_id(name: str) -> str:
    agent = global_agent_registry.get_agent_by_name(name)
    if agent is None:
        raise ValueError(f"no config agent is named {name}")
    return agent.agent_id


def side_ids(side: Restricted) -> frozenset[str]:
    ids = set(side["restricted_ids"])
    for name in side["restricted_by_stable_name"]:
        ids.add(stable_id(name))
    for name in side["restricted_by_legacy_name"]:
        stable = stable_id(name)
        ids.update(global_agent_registry.ids_for_agent(stable) - {stable})
    return frozenset(ids)


def key_auth(side: Side) -> UserAPIKeyAuth:
    if side["kind"] == "unrestricted":
        return UserAPIKeyAuth(agent_id=KEY_AGENT)
    permission = LiteLLM_ObjectPermissionTable(
        object_permission_id="key", agents=sorted(side_ids(side))
    )
    return UserAPIKeyAuth(agent_id=KEY_AGENT, object_permission=permission)


def team_ceiling(side: Side) -> TeamCeiling:
    if side["kind"] == "unrestricted":
        return TeamCeiling(None)
    return TeamCeiling(side_ids(side))


def intersect_access(call: AccessCall) -> JsonValue:
    REGISTRY.validate_python(global_agent_registry).load_agents_from_config(call["config_agents"])
    key = key_auth(call["key_access"])
    team = team_ceiling(call["team_access"])
    resolving = AgentRequestHandler.resolve_agent_access(key, resolve_ceiling=team.resolve)
    access = asyncio.run(resolving)
    match access:
        case UnrestrictedAgentAccess():
            return "unrestricted"
        case RestrictedAgentAccess(agent_ids=agent_ids):
            restricted: list[str] = sorted(agent_ids)
            return JSON.validate_python(restricted)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "ceiling":
        return ceiling(input_value)
    if input_value["operation"] == "managed_ceilings":
        return managed_ceilings(input_value)
    if input_value["operation"] == "caller_auth":
        return caller_auth(input_value)
    return intersect_access(input_value)
