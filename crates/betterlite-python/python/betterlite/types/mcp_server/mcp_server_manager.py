import json
from collections.abc import Mapping
from datetime import datetime
from typing import Annotated, ClassVar, Final, Literal, Self

from pydantic import (
    AfterValidator,
    BaseModel,
    ConfigDict,
    Field,
    JsonValue,
    TypeAdapter,
    field_validator,
    model_validator,
)
from typing_extensions import override

from betterlite.types.mcp.enums import (
    DEFAULT_SUBJECT_TOKEN_TYPE,
    MCPAuth,
    MCPAuthType,
    MCPTokenEndpointAuthMethod,
    MCPTransportType,
    MCPUpstreamProtocol,
)
from betterlite.types.mcp.headers import normalize_upstream_header_name

_UPSTREAM_PROTOCOL: Final[TypeAdapter[MCPUpstreamProtocol]] = TypeAdapter(MCPUpstreamProtocol)
_CREDENTIAL_HEADER_NAMES: Final = ("authorization", "x-api-key", "api-key", "apikey")
_HEADER_AUTH_TYPES: Final = frozenset(
    {
        None,
        MCPAuth.none,
        MCPAuth.api_key,
        MCPAuth.bearer_token,
        MCPAuth.basic,
        MCPAuth.authorization,
        MCPAuth.token,
        MCPAuth.aws_sigv4,
    }
)
# ruff S105 reads a literal assigned to a token_ name as a password, so the default sits here
_EXCHANGE_PROFILE: Final = "rfc8693"


def _validate_mcp_protocol_metadata(  # ast-grep-ignore: no-long-names
    value: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    if "protocol_version" in value:
        _UPSTREAM_PROTOCOL.validate_python(value["protocol_version"])
    return value


MCPInfo = Annotated[
    dict[str, JsonValue],
    AfterValidator(_validate_mcp_protocol_metadata),  # ast-grep-ignore: no-long-names
]


class MCPOAuthMetadata(BaseModel):
    scopes: list[str] | None = None
    authorization_url: str | None = None
    token_url: str | None = None
    registration_url: str | None = None
    discovered_issuer: str | None = None
    from_origin_fallback: bool = False


class MCPOAuthIdentityBinding(BaseModel):
    mode: Literal["disabled", "audit", "enforce"] = "disabled"
    issuer: str
    jwks_url: str | None = None
    audiences: Annotated[list[str], Field(min_length=1)]
    principal_claim: str = "email"
    caller_field: Literal["user_email", "user_id"] = "user_email"
    require_email_verified: bool = True


class PinnedMCPTool(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    description: str = ""
    input_schema: dict[str, JsonValue] = Field(default_factory=dict)


_PINNED_TOOLS: Final[TypeAdapter[dict[str, PinnedMCPTool] | None]] = TypeAdapter(
    dict[str, PinnedMCPTool] | None
)


def parse_pinned_tools(
    value: JsonValue | Mapping[str, PinnedMCPTool],
) -> dict[str, PinnedMCPTool] | None:
    decoded: JsonValue | Mapping[str, PinnedMCPTool] = value
    match value:
        case str() if value:
            decoded = json.loads(value)
        case str() | bool() | int() | float() | list() | Mapping() | None:
            pass
    if not decoded:
        return None
    return _PINNED_TOOLS.validate_python(decoded)


def _has_credential_header(headers: list[str] | None) -> bool:
    if headers is None:
        return False
    return any(header.lower() in _CREDENTIAL_HEADER_NAMES for header in headers)


class MCPServer(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)

    server_id: str
    name: str
    alias: str | None = None
    server_name: str | None = None
    url: str | None = None
    transport: MCPTransportType
    protocol_version: MCPUpstreamProtocol = "auto"
    spec_path: str | None = None
    auth_type: MCPAuthType | None = None
    authentication_token: str | None = None
    instructions: str | None = None
    mcp_info: MCPInfo | None = None
    extra_headers: list[str] | None = None
    allowed_tools: list[str] | None = None
    disallowed_tools: list[str] | None = None
    tool_name_to_display_name: dict[str, str] | None = None
    tool_name_to_description: dict[str, str] | None = None
    pinned_tools: dict[str, PinnedMCPTool] | None = None
    allowed_params: dict[str, list[str]] | None = None
    static_headers: dict[str, str] | None = None
    # each entry has name, value, scope and description; a global one fills ${NAME} in headers
    env_vars: list[dict[str, JsonValue]] | None = None
    client_id: str | None = None
    client_secret: str | None = None
    issuer: str | None = None
    issuer_is_anchored: bool = False
    scopes: list[str] | None = None
    authorization_url: str | None = None
    token_url: str | None = None
    registration_url: str | None = None
    configured_authorization_url: str | None = None
    configured_token_url: str | None = None
    configured_registration_url: str | None = None
    configured_scopes: tuple[str, ...] | None = None
    token_endpoint_auth_method: MCPTokenEndpointAuthMethod | None = None
    upstream_resource: str | None = None
    upstream_token_header: str | None = None

    @field_validator("upstream_token_header")
    @classmethod
    def _check_upstream_token_header(cls, value: str | None) -> str | None:
        if value is None or not value.strip():
            return None
        normalized: Final = normalize_upstream_header_name(value)
        if normalized is None:
            raise ValueError(
                "upstream_token_header must be a valid HTTP header name "
                f"(RFC 7230 token), got {value!r}"
            )
        return normalized

    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None
    aws_region_name: str | None = None
    aws_service_name: str | None = None
    aws_role_name: str | None = None
    aws_session_name: str | None = None
    token_exchange_endpoint: str | None = None
    audience: str | None = None
    subject_token_type: str = DEFAULT_SUBJECT_TOKEN_TYPE
    id_jag_resource_token_endpoint: str | None = None
    id_jag_resource: str | None = None
    client_private_key: str | None = None
    client_private_key_id: str | None = None
    client_assertion_signing_alg: str = "RS256"
    token_exchange_profile: str = _EXCHANGE_PROFILE
    command: str | None = None
    args: list[str] | None = None
    env: dict[str, str] | None = None
    access_groups: list[str] | None = None
    allow_all_keys: bool = False
    available_on_public_internet: bool = True
    delegate_auth_to_upstream: bool = False
    oauth_passthrough: bool = False
    dcr_bridge: bool | None = None
    per_server_oauth_discovery: bool = False
    is_byok: bool = False
    byok_description: list[str] = []
    byok_api_key_help_url: str | None = None
    source_url: str | None = None
    created_at: datetime | None = None
    updated_at: datetime | None = None
    oauth2_flow: Literal["client_credentials", "authorization_code"] | None = None
    token_validation: dict[str, JsonValue] | None = None
    oauth_identity_binding: MCPOAuthIdentityBinding | None = None
    token_storage_ttl_seconds: int | None = None
    timeout: float | None = None
    max_concurrent_requests: int | None = None
    short_prefix: str | None = None
    allow_sampling: bool = False
    allow_elicitation: bool = False

    @override
    def __repr__(self) -> str:
        return (
            f"MCPServer(server_id={self.server_id!r}, name={self.name!r}, "
            f"transport={self.transport!r}, auth_type={self.auth_type!r})"
        )

    @override
    def __str__(self) -> str:
        return self.__repr__()

    @property
    def effective_authorization_url(self) -> str | None:
        return self.authorization_url or self.configured_authorization_url

    @property
    def effective_token_url(self) -> str | None:
        return self.token_url or self.configured_token_url

    @property
    def effective_registration_url(self) -> str | None:
        return self.registration_url or self.configured_registration_url

    @property
    def has_client_credentials(self) -> bool:
        return self.oauth2_flow == "client_credentials"

    @model_validator(mode="after")
    def resolve_protocol_version(self) -> Self:
        if (
            "protocol_version" not in self.model_fields_set
            and self.mcp_info is not None
            and "protocol_version" in self.mcp_info
        ):
            self.protocol_version = _UPSTREAM_PROTOCOL.validate_python(
                self.mcp_info["protocol_version"]
            )
        return self

    @model_validator(mode="after")
    def validate_identity_binding_mode(self) -> Self:
        binding: Final = self.oauth_identity_binding
        if (
            binding is not None
            and binding.mode != "disabled"
            and (not self.needs_user_oauth_token or self.delegate_auth_to_upstream)
        ):
            raise ValueError(
                "oauth_identity_binding requires gateway-managed per-user OAuth2 credentials"
            )
        return self

    @property
    def needs_user_oauth_token(self) -> bool:
        return self.auth_type == MCPAuth.oauth2 and not self.has_client_credentials

    @property
    def is_gateway_managed_oauth2(self) -> bool:
        return self.auth_type == MCPAuth.oauth2 and not self.delegate_auth_to_upstream

    @property
    def uses_per_server_oauth_relay(self) -> bool:
        return (
            self.per_server_oauth_discovery
            and self.auth_type == MCPAuth.oauth2
            and not self.has_client_credentials
        )

    @property
    def advertises_gateway_authorization_server(self) -> bool:  # ast-grep-ignore: no-long-names
        if self.auth_type == MCPAuth.oauth2:
            return self.is_gateway_managed_oauth2 and not self.uses_per_server_oauth_relay
        return self.auth_type in _HEADER_AUTH_TYPES and not _has_credential_header(
            self.extra_headers
        )

    @property
    def is_true_passthrough(self) -> bool:
        return self.auth_type == MCPAuth.true_passthrough

    @property
    def is_oauth_delegate(self) -> bool:
        return self.auth_type == MCPAuth.oauth_delegate

    @property
    def is_client_forwarded_token(self) -> bool:
        return self.is_true_passthrough or self.is_oauth_delegate

    @property
    def is_dcr_bridge(self) -> bool:
        return bool(self.dcr_bridge) and self.is_client_forwarded_token

    @property
    def requires_per_user_auth(self) -> bool:
        if self.needs_user_oauth_token or self.is_client_forwarded_token:
            return True
        return self.auth_type == MCPAuth.none and _has_credential_header(self.extra_headers)

    @property
    def is_oauth_passthrough(self) -> bool:
        if self.auth_type not in (None, MCPAuth.none) or not self.extra_headers:
            return False
        if self.oauth_passthrough is not True:
            return False
        return any(header.lower() == "authorization" for header in self.extra_headers)
