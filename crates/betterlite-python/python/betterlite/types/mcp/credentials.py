from typing import Final

from typing_extensions import TypedDict

from betterlite.types.mcp.enums import MCPTokenEndpointAuthMethod


class MCPCredentials(TypedDict, total=False):
    auth_value: str | None
    client_id: str | None
    client_secret: str | None
    scopes: list[str] | None
    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    aws_region_name: str | None
    aws_service_name: str | None
    aws_role_name: str | None
    aws_session_name: str | None
    audience: str | None
    token_exchange_endpoint: str | None
    subject_token_type: str | None
    id_jag_resource_token_endpoint: str | None
    id_jag_resource: str | None
    upstream_resource: str | None
    upstream_token_header: str | None
    client_private_key: str | None
    client_private_key_id: str | None
    client_assertion_signing_alg: str | None
    token_endpoint_auth_method: MCPTokenEndpointAuthMethod | None
    redirect_uris: list[str] | None
    token_exchange_profile: str | None


MCP_ADMIN_CONFIG_CREDENTIAL_KEYS: Final[tuple[str, ...]] = (  # ast-grep-ignore: no-long-names
    "upstream_resource",
    "upstream_token_header",
)
