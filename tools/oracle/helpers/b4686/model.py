from dataclasses import dataclass
from typing import Final, NotRequired, TypedDict

from litellm.llms.litellm_proxy.skills.skill_search import SkillSearchResult
from litellm.proxy.agent_endpoints.agent_search import AgentSearchResult
from mcp.types import Tool
from pydantic import JsonValue, TypeAdapter

ARGUMENTS: Final = TypeAdapter(dict[str, object])
JSON: Final = TypeAdapter(JsonValue)


class CallerRecord(TypedDict):
    user_id: str | None
    end_user_id: str | None


class ClientScopeRecord(TypedDict):
    client_ip: str | None
    mcp_servers: list[str] | None


class ExecutionScopeRecord(TypedDict):
    client_ip: str | None
    mcp_auth_header: str | None
    mcp_server_auth_headers: dict[str, dict[str, str]]
    oauth2_headers: dict[str, str]
    raw_headers: dict[str, str]


class ScopeRecord(ExecutionScopeRecord):
    mcp_servers: NotRequired[list[str] | None]


class FeatureCheckRecord(TypedDict):
    feature: str
    caller: CallerRecord


class AgentSearchRecord(TypedDict):
    query: str
    top_k: int
    embedding_model: str | None
    caller: CallerRecord
    agent_ids: NotRequired[list[str]]


class SkillSearchRecord(TypedDict):
    query: str
    top_k: int
    embedding_model: str | None
    caller: CallerRecord
    skill_ids: NotRequired[list[str]]


class ListCallRecord(TypedDict):
    proxy_mode: bool
    caller: CallerRecord
    scope: ScopeRecord


class AllowedCallRecord(TypedDict):
    caller: CallerRecord
    scope: ClientScopeRecord


class DenialRecord(TypedDict):
    requested: list[str]
    caller: CallerRecord
    client_ip: str | None


class ExecutionRecord(TypedDict):
    tool_name: str
    arguments: dict[str, JsonValue]
    allowed_servers: list[str]
    requested_server_id: str | None
    guardrail_context: dict[str, JsonValue] | None
    caller: CallerRecord
    scope: ExecutionScopeRecord


class EmbeddingRecord(TypedDict):
    model: str
    texts: list[str]


class Observed(TypedDict):
    feature_checks: list[FeatureCheckRecord]
    agent_searches: list[AgentSearchRecord]
    skill_searches: list[SkillSearchRecord]
    list_calls: list[ListCallRecord]
    allowed_calls: list[AllowedCallRecord]
    denials: list[DenialRecord]
    executions: list[ExecutionRecord]
    embeddings: list[EmbeddingRecord]


class Envelope(TypedDict):
    content: list[JsonValue]
    isError: bool
    observed: Observed


class AgentSkillPayload(TypedDict):
    name: str
    description: str
    tags: list[str]


class AgentHitPayload(TypedDict):
    agent_id: str
    agent_name: str
    description: str
    skills: list[AgentSkillPayload]
    score: float


class SkillHitPayload(TypedDict):
    skill_id: str
    display_title: str
    description: str | None
    score: float


@dataclass(frozen=True)
class AgentSearchHits:
    hits: list[AgentSearchResult]


@dataclass(frozen=True)
class AgentSearchNotConfigured:
    reason: str


@dataclass(frozen=True)
class AgentSearchEmbeddingFailed:
    reason: str


@dataclass(frozen=True)
class SkillSearchHits:
    hits: list[SkillSearchResult]


@dataclass(frozen=True)
class SkillSearchNotConfigured:
    reason: str


@dataclass(frozen=True)
class SkillSearchEmbeddingFailed:
    reason: str


@dataclass(frozen=True)
class Listing:
    tools: list[Tool]


AgentOutcome = AgentSearchHits | AgentSearchNotConfigured | AgentSearchEmbeddingFailed
SkillOutcome = SkillSearchHits | SkillSearchNotConfigured | SkillSearchEmbeddingFailed
