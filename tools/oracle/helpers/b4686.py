import asyncio
import sys
from dataclasses import dataclass
from datetime import datetime
from types import ModuleType
from typing import Final, NotRequired, Required, TypedDict, Unpack

import litellm
from fastapi import HTTPException
from litellm.llms.litellm_proxy.skills.skill_search import (
    MAX_SKILL_SEARCH_TOP_K,
    SkillSearchIndex,
    global_skill_search_index,
)
from litellm.proxy._experimental.mcp_server import tool_search
from litellm.proxy._experimental.mcp_server.tool_outcome import TextResult
from litellm.proxy._experimental.mcp_server.tool_search import (
    handle_agent_search,
    handle_mcp_proxy_tool,
    handle_mcp_tool_call,
    handle_mcp_tool_search,
    handle_skill_search,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints.agent_search import AgentSearchIndex, global_agent_search_index
from mcp.types import (
    AudioContent,
    CallToolResult,
    ContentBlock,
    EmbeddedResource,
    ImageContent,
    ResourceLink,
    TextContent,
    Tool,
)
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
AGENT_MODULE: Final = "litellm.proxy.agent_endpoints.agent_search"
PERMISSION_MODULE: Final = "litellm.proxy.agent_endpoints.auth.agent_permission_handler"
RBAC_MODULE: Final = "litellm.proxy.common_utils.rbac_utils"
SKILLS_MODULE: Final = "litellm.llms.litellm_proxy.skills.handler"
SKILL_MODULE: Final = "litellm.llms.litellm_proxy.skills.skill_search"
OPERATIONS_MODULE: Final = "litellm.proxy._experimental.mcp_server.operations"
DEFAULT_TOP_K: Final = 5
AGENT_SCORE: Final = 0.5
SKILL_SCORE: Final = 0.25

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class NamedInput(TypedDict, total=False):
    id: Required[str]
    name: Required[str]
    description: str | None


class CallerInput(TypedDict, total=False):
    user_id: str | None
    team_id: str | None
    end_user_id: str | None


class ToolFields(TypedDict, total=False):
    name: Required[str]
    server_id: Required[str]
    description: str | None
    inputSchema: dict[str, JsonValue]
    outputSchema: dict[str, JsonValue]


class Input(TypedDict, total=False):
    handler: Required[str]
    query: str
    top_k: int
    name: str
    arguments: dict[str, JsonValue]
    tool_name: str
    call_arguments: dict[str, JsonValue]
    requested_server_id: str | None
    guardrail_context: dict[str, JsonValue] | None
    caller: CallerInput
    feature_denied: bool
    agents: list[NamedInput]
    skills: list[NamedInput]
    agent_outcome: str
    agent_reason: str
    skill_outcome: str
    skill_reason: str
    agent_embedding_model: str | None
    skill_embedding_model: str | None
    settings: dict[str, JsonValue] | None
    has_router: bool
    tools: list[ToolFields]
    mcp_servers: list[str] | None
    allowed_servers: list[str]
    denial_detail: str
    client_ip: str | None
    mcp_auth_header: str | None
    mcp_server_auth_headers: dict[str, dict[str, str]]
    oauth2_headers: dict[str, str]
    raw_headers: dict[str, str]


@dataclass(frozen=True)
class Scope:
    client_ip: str | None
    mcp_servers: list[str] | None
    mcp_auth_header: str | None
    mcp_server_auth_headers: dict[str, dict[str, str]]
    oauth2_headers: dict[str, str]
    raw_headers: dict[str, str]


class SearchableSkill(BaseModel):
    name: str
    description: str
    tags: list[str]


class AgentHit(BaseModel):
    agent_id: str
    agent_name: str
    description: str
    skills: list[SearchableSkill]
    score: float


class SkillHit(BaseModel):
    skill_id: str
    display_title: str | None
    description: str | None
    score: float


@dataclass(frozen=True)
class AgentSearchHits:
    hits: list[AgentHit]


@dataclass(frozen=True)
class AgentSearchNotConfigured:
    reason: str


@dataclass(frozen=True)
class AgentSearchEmbeddingFailed:
    reason: str


@dataclass(frozen=True)
class SkillSearchHits:
    hits: list[SkillHit]


@dataclass(frozen=True)
class SkillSearchNotConfigured:
    reason: str


@dataclass(frozen=True)
class SkillSearchEmbeddingFailed:
    reason: str


@dataclass(frozen=True)
class Listing:
    tools: list[Tool]


class Observations:
    def __init__(self) -> None:
        self.feature_checks: list[JsonValue] = []
        self.agent_searches: list[JsonValue] = []
        self.skill_searches: list[JsonValue] = []
        self.list_calls: list[JsonValue] = []
        self.allowed_calls: list[JsonValue] = []
        self.denials: list[JsonValue] = []
        self.executions: list[JsonValue] = []
        self.embeddings: list[JsonValue] = []

    def value(self) -> JsonValue:
        return JSON.validate_python(
            {
                "feature_checks": self.feature_checks,
                "agent_searches": self.agent_searches,
                "skill_searches": self.skill_searches,
                "list_calls": self.list_calls,
                "allowed_calls": self.allowed_calls,
                "denials": self.denials,
                "executions": self.executions,
                "embeddings": self.embeddings,
            }
        )


class FakeEmbeddingResponse:
    def __init__(self, data: list[dict[str, list[float]]], metadata: dict[str, JsonValue]) -> None:
        self.data = data
        self.metadata = metadata

    def model_dump(self) -> dict[str, JsonValue]:
        return {"data": JSON.validate_python(self.data), "metadata": self.metadata}


class EmbeddingArguments(TypedDict):
    model: str
    input: list[str]
    metadata: dict[str, JsonValue]


class FakeRouter:
    def __init__(self, seen: Observations) -> None:
        self.seen = seen

    async def aembedding(self, **kwargs: Unpack[EmbeddingArguments]) -> FakeEmbeddingResponse:
        texts = kwargs["input"]
        self.seen.embeddings.append(
            JSON.validate_python({"model": kwargs["model"], "texts": texts})
        )
        vectors = [{"embedding": [1.0, 0.0] if "alpha" in text else [0.0, 1.0]} for text in texts]
        return FakeEmbeddingResponse(vectors, kwargs["metadata"])


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


class NamedRecord:
    def __init__(self, fields: NamedInput) -> None:
        self.record_id = fields["id"]
        self.name = fields["name"]
        self.description = fields.get("description")


def query_of(input_value: Input) -> str:
    query = input_value.get("query")
    if query is None:
        return ""
    return query


def top_k_of(input_value: Input) -> int:
    top_k = input_value.get("top_k")
    if top_k is None:
        return DEFAULT_TOP_K
    return top_k


def reason_of(reason: str | None) -> str:
    if reason is None:
        return ""
    return reason


def principal_of(input_value: Input) -> UserAPIKeyAuth:
    caller = input_value.get("caller")
    if caller is None:
        return UserAPIKeyAuth()
    return UserAPIKeyAuth(
        user_id=caller.get("user_id"),
        team_id=caller.get("team_id"),
        end_user_id=caller.get("end_user_id"),
    )


def header_map(headers: dict[str, str] | None) -> dict[str, str]:
    if headers is None:
        return {}
    return headers


def server_header_map(headers: dict[str, dict[str, str]] | None) -> dict[str, dict[str, str]]:
    if headers is None:
        return {}
    return headers


def raw_headers_of(input_value: Input) -> dict[str, str]:
    headers = input_value.get("raw_headers")
    if headers is None:
        return {}
    return headers


def caller_json(key: UserAPIKeyAuth) -> JsonValue:
    return JSON.validate_python({"user_id": key.user_id, "end_user_id": key.end_user_id})


def scope_json(scope: Scope) -> JsonValue:
    return JSON.validate_python(
        {
            "client_ip": scope.client_ip,
            "mcp_servers": scope.mcp_servers,
            "mcp_auth_header": scope.mcp_auth_header,
            "mcp_server_auth_headers": scope.mcp_server_auth_headers,
            "oauth2_headers": scope.oauth2_headers,
            "raw_headers": scope.raw_headers,
        }
    )


def client_scope_json(client_ip: str | None, mcp_servers: list[str] | None) -> JsonValue:
    return JSON.validate_python({"client_ip": client_ip, "mcp_servers": mcp_servers})


def execution_scope_json(
    client_ip: str | None,
    mcp_auth_header: str | None,
    mcp_server_auth_headers: dict[str, dict[str, str]] | None,
    oauth2_headers: dict[str, str] | None,
    raw_headers: dict[str, str] | None,
) -> JsonValue:
    return JSON.validate_python(
        {
            "client_ip": client_ip,
            "mcp_auth_header": mcp_auth_header,
            "mcp_server_auth_headers": server_header_map(mcp_server_auth_headers),
            "oauth2_headers": header_map(oauth2_headers),
            "raw_headers": header_map(raw_headers),
        }
    )


def named_records(fields_list: list[NamedInput] | None) -> list[NamedRecord]:
    if fields_list is None:
        return []
    return [NamedRecord(fields) for fields in fields_list]


def guardrail_of(input_value: Input) -> dict[str, JsonValue] | None:
    context = input_value.get("guardrail_context")
    if context is None:
        return None
    metadata = context.get("metadata")
    if metadata is None:
        metadata = {}
    return {"model": context.get("model"), "metadata": metadata}


def agent_hits(agents: list[NamedRecord]) -> list[AgentHit]:
    return [
        AgentHit(
            agent_id=agent.record_id,
            agent_name=agent.name,
            description=f"agent description of {agent.record_id}",
            skills=[
                SearchableSkill(
                    name=f"skill-{agent.record_id}",
                    description=f"skill description of {agent.record_id}",
                    tags=["seeded"],
                )
            ],
            score=AGENT_SCORE,
        )
        for agent in agents
    ]


def skill_hits(skills: list[NamedRecord]) -> list[SkillHit]:
    return [
        SkillHit(
            skill_id=skill.record_id,
            display_title=skill.name,
            description=skill.description,
            score=SKILL_SCORE,
        )
        for skill in skills
    ]


def tool_of(fields: ToolFields) -> Tool:
    schema = fields.get("inputSchema")
    if schema is None:
        schema = {}
    return Tool(
        name=fields["name"],
        description=fields.get("description"),
        input_schema=schema,
        output_schema=fields.get("outputSchema"),
    )


def tools_of(input_value: Input) -> list[Tool]:
    fields_list = input_value.get("tools")
    if fields_list is None:
        return []
    return [
        tool_search.with_mcp_proxy_identity(tool_of(fields), fields["server_id"])
        for fields in fields_list
    ]


def agent_outcome(
    input_value: Input, agents: list[NamedRecord]
) -> AgentSearchHits | AgentSearchNotConfigured | AgentSearchEmbeddingFailed:
    kind = input_value.get("agent_outcome")
    if kind == "not_configured":
        return AgentSearchNotConfigured(reason=reason_of(input_value.get("agent_reason")))
    if kind == "embedding_failed":
        return AgentSearchEmbeddingFailed(reason=reason_of(input_value.get("agent_reason")))
    return AgentSearchHits(hits=agent_hits(agents))


def skill_outcome(
    input_value: Input, skills: list[NamedRecord]
) -> SkillSearchHits | SkillSearchNotConfigured | SkillSearchEmbeddingFailed:
    kind = input_value.get("skill_outcome")
    if kind == "not_configured":
        return SkillSearchNotConfigured(reason=reason_of(input_value.get("skill_reason")))
    if kind == "embedding_failed":
        return SkillSearchEmbeddingFailed(reason=reason_of(input_value.get("skill_reason")))
    return SkillSearchHits(hits=skill_hits(skills))


class AgentSearchArguments(TypedDict):
    query: str
    agents: list[NamedRecord]
    top_k: int
    router: FakeRouter | None
    embedding_model: str | None
    index: AgentSearchIndex
    user_api_key_dict: UserAPIKeyAuth
    proxy_logging_obj: FakeLogging


class SkillSearchArguments(TypedDict):
    query: str
    skills: list[NamedRecord]
    top_k: int
    router: FakeRouter | None
    embedding_model: str | None
    index: SkillSearchIndex
    user_api_key_dict: UserAPIKeyAuth
    proxy_logging_obj: FakeLogging


class ToolListArguments(TypedDict):
    user_api_key_auth: UserAPIKeyAuth
    mcp_servers: list[str] | None
    client_ip: str | None
    mcp_auth_header: str | None
    mcp_server_auth_headers: dict[str, dict[str, str]] | None
    oauth2_headers: dict[str, str] | None
    raw_headers: dict[str, str] | None
    mcp_proxy_mode: NotRequired[bool]


class AllowedArguments(TypedDict):
    user_api_key_auth: UserAPIKeyAuth
    mcp_servers: list[str] | None
    client_ip: str | None


class DeniedArguments(TypedDict):
    requested_names: list[str]
    user_api_key_auth: UserAPIKeyAuth
    client_ip: str | None


class ExecuteArguments(TypedDict):
    name: str
    arguments: dict[str, JsonValue]
    allowed_mcp_servers: list[str]
    start_time: datetime
    user_api_key_auth: UserAPIKeyAuth
    mcp_auth_header: str | None
    mcp_server_auth_headers: dict[str, dict[str, str]] | None
    oauth2_headers: dict[str, str] | None
    raw_headers: dict[str, str] | None
    client_ip: str | None
    litellm_logging_obj: FakeLogging | None
    requested_server_id: str | None
    guardrail_context: dict[str, JsonValue] | None


class Fakes:
    def __init__(self, input_value: Input) -> None:
        self.input_value = input_value
        self.seen = Observations()
        self.principal = principal_of(input_value)
        self.router = FakeRouter(self.seen)
        self.logging = FakeLogging(self.principal)
        self.server_headers = server_header_map(input_value.get("mcp_server_auth_headers"))
        self.oauth2_headers = header_map(input_value.get("oauth2_headers"))
        self.raw_headers = raw_headers_of(input_value)


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
        scope = Scope(
            client_ip=kwargs["client_ip"],
            mcp_servers=kwargs["mcp_servers"],
            mcp_auth_header=kwargs["mcp_auth_header"],
            mcp_server_auth_headers=server_header_map(kwargs["mcp_server_auth_headers"]),
            oauth2_headers=header_map(kwargs["oauth2_headers"]),
            raw_headers=header_map(kwargs["raw_headers"]),
        )
        self.fakes.seen.list_calls.append(
            JSON.validate_python(
                {
                    "proxy_mode": kwargs.get("mcp_proxy_mode") is True,
                    "caller": caller_json(kwargs["user_api_key_auth"]),
                    "scope": scope_json(scope),
                }
            )
        )
        return Listing(self.tools)

    async def _get_allowed_mcp_servers(self, **kwargs: Unpack[AllowedArguments]) -> list[str]:
        self.fakes.seen.allowed_calls.append(
            JSON.validate_python(
                {
                    "caller": caller_json(kwargs["user_api_key_auth"]),
                    "scope": client_scope_json(kwargs["client_ip"], kwargs["mcp_servers"]),
                }
            )
        )
        allowed = self.fakes.input_value.get("allowed_servers")
        if allowed is None:
            return []
        return allowed

    async def raise_denied_scoped_mcp_access(self, **kwargs: Unpack[DeniedArguments]) -> None:
        self.fakes.seen.denials.append(
            JSON.validate_python(
                {
                    "requested": kwargs["requested_names"],
                    "caller": caller_json(kwargs["user_api_key_auth"]),
                    "client_ip": kwargs["client_ip"],
                }
            )
        )
        detail = self.fakes.input_value.get("denial_detail")
        if detail is None:
            detail = ""
        raise HTTPException(status_code=403, detail=detail)

    async def execute_mcp_tool(self, **kwargs: Unpack[ExecuteArguments]) -> TextResult:
        self.fakes.seen.executions.append(
            JSON.validate_python(
                {
                    "tool_name": kwargs["name"],
                    "arguments": kwargs["arguments"],
                    "allowed_servers": kwargs["allowed_mcp_servers"],
                    "requested_server_id": kwargs["requested_server_id"],
                    "guardrail_context": kwargs["guardrail_context"],
                    "caller": caller_json(kwargs["user_api_key_auth"]),
                    "scope": execution_scope_json(
                        kwargs["client_ip"],
                        kwargs["mcp_auth_header"],
                        kwargs["mcp_server_auth_headers"],
                        kwargs["oauth2_headers"],
                        kwargs["raw_headers"],
                    ),
                }
            )
        )
        return TextResult(text=f"executed {kwargs['name']}")


class FakeAgentModule(ModuleType):
    def __init__(self, fakes: Fakes) -> None:
        super().__init__(AGENT_MODULE)
        self.fakes = fakes
        self.AgentSearchHits = AgentSearchHits
        self.AgentSearchNotConfigured = AgentSearchNotConfigured
        self.AgentSearchEmbeddingFailed = AgentSearchEmbeddingFailed
        self.global_agent_search_index = global_agent_search_index

    async def search_agents(
        self, **kwargs: Unpack[AgentSearchArguments]
    ) -> AgentSearchHits | AgentSearchNotConfigured | AgentSearchEmbeddingFailed:
        self.fakes.seen.agent_searches.append(
            JSON.validate_python(
                {
                    "query": kwargs["query"],
                    "top_k": kwargs["top_k"],
                    "embedding_model": kwargs["embedding_model"],
                    "agent_ids": [agent.record_id for agent in kwargs["agents"]],
                    "caller": caller_json(kwargs["user_api_key_dict"]),
                }
            )
        )
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
        self.fakes.seen.feature_checks.append(
            JSON.validate_python({"feature": feature, "caller": caller_json(user_api_key_dict)})
        )
        if self.fakes.input_value.get("feature_denied") is True:
            raise HTTPException(status_code=403, detail=f"{feature} access is disabled")


class FakeSkillsHandler:
    def __init__(self, fakes: Fakes) -> None:
        self.fakes = fakes

    async def list_skills_for_search(self, user_api_key_dict: UserAPIKeyAuth) -> list[NamedRecord]:
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

    async def search_skills(
        self, **kwargs: Unpack[SkillSearchArguments]
    ) -> SkillSearchHits | SkillSearchNotConfigured | SkillSearchEmbeddingFailed:
        self.fakes.seen.skill_searches.append(
            JSON.validate_python(
                {
                    "query": kwargs["query"],
                    "top_k": kwargs["top_k"],
                    "embedding_model": kwargs["embedding_model"],
                    "skill_ids": [skill.record_id for skill in kwargs["skills"]],
                    "caller": caller_json(kwargs["user_api_key_dict"]),
                }
            )
        )
        return skill_outcome(self.fakes.input_value, kwargs["skills"])

    def skill_search_result(self, hit: SkillHit) -> SkillHit:
        return hit


async def dispatch(input_value: Input, fakes: Fakes) -> CallToolResult:
    handler = input_value["handler"]
    if handler == "agent_search":
        return await handle_agent_search(
            query=query_of(input_value),
            top_k=top_k_of(input_value),
            user_api_key_dict=fakes.principal,
        )
    if handler == "skill_search":
        return await handle_skill_search(
            query=query_of(input_value),
            top_k=top_k_of(input_value),
            user_api_key_dict=fakes.principal,
        )
    if handler == "mcp_tool_search":
        return await handle_mcp_tool_search(
            query=query_of(input_value),
            top_k=top_k_of(input_value),
            user_api_key_dict=fakes.principal,
            client_ip=input_value.get("client_ip"),
            mcp_servers=input_value.get("mcp_servers"),
            mcp_auth_header=input_value.get("mcp_auth_header"),
            mcp_server_auth_headers=fakes.server_headers,
            oauth2_headers=fakes.oauth2_headers,
            raw_headers=fakes.raw_headers,
        )
    if handler == "mcp_proxy_tool":
        name = input_value.get("name")
        if name is None:
            name = ""
        arguments = input_value.get("arguments")
        if arguments is None:
            arguments = {}
        return await handle_mcp_proxy_tool(
            name=name,
            arguments=arguments,  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/tool_search.py#L503-L506
            user_api_key_dict=fakes.principal,
            client_ip=input_value.get("client_ip"),
            mcp_servers=input_value.get("mcp_servers"),
            mcp_auth_header=input_value.get("mcp_auth_header"),
            mcp_server_auth_headers=fakes.server_headers,
            oauth2_headers=fakes.oauth2_headers,
            raw_headers=fakes.raw_headers,
        )
    if handler == "mcp_tool_call":
        tool_name = input_value.get("tool_name")
        if tool_name is None:
            tool_name = ""
        arguments = input_value.get("call_arguments")
        if arguments is None:
            arguments = {}
        return await handle_mcp_tool_call(
            tool_name=tool_name,
            arguments=arguments,
            user_api_key_dict=fakes.principal,
            client_ip=input_value.get("client_ip"),
            mcp_servers=input_value.get("mcp_servers"),
            mcp_auth_header=input_value.get("mcp_auth_header"),
            mcp_server_auth_headers=fakes.server_headers,
            oauth2_headers=fakes.oauth2_headers,
            raw_headers=fakes.raw_headers,
            requested_server_id=input_value.get("requested_server_id"),
            guardrail_context=guardrail_of(input_value),
        )
    raise ValueError(f"unknown handler: {handler}")


def text_block_json(text: str) -> JsonValue:
    try:
        parsed = JSON.validate_json(text)
    except ValidationError:
        return JSON.validate_python({"type": "text", "text": text})
    return JSON.validate_python({"type": "text", "json": parsed})


def content_json(block: ContentBlock) -> JsonValue:
    match block:
        case TextContent(text=text):
            return text_block_json(text)
        case ImageContent() | AudioContent() | ResourceLink() | EmbeddedResource():
            raise ValueError("the search handlers return text content blocks only")


def envelope(result: CallToolResult, observations: Observations) -> JsonValue:
    return JSON.validate_python(
        {
            "content": [content_json(block) for block in result.content],
            "isError": result.is_error,
            "observed": observations.value(),
        }
    )


def run(input_value: Input) -> JsonValue:
    fakes = Fakes(input_value)
    modules: list[ModuleType] = [
        FakeProxyModule(fakes),
        FakeOperationsModule(fakes),
        FakeAgentModule(fakes),
        FakePermissionModule(fakes),
        FakeRbacModule(fakes),
        FakeSkillsModule(fakes),
        FakeSkillModule(fakes),
    ]
    for module in modules:
        sys.modules[module.__name__] = module
    litellm.agent_search_embedding_model = input_value.get("agent_embedding_model")
    litellm.skill_search_embedding_model = input_value.get("skill_embedding_model")
    litellm.mcp_tool_search = input_value.get("settings")
    return envelope(asyncio.run(dispatch(input_value, fakes)), fakes.seen)
