from litellm.proxy._experimental.mcp_server import tool_search
from litellm.proxy._types import UserAPIKeyAuth
from mcp.types import Tool
from pydantic import JsonValue

from b4686.model import (
    AgentHit,
    AgentOutcome,
    AgentSearchEmbeddingFailed,
    AgentSearchHits,
    AgentSearchNotConfigured,
    AgentSearchRecord,
    AllowedCallRecord,
    CallerRecord,
    ClientScopeRecord,
    DenialRecord,
    EmbeddingRecord,
    ExecutionRecord,
    FeatureCheckRecord,
    ListCallRecord,
    ScopeRecord,
    SearchableSkill,
    SkillHit,
    SkillOutcome,
    SkillSearchEmbeddingFailed,
    SkillSearchHits,
    SkillSearchNotConfigured,
    SkillSearchRecord,
)
from b4686.schema import (
    DEFAULT_TOP_K,
    AgentSearchArguments,
    AllowedArguments,
    AuthScopeArguments,
    CallerInput,
    DeniedArguments,
    ExecuteArguments,
    Input,
    NamedInput,
    NamedRecord,
    SkillSearchArguments,
    ToolFields,
    ToolListArguments,
)

AGENT_SCORE = 0.5
SKILL_SCORE = 0.25


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


def caller_of(input_value: Input) -> CallerInput:
    caller = input_value.get("caller")
    if caller is None:
        return {}
    return caller


def principal_of(input_value: Input) -> UserAPIKeyAuth:
    caller = caller_of(input_value)
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


def guardrail_of(input_value: Input) -> dict[str, JsonValue] | None:
    context = input_value.get("guardrail_context")
    if context is None:
        return None
    metadata = context.get("metadata")
    if metadata is None:
        metadata = {}
    return {"model": context.get("model"), "metadata": metadata}


def named_records(fields_list: list[NamedInput] | None) -> list[NamedRecord]:
    if fields_list is None:
        return []
    return [NamedRecord(fields) for fields in fields_list]


def tool_schema(fields: ToolFields) -> dict[str, JsonValue]:
    schema = fields.get("inputSchema")
    if schema is None:
        return {}
    return schema


def tool_of(fields: ToolFields) -> Tool:
    return Tool(
        name=fields["name"],
        description=fields.get("description"),
        input_schema=tool_schema(fields),
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


def agent_hit(agent: NamedRecord) -> AgentHit:
    skill = SearchableSkill(
        name=f"skill-{agent.record_id}",
        description=f"skill description of {agent.record_id}",
        tags=["seeded"],
    )
    return AgentHit(
        agent_id=agent.record_id,
        agent_name=agent.name,
        description=f"agent description of {agent.record_id}",
        skills=[skill],
        score=AGENT_SCORE,
    )


def skill_hit(skill: NamedRecord) -> SkillHit:
    return SkillHit(
        skill_id=skill.record_id,
        display_title=skill.name,
        description=skill.description,
        score=SKILL_SCORE,
    )


def agent_outcome(input_value: Input, agents: list[NamedRecord]) -> AgentOutcome:
    kind = input_value.get("agent_outcome")
    if kind == "not_configured":
        return AgentSearchNotConfigured(reason=reason_of(input_value.get("agent_reason")))
    if kind == "embedding_failed":
        return AgentSearchEmbeddingFailed(reason=reason_of(input_value.get("agent_reason")))
    return AgentSearchHits(hits=[agent_hit(agent) for agent in agents])


def skill_outcome(input_value: Input, skills: list[NamedRecord]) -> SkillOutcome:
    kind = input_value.get("skill_outcome")
    if kind == "not_configured":
        return SkillSearchNotConfigured(reason=reason_of(input_value.get("skill_reason")))
    if kind == "embedding_failed":
        return SkillSearchEmbeddingFailed(reason=reason_of(input_value.get("skill_reason")))
    return SkillSearchHits(hits=[skill_hit(skill) for skill in skills])


def caller_record(key: UserAPIKeyAuth) -> CallerRecord:
    return {
        "user_id": key.user_id,
        "end_user_id": key.end_user_id,
    }


def client_scope(kwargs: AllowedArguments) -> ClientScopeRecord:
    return {
        "client_ip": kwargs["client_ip"],
        "mcp_servers": kwargs["mcp_servers"],
    }


def execution_scope(kwargs: AuthScopeArguments) -> ScopeRecord:
    return {
        "client_ip": kwargs["client_ip"], "mcp_auth_header": kwargs["mcp_auth_header"],
        "mcp_server_auth_headers": server_header_map(kwargs["mcp_server_auth_headers"]),
        "oauth2_headers": header_map(kwargs["oauth2_headers"]),
        "raw_headers": header_map(kwargs["raw_headers"]),
    }


def list_scope(kwargs: ToolListArguments) -> ScopeRecord:
    scope = execution_scope(kwargs)
    scope["mcp_servers"] = kwargs["mcp_servers"]
    return scope


def list_call_record(kwargs: ToolListArguments) -> ListCallRecord:
    return {
        "proxy_mode": kwargs.get("mcp_proxy_mode") is True,
        "caller": caller_record(kwargs["user_api_key_auth"]),
        "scope": list_scope(kwargs),
    }


def allowed_call_record(kwargs: AllowedArguments) -> AllowedCallRecord:
    return {
        "caller": caller_record(kwargs["user_api_key_auth"]),
        "scope": client_scope(kwargs),
    }


def denial_record(kwargs: DeniedArguments) -> DenialRecord:
    return {
        "requested": kwargs["requested_names"],
        "caller": caller_record(kwargs["user_api_key_auth"]),
        "client_ip": kwargs["client_ip"],
    }


def execution_record(kwargs: ExecuteArguments) -> ExecutionRecord:
    return {
        "tool_name": kwargs["name"],
        "arguments": kwargs["arguments"],
        "allowed_servers": kwargs["allowed_mcp_servers"],
        "requested_server_id": kwargs["requested_server_id"],
        "guardrail_context": kwargs["guardrail_context"],
        "caller": caller_record(kwargs["user_api_key_auth"]),
        "scope": execution_scope(kwargs),
    }


def feature_record(key: UserAPIKeyAuth, feature: str) -> FeatureCheckRecord:
    return {
        "feature": feature,
        "caller": caller_record(key),
    }


def embedding_record(model: str, texts: list[str]) -> EmbeddingRecord:
    return {
        "model": model,
        "texts": texts,
    }


def agent_search_fields(kwargs: AgentSearchArguments) -> AgentSearchRecord:
    return {
        "query": kwargs["query"],
        "top_k": kwargs["top_k"],
        "embedding_model": kwargs["embedding_model"],
        "caller": caller_record(kwargs["user_api_key_dict"]),
    }


def skill_search_fields(kwargs: SkillSearchArguments) -> SkillSearchRecord:
    return {
        "query": kwargs["query"],
        "top_k": kwargs["top_k"],
        "embedding_model": kwargs["embedding_model"],
        "caller": caller_record(kwargs["user_api_key_dict"]),
    }


def agent_search_record(kwargs: AgentSearchArguments) -> AgentSearchRecord:
    record = agent_search_fields(kwargs)
    record["agent_ids"] = [agent.record_id for agent in kwargs["agents"]]
    return record


def skill_search_record(kwargs: SkillSearchArguments) -> SkillSearchRecord:
    record = skill_search_fields(kwargs)
    record["skill_ids"] = [skill.record_id for skill in kwargs["skills"]]
    return record
