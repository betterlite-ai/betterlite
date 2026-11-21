import asyncio
import sys
from types import ModuleType
from typing import Literal, TypeAlias, TypedDict

from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.agent_endpoints.auth.agent_permission_handler import (
    AgentAccess,
    RestrictedAgentAccess,
    UnrestrictedAgentAccess,
    accessible_agents,
    resolve_delegated_agent_team,
    verified_human_agent_grants,
)
from litellm.types.agents import AgentResponse
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

HUMANS_MODULE = "litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp"
PROXY_MODULE = "litellm.proxy.proxy_server"
CHECKS_MODULE = "litellm.proxy.auth.auth_checks"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)


class AgentFields(TypedDict):
    agent_id: str
    identity_managed: bool


class ContextFields(TypedDict):
    user_id: str


Grant: TypeAlias = Literal["unrestricted"] | list[str]


class AccessibleCall(TypedDict):
    operation: Literal["accessible"]
    agents: list[AgentFields]
    user_role: str | None
    contexts: list[ContextFields]
    access: dict[str, Grant]


class SourceFields(TypedDict):
    team_id: str | None
    agent_ids: list[str]


class HumanGrantsCall(TypedDict):
    operation: Literal["human_grants"]
    user_id: str | None
    team_id: str | None
    sources: list[SourceFields]


class DelegatedTeamCall(TypedDict):
    operation: Literal["delegated_team"]
    user_id: str | None
    team_id: str | None
    sources: list[SourceFields]
    agent_id: str
    explicit_team: bool
    allowed_team_ids: list[str] | None


Input: TypeAlias = AccessibleCall | HumanGrantsCall | DelegatedTeamCall


class Rejection(TypedDict):
    status: int
    detail: str


class CaseAccess:
    def __init__(self, call: AccessibleCall, lister: UserAPIKeyAuth) -> None:
        self.access = call["access"]
        self.lister = lister
        self.contexts = [UserAPIKeyAuth(user_id=context["user_id"]) for context in call["contexts"]]

    async def resolve(self, auth: UserAPIKeyAuth) -> AgentAccess:
        if auth.user_id is None:
            raise ValueError("every context names a user")
        grant = self.access[auth.user_id]
        if grant == "unrestricted":
            return UnrestrictedAgentAccess()
        return RestrictedAgentAccess(frozenset(grant))

    async def effective_contexts(self, auth: UserAPIKeyAuth) -> list[UserAPIKeyAuth]:
        if auth is not self.lister:
            raise ValueError("the contexts belong to the listing auth")
        return self.contexts


class CaseHumans:
    def __init__(self, sources: list[SourceFields]) -> None:
        self.sources = sources

    async def reload_admitted_user(
        self, user_id: str, *, requires_fresh_policy: bool = False
    ) -> UserAPIKeyAuth:
        if not requires_fresh_policy:
            raise ValueError("the human is reloaded without a fresh policy")
        return UserAPIKeyAuth(user_id=user_id)

    async def admitted_subject_sources(
        self, auth: UserAPIKeyAuth, *, allowed_team_ids: frozenset[str] | None = None
    ) -> list[UserAPIKeyAuth]:
        admitted: list[UserAPIKeyAuth] = []
        for source in self.sources:
            team_id = source["team_id"]
            if team_id is None:
                permission = LiteLLM_ObjectPermissionTable(
                    object_permission_id="user", agents=source["agent_ids"]
                )
                admitted.append(UserAPIKeyAuth(user_id=auth.user_id, object_permission=permission))
            elif allowed_team_ids is None or team_id in allowed_team_ids:
                admitted.append(UserAPIKeyAuth(user_id=auth.user_id, team_id=team_id))
        return admitted


class HumansModule(ModuleType):
    def __init__(self, handler: CaseHumans) -> None:
        super().__init__(HUMANS_MODULE)
        self.MCPRequestHandler = handler


class CaseDatabase:
    pass


class ProxyModule(ModuleType):
    def __init__(self) -> None:
        super().__init__(PROXY_MODULE)
        self.prisma_client = CaseDatabase()
        self.proxy_logging_obj = None
        self.user_api_key_cache = None


class TeamsModule(ModuleType):
    def __init__(self, sources: list[SourceFields]) -> None:
        super().__init__(CHECKS_MODULE)
        self.sources = sources

    async def get_team_object(
        self, team_id: str, *, check_db_only: bool, **_lookup: CaseDatabase | None
    ) -> LiteLLM_TeamTable:
        if not check_db_only:
            raise ValueError("a human's team is read from the database")
        for source in self.sources:
            if source["team_id"] == team_id:
                permission = LiteLLM_ObjectPermissionTable(
                    object_permission_id=team_id, agents=source["agent_ids"]
                )
                return LiteLLM_TeamTable(team_id=team_id, object_permission=permission)
        raise ValueError(f"the case has no team {team_id}")


def install_sources(sources: list[SourceFields]) -> None:
    sys.modules[HUMANS_MODULE] = HumansModule(CaseHumans(sources))
    sys.modules[PROXY_MODULE] = ProxyModule()
    sys.modules[CHECKS_MODULE] = TeamsModule(sources)


def listed_agent(agent: AgentFields) -> AgentResponse:
    fields: dict[str, JsonValue] = {
        "agent_id": agent["agent_id"],
        "agent_name": agent["agent_id"],
        "agent_card_params": {},
        "identity_managed": agent["identity_managed"],
    }
    return AGENT.validate_python(fields)


def accessible(call: AccessibleCall) -> JsonValue:
    role = call["user_role"]
    lister = UserAPIKeyAuth(user_role=None if role is None else LitellmUserRoles(role))
    case = CaseAccess(call, lister)
    agents = tuple(listed_agent(agent) for agent in call["agents"])
    listing = accessible_agents(
        lister,
        all_agents=agents,
        resolve_access=case.resolve,
        effective_contexts=case.effective_contexts,
    )
    listed: list[str] = [agent.agent_id for agent in asyncio.run(listing)]
    return JSON.validate_python(listed)


def human_grants(call: HumanGrantsCall) -> JsonValue:
    install_sources(call["sources"])
    granted = asyncio.run(verified_human_agent_grants(call["user_id"], call["team_id"]))
    ordered: list[str] = sorted(granted)
    return JSON.validate_python(ordered)


def delegated_team(call: DelegatedTeamCall) -> JsonValue:
    install_sources(call["sources"])
    allowed = call["allowed_team_ids"]
    choosing = resolve_delegated_agent_team(
        call["user_id"],
        call["agent_id"],
        call["team_id"],
        explicit_team=call["explicit_team"],
        allowed_team_ids=None if allowed is None else frozenset(allowed),
    )
    try:
        team = asyncio.run(choosing)
    except HTTPException as error:
        rejection: Rejection = {"status": error.status_code, "detail": error.detail}
        return JSON.validate_python(rejection)
    return team


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "accessible":
        return accessible(input_value)
    if input_value["operation"] == "human_grants":
        return human_grants(input_value)
    return delegated_team(input_value)
