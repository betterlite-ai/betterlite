use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::pinned::{MCPInfo, PinnedError};
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPCredentials};
use betterlite_mcp::registry::public_servers::PublicServers as _;
use betterlite_mcp::registry::servers::ServerRegistry;
use thiserror::Error;

mod tests;

// ast-grep-ignore: no-long-names - LiteLLM names this constant, and B4717 reads it under the same name
pub const MCP_ADMIN_CONFIG_CREDENTIAL_KEYS: [&str; 2] =
    ["upstream_resource", "upstream_token_header"];

#[derive(Debug, Error)]
pub enum CredentialViewError {
    #[error("the stored MCP info is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the stored MCP info is not valid metadata")]
    Pinned(#[source] PinnedError),
}

pub fn redact_mcp_credentials(
    server: &McpServerTable,
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<McpServerTable, CredentialViewError> {
    let mut redacted = server.clone();
    redacted.credentials = server
        .credentials
        .as_ref()
        .and_then(admin_config_credentials);
    redacted.mcp_info = Some(mcp_info_with_publicity(server, registry, public_ids)?);
    Ok(redacted)
}

fn admin_config_credentials(stored: &MCPCredentials) -> Option<MCPCredentials> {
    let upstream_resource = kept_string(stored.upstream_resource.as_ref());
    let upstream_token_header = kept_string(stored.upstream_token_header.as_ref());
    let scopes = kept_scopes(stored.scopes.as_ref());
    if upstream_resource.is_none() && upstream_token_header.is_none() && scopes.is_none() {
        return None;
    }
    Some(MCPCredentials {
        auth_value: None,
        client_id: None,
        client_secret: None,
        scopes: scopes.map(Some),
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_region_name: None,
        aws_service_name: None,
        aws_role_name: None,
        aws_session_name: None,
        audience: None,
        token_exchange_endpoint: None,
        subject_token_type: None,
        id_jag_resource_token_endpoint: None,
        id_jag_resource: None,
        upstream_resource: upstream_resource.map(Some),
        upstream_token_header: upstream_token_header.map(Some),
        client_private_key: None,
        client_private_key_id: None,
        client_assertion_signing_alg: None,
        token_endpoint_auth_method: None,
        redirect_uris: None,
        token_exchange_profile: None,
    })
}

// a present null and an empty string both read as absent, so the admin form shows nothing to clear
fn kept_string(value: Option<&Option<String>>) -> Option<String> {
    match value {
        Some(Some(text)) if !text.is_empty() => Some(text.clone()),
        Some(Some(_) | None) | None => None,
    }
}

fn kept_scopes(value: Option<&Option<Vec<String>>>) -> Option<Vec<String>> {
    match value {
        Some(Some(items)) if !items.is_empty() && items.iter().all(|scope| !scope.is_empty()) => {
            Some(items.clone())
        }
        Some(Some(_) | None) | None => None,
    }
}

fn mcp_info_with_publicity(
    server: &McpServerTable,
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<MCPInfo, CredentialViewError> {
    let mut info: BTreeMap<String, JsonValue> = match &server.mcp_info {
        Some(stored) => {
            let encoded = serde_json::to_string(stored).map_err(CredentialViewError::Json)?;
            serde_json::from_str(&encoded).map_err(CredentialViewError::Json)?
        }
        None => BTreeMap::new(),
    };
    let is_public = registry.is_server_public(&server.server_id, Some(public_ids));
    info.insert(String::from("is_public"), JsonValue::Bool(is_public));
    let is_public_explicit = is_public && public_ids.contains(&server.server_id);
    info.insert(
        String::from("is_public_explicit"),
        JsonValue::Bool(is_public_explicit),
    );
    MCPInfo::try_from(info).map_err(CredentialViewError::Pinned)
}
