use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::vector_index_admin::auth_is_proxy_admin;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::MCPCredentials;
use betterlite_mcp::registry::lookup::ServerLookups as _;
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::server_payload::request::NewMCPServerRequest;
use betterlite_models::decisions::Json;

use self::credentials::{
    inherited_credentials, method_supplied, secret_supplied, text_supplied, texts_supplied,
};
use self::sanitize_fields::{non_admin_view, virtual_key_view};
use crate::mcp::admin_credential_view::{CredentialViewError, redact_mcp_credentials};

mod credentials;
mod sanitize_fields;
mod tests;

pub const ADMIN_REQUIRED: &str =
    "Proxy admin access required to revoke another user's MCP credential.";
pub const MISSING_USER_ID: &str = "User ID not found in token";

// LiteLLM's (server attribute, credentials key) pairs a session server inherits from the server it derives from
pub const INHERITED_CREDENTIAL_FIELDS: [(&str, &str); 11] = [
    ("authentication_token", "auth_value"),
    ("client_id", "client_id"),
    ("client_secret", "client_secret"),
    ("scopes", "scopes"),
    ("aws_access_key_id", "aws_access_key_id"),
    ("aws_secret_access_key", "aws_secret_access_key"),
    ("aws_session_token", "aws_session_token"),
    ("aws_region_name", "aws_region_name"),
    ("aws_service_name", "aws_service_name"),
    ("upstream_resource", "upstream_resource"),
    ("upstream_token_header", "upstream_token_header"),
];

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialTargetError {
    #[error("{ADMIN_REQUIRED}")]
    AdminRequired,
    #[error("{MISSING_USER_ID}")]
    MissingUserId,
}

impl CredentialTargetError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::AdminRequired => StatusCode::FORBIDDEN,
            Self::MissingUserId => StatusCode::BAD_REQUEST,
        }
    }

    // the HTTPException detail LiteLLM raises: {"error": message}
    #[must_use]
    pub fn detail(&self) -> BTreeMap<String, Json> {
        let message = match self {
            Self::AdminRequired => ADMIN_REQUIRED,
            Self::MissingUserId => MISSING_USER_ID,
        };
        BTreeMap::from([("error".to_owned(), Json::Text(message.to_owned()))])
    }
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - LiteLLM's named predicate, which B4723 calls; it is B10071's proxy-admin check
pub const fn user_is_full_admin(auth: &UserAPIKeyAuth) -> bool {
    auth_is_proxy_admin(auth)
}

// ast-grep-ignore: no-long-names - LiteLLM's name for the credential target guard, and B4723 calls it by this name
pub fn resolve_credential_target_user_id(
    auth: &UserAPIKeyAuth,
    requested_user_id: Option<&str>,
) -> Result<String, CredentialTargetError> {
    let caller_user_id = auth.request_quota_view().user_id.unwrap_or("");
    if let Some(requested) = requested_user_id
        && requested != caller_user_id
    {
        if !user_is_full_admin(auth) {
            return Err(CredentialTargetError::AdminRequired);
        }
        return Ok(requested.to_owned());
    }
    if caller_user_id.is_empty() {
        return Err(CredentialTargetError::MissingUserId);
    }
    Ok(caller_user_id.to_owned())
}

// the three list forms differ only in the per-server view, so one loop carries the mapping
fn mapped<T, F>(
    servers: &[McpServerTable],
    registry: &ServerRegistry,
    public_ids: &[String],
    view: F,
) -> Result<Vec<T>, CredentialViewError>
where
    F: Fn(&McpServerTable, &ServerRegistry, &[String]) -> Result<T, CredentialViewError>,
{
    let mut views = Vec::with_capacity(servers.len());
    for server in servers {
        views.push(view(server, registry, public_ids)?);
    }
    Ok(views)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's list redaction name, kept for the MCP management routes
pub fn redact_mcp_credentials_list(
    servers: &[McpServerTable],
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<Vec<McpServerTable>, CredentialViewError> {
    mapped(servers, registry, public_ids, redact_mcp_credentials)
}

// ast-grep-ignore: no-long-names, no-one-line-functions - LiteLLM's sanitizer name, kept for the MCP management routes; the view is shared with the list form
pub fn sanitize_mcp_server_for_non_admin(
    server: &McpServerTable,
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<McpServerTable, CredentialViewError> {
    non_admin_view(server, registry, public_ids)
}

// ast-grep-ignore: no-long-names, no-one-line-functions - LiteLLM's sanitizer name, kept for the MCP management routes
pub fn sanitize_mcp_server_list_for_non_admin(
    servers: &[McpServerTable],
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<Vec<McpServerTable>, CredentialViewError> {
    mapped(servers, registry, public_ids, non_admin_view)
}

// ast-grep-ignore: no-long-names, no-one-line-functions - LiteLLM's sanitizer name, kept for the MCP management routes; the view is shared with the list form
pub fn sanitize_mcp_server_for_virtual_key(
    server: &McpServerTable,
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<McpServerTable, CredentialViewError> {
    virtual_key_view(server, registry, public_ids)
}

// ast-grep-ignore: no-long-names, no-one-line-functions - LiteLLM's sanitizer name, kept for the MCP management routes
pub fn sanitize_mcp_server_list_for_virtual_key(
    servers: &[McpServerTable],
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<Vec<McpServerTable>, CredentialViewError> {
    mapped(servers, registry, public_ids, virtual_key_view)
}

#[must_use]
// ast-grep-ignore: no-long-names - LiteLLM's name for the guard that keeps admin config from reading as a supplied credential
pub fn has_non_admin_config_credentials(credentials: Option<&MCPCredentials>) -> bool {
    let Some(credentials) = credentials else {
        return false;
    };
    secret_supplied(credentials.auth_value.as_ref())
        || text_supplied(credentials.client_id.as_ref())
        || secret_supplied(credentials.client_secret.as_ref())
        || secret_supplied(credentials.aws_access_key_id.as_ref())
        || secret_supplied(credentials.aws_secret_access_key.as_ref())
        || secret_supplied(credentials.aws_session_token.as_ref())
        || text_supplied(credentials.aws_region_name.as_ref())
        || text_supplied(credentials.aws_service_name.as_ref())
        || text_supplied(credentials.aws_role_name.as_ref())
        || text_supplied(credentials.aws_session_name.as_ref())
        || text_supplied(credentials.audience.as_ref())
        || text_supplied(credentials.token_exchange_endpoint.as_ref())
        || text_supplied(credentials.subject_token_type.as_ref())
        || text_supplied(credentials.id_jag_resource_token_endpoint.as_ref())
        || text_supplied(credentials.id_jag_resource.as_ref())
        || secret_supplied(credentials.client_private_key.as_ref())
        || text_supplied(credentials.client_private_key_id.as_ref())
        || text_supplied(credentials.client_assertion_signing_alg.as_ref())
        || method_supplied(credentials.token_endpoint_auth_method.as_ref())
        || texts_supplied(credentials.redirect_uris.as_ref())
        || text_supplied(credentials.token_exchange_profile.as_ref())
}

#[must_use]
// ast-grep-ignore: no-long-names - LiteLLM's name for the session-server inheritance step, and B4718 calls it by this name
pub fn inherit_credentials_from_existing_server(
    payload: &NewMCPServerRequest,
    registry: &ServerRegistry,
) -> NewMCPServerRequest {
    let Some(server_id) = payload.server_id.as_deref() else {
        return payload.clone();
    };
    // ast-grep-ignore: no-long-names - LiteLLM's guard name, and its definition carries the same ignore
    if server_id.is_empty() || has_non_admin_config_credentials(payload.credentials.as_ref()) {
        return payload.clone();
    }
    let Some(server) = registry.server_by_id(server_id, None) else {
        return payload.clone();
    };
    let Some(merged) = inherited_credentials(&server.fields, payload.credentials.as_ref()) else {
        return payload.clone();
    };
    let mut inherited_payload = payload.clone();
    inherited_payload.credentials = Some(merged);
    inherited_payload
}
