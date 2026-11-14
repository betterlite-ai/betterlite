from collections.abc import Mapping, Sequence
from datetime import datetime
from typing import Annotated, ClassVar, Final, Literal, TypeAlias
from urllib.parse import urlsplit

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    JsonValue,
    PrivateAttr,
    field_validator,
)
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.llms.base import LiteLLMPydanticObjectBase

class AgentProvider(TypedDict, total=False):
    organization: str
    url: str

class AgentExtension(TypedDict, total=False):
    uri: str
    description: str | None
    required: bool | None
    params: dict[str, JsonValue] | None

class AgentCapabilities(TypedDict, total=False):
    streaming: bool | None
    pushNotifications: bool | None
    stateTransitionHistory: bool | None
    extensions: list[AgentExtension] | None

class SecuritySchemeBase(TypedDict, total=False):
    description: str | None

class APIKeySecurityScheme(SecuritySchemeBase, total=False):
    type: Required[Literal["apiKey"]]
    in_: Required[Literal["query", "header", "cookie"]]
    name: Required[str]

class HTTPAuthSecurityScheme(SecuritySchemeBase, total=False):
    type: Required[Literal["http"]]
    scheme: Required[str]
    bearerFormat: str | None

class MutualTLSSecurityScheme(SecuritySchemeBase, total=False):
    type: Required[Literal["mutualTLS"]]

class OAuthFlows(TypedDict, total=False):
    authorizationCode: dict[str, JsonValue] | None
    clientCredentials: dict[str, JsonValue] | None
    implicit: dict[str, JsonValue] | None
    password: dict[str, JsonValue] | None

class OAuth2SecurityScheme(SecuritySchemeBase, total=False):
    type: Required[Literal["oauth2"]]
    flows: Required[OAuthFlows]
    oauth2MetadataUrl: str | None

class OpenIdConnectSecurityScheme(SecuritySchemeBase, total=False):
    type: Required[Literal["openIdConnect"]]
    openIdConnectUrl: Required[str]

SecurityScheme: TypeAlias = (
    APIKeySecurityScheme
    | HTTPAuthSecurityScheme
    | OAuth2SecurityScheme
    | OpenIdConnectSecurityScheme
    | MutualTLSSecurityScheme
)

class AgentSkill(TypedDict, total=False):
    id: str
    name: str
    description: str
    tags: list[str]
    examples: list[str] | None
    inputModes: list[str] | None
    outputModes: list[str] | None
    security: list[dict[str, list[str]]] | None

class AgentInterface(TypedDict, total=False):
    url: str
    transport: str

class AgentCardSignature(TypedDict, total=False):
    protected: str
    signature: str
    header: dict[str, JsonValue] | None

class AgentCard(TypedDict, total=False):
    protocolVersion: str
    name: str
    description: str
    url: str
    version: str
    capabilities: AgentCapabilities
    defaultInputModes: list[str]
    defaultOutputModes: list[str]
    skills: list[AgentSkill]

    preferredTransport: str | None
    additionalInterfaces: list[AgentInterface] | None
    iconUrl: str | None
    provider: AgentProvider | None
    documentationUrl: str | None
    securitySchemes: dict[str, SecurityScheme] | None
    security: list[dict[str, list[str]]] | None
    supportsAuthenticatedExtendedCard: bool | None  # ast-grep-ignore: no-long-names
    signatures: list[AgentCardSignature] | None

class AugmentedAgentCard(AgentCard):
    is_public: bool

class AgentObjectPermission(TypedDict, total=False):
    mcp_servers: list[str] | None
    mcp_access_groups: list[str] | None
    mcp_toolsets: ReadOnly[Sequence[str] | None]
    mcp_tool_permissions: dict[str, list[str]] | None
    models: list[str] | None
    agents: list[str] | None

class AgentKillSwitchBearerAuth(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")
    type: Literal["bearer"]
    token: str

class AgentKillSwitchApiKeyAuth(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")
    type: Literal["api_key"]
    header_name: str = "x-api-key"
    api_key: str

class AgentKillSwitchBasicAuth(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")
    type: Literal["basic"]
    username: str
    password: str

AgentKillSwitchAuth: TypeAlias = Annotated[
    AgentKillSwitchBearerAuth | AgentKillSwitchApiKeyAuth | AgentKillSwitchBasicAuth,
    Field(discriminator="type"),
]

AgentKillSwitchMethod: TypeAlias = Literal["POST", "PUT", "PATCH", "DELETE", "GET"]

class AgentKillSwitchConfig(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")
    url: str
    method: AgentKillSwitchMethod = "POST"
    headers: Mapping[str, str] = Field(default_factory=dict)
    query_params: Mapping[str, str] = Field(default_factory=dict)
    body: Mapping[str, JsonValue] | None = None
    auth: AgentKillSwitchAuth | None = None

    @field_validator("url")
    @classmethod
    def _require_absolute_http_url(cls, value: str) -> str:
        parts: Final = urlsplit(value)
        if parts.scheme not in ("http", "https") or not parts.netloc:
            raise ValueError("kill_switch.url must be an absolute http(s) URL")
        return value

class AgentKillSwitchResult(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    agent_id: str
    url: str
    method: AgentKillSwitchMethod
    status_code: int | None = None
    response_body: str | None = None
    error: str | None = None

    @property
    def succeeded(self) -> bool:
        return self.status_code is not None and self.status_code in range(200, 300)

class AgentConfig(TypedDict, total=False):
    # docs/deviations/B7942.yaml: B7949 owns EntraIdentityConfig, so identity is a mapping here
    identity: ReadOnly[dict[str, JsonValue] | None]
    enabled: ReadOnly[bool]
    execution_mode: ReadOnly[Literal["autonomous", "delegated", "both"]]
    agent_name: Required[str]
    agent_card_params: ReadOnly[AgentCard]
    litellm_params: dict[str, JsonValue]
    object_permission: AgentObjectPermission
    tpm_limit: int | None
    rpm_limit: int | None
    session_tpm_limit: int | None
    session_rpm_limit: int | None
    static_headers: dict[str, str] | None
    extra_headers: list[str] | None
    access_group_ids: ReadOnly[Sequence[str] | None]
    kill_switch: ReadOnly[AgentKillSwitchConfig | None]

class PatchAgentRequest(TypedDict, total=False):
    identity: ReadOnly[dict[str, JsonValue] | None]
    enabled: ReadOnly[bool]
    execution_mode: ReadOnly[Literal["autonomous", "delegated", "both"]]
    agent_name: str
    agent_card_params: AgentCard
    litellm_params: dict[str, JsonValue]
    object_permission: AgentObjectPermission
    tpm_limit: int | None
    rpm_limit: int | None
    session_tpm_limit: int | None
    session_rpm_limit: int | None
    static_headers: dict[str, str] | None
    extra_headers: list[str] | None
    access_group_ids: ReadOnly[Sequence[str] | None]
    kill_switch: ReadOnly[AgentKillSwitchConfig | None]

AGENT_CALLER_USER_ID_HEADER: Final = "x-litellm-user-id"
AGENT_CALLER_TEAM_ID_HEADER: Final = "x-litellm-team-id"

class AgentCaller(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    user_id: str | None = None
    team_id: str | None = None

class AgentKeySummary(BaseModel):
    token: str
    key_alias: str | None = None
    key_name: str | None = None

class AgentResponse(BaseModel):
    # docs/deviations/B7942.yaml: B7949 owns AgentIdentityBinding, so identity is a mapping here
    identity: dict[str, JsonValue] | None = None
    identity_managed: bool = False
    enabled: bool = True
    execution_mode: Literal["autonomous", "delegated", "both"] = "autonomous"
    jwt_auth_configured: bool = False
    agent_id: str
    agent_name: str
    litellm_params: dict[str, JsonValue] | None = None
    agent_card_params: dict[str, JsonValue]
    object_permission: dict[str, JsonValue] | None = None
    spend: float | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    session_tpm_limit: int | None = None
    session_rpm_limit: int | None = None
    static_headers: dict[str, str] | None = None
    extra_headers: list[str] | None = None
    access_group_ids: Sequence[str] | None = None
    kill_switch: AgentKillSwitchConfig | None = None
    keys: list[AgentKeySummary] | None = None
    search_score: float | None = None
    created_at: datetime | None = None
    updated_at: datetime | None = None
    created_by: str | None = None
    updated_by: str | None = None

class ListAgentsResponse(BaseModel):
    agents: list[AgentResponse]

class AgentCreateResponse(LiteLLMPydanticObjectBase):
    id: str | None = None
    name: str | None = None
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

class AgentDeleteResult(LiteLLMPydanticObjectBase):
    name: str
    deleted: bool = True
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

class AgentListResponse(LiteLLMPydanticObjectBase):
    agents: list[dict[str, JsonValue]] = Field(default_factory=list)
    next_page_token: str | None = None
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

class AgentVersionsResponse(LiteLLMPydanticObjectBase):
    agent_versions: list[dict[str, JsonValue]] = Field(default_factory=list)
    next_page_token: str | None = None
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

class AgentMakePublicResponse(BaseModel):
    message: str
    public_agent_groups: list[str]
    updated_by: str

class MakeAgentsPublicRequest(BaseModel):
    agent_ids: list[str]
