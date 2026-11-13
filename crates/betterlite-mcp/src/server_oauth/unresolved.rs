use betterlite_db::mcp_stored_contracts::MCPAuth;

use super::basics::{UPSTREAM_DISCOVERY_AUTH_TYPES, blank_to_none, has_discovery_source};
use super::flow::effective_oauth2_flow;
use crate::server_record::{MCPServer, OAuth2Flow};

#[must_use]
pub fn flow_endpoints_missing(
    auth_type: Option<MCPAuth>,
    oauth2_flow: Option<OAuth2Flow>,
    authorization_url: Option<&str>,
    token_url: Option<&str>,
    token_exchange_endpoint: Option<&str>,
) -> bool {
    if auth_type == Some(MCPAuth::Oauth2TokenExchange) {
        return token_exchange_endpoint.is_none() && token_url.is_none();
    }
    if !auth_type.is_some_and(|auth| UPSTREAM_DISCOVERY_AUTH_TYPES.contains(&auth)) {
        return false;
    }
    if oauth2_flow == Some(OAuth2Flow::ClientCredentials) {
        return token_url.is_none();
    }
    authorization_url.is_none() || token_url.is_none()
}

#[must_use]
pub fn endpoints_unresolved(server: &MCPServer) -> bool {
    if server.fields.auth_type == Some(MCPAuth::Oauth2TokenExchange)
        && server.fields.token_exchange_profile == "entra_obo"
        && server.fields.scopes.as_ref().is_none_or(Vec::is_empty)
    {
        return true;
    }
    if server.is_dcr_bridge()
        && blank_to_none(server.fields.client_id.as_deref()).is_none()
        && server.effective_registration_url().is_none()
    {
        return true;
    }
    flow_endpoints_missing(
        server.fields.auth_type,
        effective_oauth2_flow(server),
        server.effective_authorization_url(),
        server.effective_token_url(),
        server.fields.token_exchange_endpoint.as_deref(),
    )
}

#[must_use]
pub fn requires_discovery(
    server_url: Option<&str>,
    use_issuer_anchor: bool,
    server: &MCPServer,
) -> bool {
    if !has_discovery_source(server_url, use_issuer_anchor) {
        return false;
    }
    endpoints_unresolved(server)
}
