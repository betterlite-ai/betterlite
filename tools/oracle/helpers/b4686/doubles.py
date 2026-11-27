from types import ModuleType
from typing import Final, Unpack

from fastapi import HTTPException
from litellm.llms.litellm_proxy.skills.skill_search import (
    MAX_SKILL_SEARCH_TOP_K,
    global_skill_search_index,
)
from litellm.proxy._experimental.mcp_server.tool_outcome import TextResult
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints.agent_search import global_agent_search_index
from pydantic import BaseModel, JsonValue

from b4686.model import (
    AgentHit,
    AgentOutcome,
    AgentSearchEmbeddingFailed,
    AgentSearchHits,
    AgentSearchNotConfigured,
    Listing,
    Observed,
    SkillHit,
    SkillOutcome,
    SkillSearchEmbeddingFailed,
    SkillSearchHits,
    SkillSearchNotConfigured,
)
from b4686.readers import (
    agent_outcome,
    agent_search_record,
    allowed_call_record,
    denial_record,
    embedding_record,
    execution_record,
    feature_record,
    header_map,
    list_call_record,
    named_records,
    principal_of,
    raw_headers_of,
    server_header_map,
    skill_outcome,
    skill_search_record,
    tools_of,
)
from b4686.schema import (
    AgentSearchArguments,
    AllowedArguments,
    DeniedArguments,
    EmbeddingArguments,
    ExecuteArguments,
    Input,
    NamedRecord,
    SkillSearchArguments,
    ToolListArguments,
)

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
AGENT_MODULE: Final = "litellm.proxy.agent_endpoints.agent_search"
PERMISSION_MODULE: Final = "litellm.proxy.agent_endpoints.auth.agent_permission_handler"
RBAC_MODULE: Final = "litellm.proxy.common_utils.rbac_utils"
SKILLS_MODULE: Final = "litellm.llms.litellm_proxy.skills.handler"
SKILL_MODULE: Final = "litellm.llms.litellm_proxy.skills.skill_search"
OPERATIONS_MODULE: Final = "litellm.proxy._experimental.mcp_server.operations"


class FakeEmbeddingResponse(BaseModel):
    data: list[dict[str, list[float]]]
    metadata: dict[str, JsonValue]


class FakeRouter:
    def __init__(self, observed: Observed) -> None:
        self.observed = observed

    async def aembedding(self, **kwargs: Unpack[EmbeddingArguments]) -> FakeEmbeddingResponse:
        texts = kwargs["input"]
        self.observed["embeddings"].append(embedding_record(kwargs["model"], texts))
        vectors = [{"embedding": [1.0, 0.0] if "alpha" in text else [0.0, 1.0]} for text in texts]
        return FakeEmbeddingResponse(data=vectors, metadata=kwargs["metadata"])


class FakeLogging:
    def __init__(self, principal: UserAPIKeyAuth) -> None:
        self.principal = principal

    async def pre_call_hook(
        self, *, user_api_key_dict: UserAPIKeyAuth, data: dict[str, JsonValue], call_type: str
    ) -> dict[str, JsonValue]:
        if user_api_key_dict is not self.principal:
            raise ValueError("the embedding pre-call hook got another caller")
        if call_type != "aembedding":
            raise ValueError(f"the embedding pre-call hook got call type {call_type}")
        return data


class Fakes:
    def __init__(self, input_value: Input) -> None:
        self.input_value = input_value
        self.observed: Observed = {
            "feature_checks": [],
            "agent_searches": [],
            "skill_searches": [],
            "list_calls": [],
            "allowed_calls": [],
            "denials": [],
            "executions": [],
            "embeddings": [],
        }
        self.principal = principal_of(input_value)
        self.router = FakeRouter(self.observed)
        self.logging = FakeLogging(self.principal)
        self.server_headers = server_header_map(input_value.get("mcp_server_auth_headers"))
        self.oauth2_headers = header_map(input_value.get("oauth2_headers"))
        self.raw_headers = raw_headers_of(input_value)

    def allowed_servers(self) -> list[str]:
        allowed = self.input_value.get("allowed_servers")
        if allowed is None:
            return []
        return allowed

    def denial_detail(self) -> str:
        detail = self.input_value.get("denial_detail")
        if detail is None:
            return ""
        return detail


class FakeProxyModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(PROXY_MODULE)
        self.llm_router = fakes.router if fakes.input_value.get("has_router") is True else None
        self.proxy_logging_obj = fakes.logging


class FakeOperationsModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(OPERATIONS_MODULE)
        self.fakes = fakes
        self.tools = tools_of(fakes.input_value)

    async def _list_mcp_tools(self, **kwargs: Unpack[ToolListArguments]) -> Listing:
        self.fakes.observed["list_calls"].append(list_call_record(kwargs))
        return Listing(self.tools)

    async def _get_allowed_mcp_servers(self, **kwargs: Unpack[AllowedArguments]) -> list[str]:
        self.fakes.observed["allowed_calls"].append(allowed_call_record(kwargs))
        return self.fakes.allowed_servers()

    async def raise_denied_scoped_mcp_access(self, **kwargs: Unpack[DeniedArguments]) -> None:
        self.fakes.observed["denials"].append(denial_record(kwargs))
        raise HTTPException(status_code=403, detail=self.fakes.denial_detail())

    async def execute_mcp_tool(self, **kwargs: Unpack[ExecuteArguments]) -> TextResult:
        self.fakes.observed["executions"].append(execution_record(kwargs))
        return TextResult(text=f"executed {kwargs['name']}")


class FakeAgentModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(AGENT_MODULE)
        self.fakes = fakes
        self.AgentSearchHits = AgentSearchHits
        self.AgentSearchNotConfigured = AgentSearchNotConfigured
        self.AgentSearchEmbeddingFailed = AgentSearchEmbeddingFailed
        self.global_agent_search_index = global_agent_search_index

    async def search_agents(self, **kwargs: Unpack[AgentSearchArguments]) -> AgentOutcome:
        self.fakes.observed["agent_searches"].append(agent_search_record(kwargs))
        return agent_outcome(self.fakes.input_value, kwargs["agents"])

    def agent_search_result(self, hit: AgentHit) -> AgentHit:
        return hit


class FakePermissionModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(PERMISSION_MODULE)
        self.fakes = fakes

    async def accessible_agents(self, user_api_key_dict: UserAPIKeyAuth) -> list[NamedRecord]:
        if user_api_key_dict is not self.fakes.principal:
            raise ValueError("agent search resolved agents for another caller")
        return named_records(self.fakes.input_value.get("agents"))


class FakeRbacModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(RBAC_MODULE)
        self.fakes = fakes

    async def check_feature_access_for_user(
        self, user_api_key_dict: UserAPIKeyAuth, feature: str
    ) -> None:
        self.fakes.observed["feature_checks"].append(feature_record(user_api_key_dict, feature))
        if self.fakes.input_value.get("feature_denied") is True:
            raise HTTPException(status_code=403, detail=f"{feature} access is disabled")


class FakeSkillsHandler:
    def __init__(self, fakes: Fakes) -> None:
        self.fakes = fakes

    async def list_skills_for_search(
        self, user_api_key_dict: UserAPIKeyAuth
    ) -> list[NamedRecord]:
        if user_api_key_dict is not self.fakes.principal:
            raise ValueError("skill search resolved skills for another caller")
        return named_records(self.fakes.input_value.get("skills"))


class FakeSkillsModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(SKILLS_MODULE)
        self.LiteLLMSkillsHandler = FakeSkillsHandler(fakes)


class FakeSkillModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(SKILL_MODULE)
        self.fakes = fakes
        self.MAX_SKILL_SEARCH_TOP_K = MAX_SKILL_SEARCH_TOP_K
        self.SkillSearchHits = SkillSearchHits
        self.SkillSearchNotConfigured = SkillSearchNotConfigured
        self.SkillSearchEmbeddingFailed = SkillSearchEmbeddingFailed
        self.global_skill_search_index = global_skill_search_index

    async def search_skills(self, **kwargs: Unpack[SkillSearchArguments]) -> SkillOutcome:
        self.fakes.observed["skill_searches"].append(skill_search_record(kwargs))
        return skill_outcome(self.fakes.input_value, kwargs["skills"])

    def skill_search_result(self, hit: SkillHit) -> SkillHit:
        return hit
