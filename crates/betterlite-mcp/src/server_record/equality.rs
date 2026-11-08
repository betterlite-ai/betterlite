use secrecy::ExposeSecret as _;

use super::{MCPServer, StoredSecret};

fn same_secret(left: Option<&StoredSecret>, right: Option<&StoredSecret>) -> bool {
    match (left, right) {
        (Some(first), Some(second)) => first.0.expose_secret() == second.0.expose_secret(),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn same_identity(left: &MCPServer, right: &MCPServer) -> bool {
    left.server_id == right.server_id
        && left.name == right.name
        && left.alias == right.alias
        && left.server_name == right.server_name
        && left.url == right.url
        && left.transport == right.transport
        && left.protocol_version == right.protocol_version
        && left.spec_path == right.spec_path
        && left.auth_type == right.auth_type
        && same_secret(
            left.authentication_token.as_ref(),
            right.authentication_token.as_ref(),
        )
        && left.instructions == right.instructions
        && left.mcp_info == right.mcp_info
}

fn same_catalog(left: &MCPServer, right: &MCPServer) -> bool {
    left.extra_headers == right.extra_headers
        && left.allowed_tools == right.allowed_tools
        && left.disallowed_tools == right.disallowed_tools
        && left.tool_name_to_display_name == right.tool_name_to_display_name
        && left.tool_name_to_description == right.tool_name_to_description
        && left.pinned_tools == right.pinned_tools
        && left.allowed_params == right.allowed_params
        && left.static_headers == right.static_headers
        && left.env_vars == right.env_vars
}

fn same_oauth(left: &MCPServer, right: &MCPServer) -> bool {
    left.client_id == right.client_id
        && same_secret(left.client_secret.as_ref(), right.client_secret.as_ref())
        && left.issuer == right.issuer
        && left.issuer_is_anchored == right.issuer_is_anchored
        && left.scopes == right.scopes
        && left.authorization_url == right.authorization_url
        && left.token_url == right.token_url
        && left.registration_url == right.registration_url
        && left.configured_authorization_url == right.configured_authorization_url
        && left.configured_token_url == right.configured_token_url
        && left.configured_registration_url == right.configured_registration_url
        && left.configured_scopes == right.configured_scopes
        && left.token_endpoint_auth_method == right.token_endpoint_auth_method
        && left.upstream_resource == right.upstream_resource
        && left.upstream_token_header == right.upstream_token_header
}

fn same_credentials(left: &MCPServer, right: &MCPServer) -> bool {
    same_secret(
        left.aws_access_key_id.as_ref(),
        right.aws_access_key_id.as_ref(),
    ) && same_secret(
        left.aws_secret_access_key.as_ref(),
        right.aws_secret_access_key.as_ref(),
    ) && same_secret(
        left.aws_session_token.as_ref(),
        right.aws_session_token.as_ref(),
    ) && left.aws_region_name == right.aws_region_name
        && left.aws_service_name == right.aws_service_name
        && left.aws_role_name == right.aws_role_name
        && left.aws_session_name == right.aws_session_name
        && left.token_exchange_endpoint == right.token_exchange_endpoint
        && left.audience == right.audience
        && left.subject_token_type == right.subject_token_type
        && left.id_jag_resource_token_endpoint == right.id_jag_resource_token_endpoint
        && left.id_jag_resource == right.id_jag_resource
        && same_secret(
            left.client_private_key.as_ref(),
            right.client_private_key.as_ref(),
        )
        && left.client_private_key_id == right.client_private_key_id
        && left.client_assertion_signing_alg == right.client_assertion_signing_alg
        && left.token_exchange_profile == right.token_exchange_profile
}

fn same_stdio(left: &MCPServer, right: &MCPServer) -> bool {
    left.command == right.command
        && left.args == right.args
        && left.env == right.env
        && left.access_groups == right.access_groups
}

fn same_flags(left: &MCPServer, right: &MCPServer) -> bool {
    left.allow_all_keys == right.allow_all_keys
        && left.available_on_public_internet == right.available_on_public_internet
        && left.delegate_auth_to_upstream == right.delegate_auth_to_upstream
        && left.oauth_passthrough == right.oauth_passthrough
        && left.dcr_bridge == right.dcr_bridge
        && left.per_server_oauth_discovery == right.per_server_oauth_discovery
        && left.is_byok == right.is_byok
        && left.byok_description == right.byok_description
        && left.byok_api_key_help_url == right.byok_api_key_help_url
        && left.source_url == right.source_url
}

fn same_runtime(left: &MCPServer, right: &MCPServer) -> bool {
    left.created_at == right.created_at
        && left.updated_at == right.updated_at
        && left.oauth2_flow == right.oauth2_flow
        && left.token_validation == right.token_validation
        && left.oauth_identity_binding == right.oauth_identity_binding
        && left.token_storage_ttl_seconds == right.token_storage_ttl_seconds
        && left.timeout == right.timeout
        && left.max_concurrent_requests == right.max_concurrent_requests
        && left.short_prefix == right.short_prefix
        && left.allow_sampling == right.allow_sampling
        && left.allow_elicitation == right.allow_elicitation
}

impl PartialEq for MCPServer {
    fn eq(&self, other: &Self) -> bool {
        same_identity(self, other)
            && same_catalog(self, other)
            && same_oauth(self, other)
            && same_credentials(self, other)
            && same_stdio(self, other)
            && same_flags(self, other)
            && same_runtime(self, other)
    }
}
