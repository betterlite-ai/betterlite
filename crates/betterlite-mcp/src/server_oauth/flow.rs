use std::borrow::Cow;

use betterlite_db::mcp_stored_contracts::MCPAuth;
use secrecy::{ExposeSecret as _, SecretString};

use super::basics::blank_to_none;
use crate::server_record::{MCPServer, OAuth2Flow};

#[derive(Clone, Copy)]
pub struct FlowResolution<'fields> {
    pub auth_type: Option<MCPAuth>,
    pub oauth2_flow: Option<OAuth2Flow>,
    pub token_url: Option<&'fields str>,
    pub authorization_url: Option<&'fields str>,
    pub client_id: Option<&'fields str>,
    pub client_secret: Option<&'fields SecretString>,
}

#[must_use]
pub fn explicit_oauth2_flow(oauth2_flow: Option<&str>) -> Option<OAuth2Flow> {
    match oauth2_flow {
        Some("client_credentials") => Some(OAuth2Flow::ClientCredentials),
        Some("authorization_code") => Some(OAuth2Flow::AuthorizationCode),
        Some(_) | None => None,
    }
}

#[must_use]
pub fn resolve_oauth2_flow(inputs: FlowResolution<'_>) -> Option<OAuth2Flow> {
    if let Some(flow) = inputs.oauth2_flow {
        return Some(flow);
    }
    if inputs.auth_type != Some(MCPAuth::Oauth2) {
        return None;
    }
    if blank_to_none(inputs.authorization_url).is_some() {
        return None;
    }
    let has_secret = inputs
        .client_secret
        .is_some_and(|secret| !secret.expose_secret().is_empty());
    if blank_to_none(inputs.token_url).is_some()
        && blank_to_none(inputs.client_id).is_some()
        && has_secret
    {
        return Some(OAuth2Flow::ClientCredentials);
    }
    None
}

#[must_use]
pub fn effective_oauth2_flow(server: &MCPServer) -> Option<OAuth2Flow> {
    resolve_oauth2_flow(FlowResolution {
        auth_type: server.fields.auth_type,
        oauth2_flow: server.fields.oauth2_flow,
        token_url: server.fields.token_url.as_deref(),
        authorization_url: server.fields.authorization_url.as_deref(),
        client_id: server.fields.client_id.as_deref(),
        client_secret: server
            .fields
            .client_secret
            .as_ref()
            .map(|credential| &credential.0.0),
    })
}

#[must_use]
pub fn flow_for_request(server: &MCPServer) -> Cow<'_, MCPServer> {
    let Some(effective) = effective_oauth2_flow(server) else {
        return Cow::Borrowed(server);
    };
    if server.fields.oauth2_flow == Some(effective) {
        return Cow::Borrowed(server);
    }
    let inferred = match effective {
        OAuth2Flow::ClientCredentials => "client_credentials",
        OAuth2Flow::AuthorizationCode => "authorization_code",
    };
    tracing::warn!(
        server_id = %server.fields.server_id,
        "MCP server has no persisted oauth2_flow but matches the {inferred} shape; using the inferred flow for this request. Set oauth2_flow explicitly: client_credentials for M2M, or authorization_code after an interactive sign-in."
    );
    let mut resolved = server.clone();
    resolved.fields.oauth2_flow = Some(effective);
    Cow::Owned(resolved)
}
