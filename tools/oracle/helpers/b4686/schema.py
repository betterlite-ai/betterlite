from typing import Final, NotRequired, Required, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue

DEFAULT_TOP_K: Final = 5


class NamedInput(TypedDict, total=False):
    id: Required[str]
    name: Required[str]
    description: str | None


class CallerInput(TypedDict, total=False):
    user_id: str | None
    team_id: str | None
    end_user_id: str | None


class NamedRecord:
    def __init__(self, fields: NamedInput) -> None:
        self.record_id = fields["id"]
        self.name = fields["name"]
        self.description = fields.get("description")


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


class AuthScopeArguments(TypedDict):
    client_ip: str | None
    mcp_auth_header: str | None
    mcp_server_auth_headers: dict[str, dict[str, str]]
    oauth2_headers: dict[str, str]
    raw_headers: dict[str, str]


class ToolListArguments(AuthScopeArguments):
    user_api_key_auth: Required[UserAPIKeyAuth]
    mcp_servers: list[str] | None
    mcp_proxy_mode: NotRequired[bool]


class AllowedArguments(TypedDict):
    user_api_key_auth: UserAPIKeyAuth
    mcp_servers: list[str] | None
    client_ip: str | None


class DeniedArguments(TypedDict):
    requested_names: list[str]
    user_api_key_auth: UserAPIKeyAuth
    client_ip: str | None


class ExecuteArguments(AuthScopeArguments):
    name: Required[str]
    arguments: Required[dict[str, JsonValue]]
    allowed_mcp_servers: Required[list[str]]
    user_api_key_auth: Required[UserAPIKeyAuth]
    requested_server_id: str | None
    guardrail_context: dict[str, JsonValue] | None


class AgentSearchArguments(TypedDict):
    query: str
    agents: list[NamedRecord]
    top_k: int
    embedding_model: str | None
    user_api_key_dict: UserAPIKeyAuth


class SkillSearchArguments(TypedDict):
    query: str
    skills: list[NamedRecord]
    top_k: int
    embedding_model: str | None
    user_api_key_dict: UserAPIKeyAuth


class EmbeddingArguments(TypedDict):
    model: str
    input: list[str]
    metadata: dict[str, JsonValue]
