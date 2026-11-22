import asyncio
import sys
from decimal import Decimal
from types import ModuleType
from typing import Literal, TypeAlias, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints.auth.managed_authorization import (
    admit_managed_actor,
    prepare_agent_invocation,
)
from litellm.proxy.agent_endpoints.identity_store import AgentIdentityStore
from litellm.types.agents import AgentResponse
from litellm.types.proxy.agent_identity import ManagedAgentContext
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

REGISTRY_MODULE = "litellm.proxy.agent_endpoints.agent_registry"
PERMISSIONS_MODULE = "litellm.proxy.agent_endpoints.auth.agent_permission_handler"
READ_THROUGH_MODULE = "litellm.proxy.common_utils.registry_read_through"
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
DETAIL: TypeAdapter[str] = TypeAdapter(str)


class ContextFields(TypedDict):
    agent_id: str
    binding_revision: str | None
    mode: Literal["autonomous", "delegated"]
    user_id: str | None
    subject_oid: str | None


class AuthFields(TypedDict):
    user_id: str | None
    team_id: str | None
    agent_id: str | None
    context: ContextFields | None


class AdmitCall(TypedDict):
    operation: Literal["admit"]
    auth: AuthFields
    registry: list[str]
    store: list[str] | None
    retired: bool
    granted: list[str]


class InvokeCall(TypedDict):
    operation: Literal["invoke"]
    auth: AuthFields
    target_name: str
    registry: list[str]
    store: list[str] | None
    granted: list[str]
    billable: bool


Input: TypeAlias = AdmitCall | InvokeCall


class Denial(TypedDict):
    status: int
    detail: str


class Answer(TypedDict):
    outcome: Literal["ok"] | Denial
    managed_agent_policy: str | None
    billing_agent_policy: str | None
    requires_fresh_policy: bool
    invoked_agent_id: str | None
    invoked_agent_policy: str | None
    agent_invocation_cost: str | None


class CaseAgentTable:
    def __init__(self, agents: list[AgentResponse]) -> None:
        self.agents = agents

    async def find_unique(
        self, *, where: dict[str, str], include: dict[str, bool]
    ) -> AgentResponse | None:
        del include
        for agent in self.agents:
            if agent.agent_id == where["agent_id"]:
                return agent
        return None


class CaseRetiredTable:
    def __init__(self, *, retired: bool) -> None:
        self.retired = retired

    async def find_unique(self, *, where: dict[str, str]) -> bool | None:
        del where
        if self.retired:
            return True
        return None


class CaseWriterDatabase:
    def __init__(self, agents: list[AgentResponse], *, retired: bool) -> None:
        self.litellm_agentstable = CaseAgentTable(agents)
        self.litellm_retiredagent = CaseRetiredTable(retired=retired)


class CaseClient:
    def __init__(self, agents: list[AgentResponse], *, retired: bool) -> None:
        self.writer_db = CaseWriterDatabase(agents, retired=retired)


class CaseRegistry:
    def __init__(self, agents: list[AgentResponse]) -> None:
        self.agents = agents

    def get_agent_by_id(self, agent_id: str) -> AgentResponse | None:
        for agent in self.agents:
            if agent.agent_id == agent_id:
                return agent
        return None

    def get_agent_by_name(self, agent_name: str) -> AgentResponse | None:
        for agent in self.agents:
            if agent.agent_name == agent_name:
                return agent
        return None


class RegistryModule(ModuleType):
    def __init__(self, registry: CaseRegistry) -> None:
        super().__init__(REGISTRY_MODULE)
        self.global_agent_registry = registry


class CaseAgentRequests:
    def __init__(self, granted: frozenset[str]) -> None:
        self.granted = granted

    async def is_agent_allowed(
        self, agent_id: str, user_api_key_auth: UserAPIKeyAuth | None = None
    ) -> bool:
        if user_api_key_auth is None:
            return False
        return agent_id in self.granted


class PermissionsModule(ModuleType):
    def __init__(self, granted: frozenset[str]) -> None:
        super().__init__(PERMISSIONS_MODULE)
        self.granted = granted
        self.AgentRequestHandler = CaseAgentRequests(granted)

    async def verified_human_agent_grants(
        self, user_id: str | None, team_id: str | None
    ) -> frozenset[str]:
        del team_id
        if user_id is None:
            return frozenset()
        return self.granted


class ReadThroughModule(ModuleType):
    def __init__(self, registry: CaseRegistry) -> None:
        super().__init__(READ_THROUGH_MODULE)
        self.agents = registry

    async def get_agent_with_read_through(self, agent_id_or_name: str) -> AgentResponse | None:
        by_id = self.agents.get_agent_by_id(agent_id_or_name)
        if by_id is not None:
            return by_id
        return self.agents.get_agent_by_name(agent_id_or_name)


def install(rows: list[str], granted: list[str]) -> None:
    agents = [AGENT.validate_json(row) for row in rows]
    registry = CaseRegistry(agents)
    sys.modules[REGISTRY_MODULE] = RegistryModule(registry)
    sys.modules[PERMISSIONS_MODULE] = PermissionsModule(frozenset(granted))
    sys.modules[READ_THROUGH_MODULE] = ReadThroughModule(registry)


def store_of(rows: list[str] | None, *, retired: bool) -> AgentIdentityStore | None:
    if rows is None:
        return None
    agents = [AGENT.validate_json(row) for row in rows]
    return AgentIdentityStore.from_client(CaseClient(agents, retired=retired))


def token_of(fields: AuthFields) -> UserAPIKeyAuth:
    auth = UserAPIKeyAuth(
        user_id=fields["user_id"],
        team_id=fields["team_id"],
        agent_id=fields["agent_id"],
    )
    context = fields["context"]
    if context is not None:
        # the constructor strips this field, so the case assigns it afterwards
        auth.managed_agent_context = ManagedAgentContext(
            agent_id=context["agent_id"],
            binding_revision=context["binding_revision"],
            mode=context["mode"],
            user_id=context["user_id"],
            subject_oid=context["subject_oid"],
        )
    return auth


def answer(auth: UserAPIKeyAuth, verdict: Literal["ok"] | Denial) -> JsonValue:
    managed = auth.managed_agent_policy
    billing = auth.billing_agent_policy
    invoked = auth.invoked_agent_policy
    cost = auth.agent_invocation_cost
    result = Answer(
        outcome=verdict,
        managed_agent_policy=None if managed is None else managed.agent_id,
        billing_agent_policy=None if billing is None else billing.agent_id,
        requires_fresh_policy=auth.requires_fresh_policy,
        invoked_agent_id=auth.invoked_agent_id,
        invoked_agent_policy=None if invoked is None else invoked.agent_id,
        agent_invocation_cost=None
        if cost is None
        else format(Decimal(repr(cost)).normalize(), "f"),
    )
    return JSON.validate_python(result)


def run(input_value: Input) -> JsonValue:
    install(input_value["registry"], input_value["granted"])
    auth = token_of(input_value["auth"])
    try:
        if input_value["operation"] == "admit":
            asyncio.run(
                admit_managed_actor(
                    auth,
                    store_of(input_value["store"], retired=input_value["retired"]),
                )
            )
        else:
            asyncio.run(
                prepare_agent_invocation(
                    auth,
                    input_value["target_name"],
                    store_of(input_value["store"], retired=False),
                    billable=input_value["billable"],
                )
            )
    except HTTPException as error:
        return answer(
            auth,
            Denial(
                status=error.status_code,
                detail=DETAIL.validate_python(error.detail),
            ),
        )
    return answer(auth, "ok")
