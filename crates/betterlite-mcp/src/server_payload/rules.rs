use std::collections::BTreeSet;

use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPCredentials};
use secrecy::ExposeSecret as _;

use crate::server_payload::PayloadError;
use crate::server_payload::repr;
use crate::server_payload::request::NewMCPServerRequest;
use crate::server_record::OAuth2Flow;

mod fields;
mod tests;

pub const MCP_STDIO_DISABLED_MESSAGE: &str = "stdio MCP servers are disabled on this proxy. Set LITELLM_ENABLE_MCP_STDIO=true on the proxy and restart to enable them";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadRules {
    pub prefix_separator: String,
    pub stdio: StdioPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioPolicy {
    pub enabled: bool,
    pub allowed_commands: BTreeSet<String>,
}

#[must_use]
pub fn is_mcp_stdio_blocked(transport: Option<&str>, policy: &StdioPolicy) -> bool {
    if transport != Some("stdio") {
        return false;
    }
    !policy.enabled
}

pub fn warn_if_mcp_stdio_blocked(
    server_name: Option<&str>,
    transport: Option<&str>,
    policy: &StdioPolicy,
) {
    if !is_mcp_stdio_blocked(transport, policy) {
        return;
    }
    let name = server_name.unwrap_or("None");
    tracing::warn!("MCP server '{name}' will not start: {MCP_STDIO_DISABLED_MESSAGE}");
}

pub fn stamp_omitted_oauth2_flow(payload: &mut NewMCPServerRequest) {
    if payload.auth_type != Some(MCPAuth::Oauth2) || payload.oauth2_flow.is_some() {
        return;
    }
    payload.oauth2_flow = Some(machine_to_machine_flow(payload));
}

fn machine_to_machine_flow(payload: &NewMCPServerRequest) -> OAuth2Flow {
    let token_url = payload
        .token_url
        .as_deref()
        .is_some_and(|url| !url.is_empty());
    let authorization_url = payload
        .authorization_url
        .as_deref()
        .is_some_and(|url| !url.is_empty());
    let machine_to_machine = token_url
        && payload
            .credentials
            .as_ref()
            .is_some_and(credentials_have_client_id)
        && payload
            .credentials
            .as_ref()
            .is_some_and(credentials_have_client_secret)
        && !authorization_url;
    if machine_to_machine {
        OAuth2Flow::ClientCredentials
    } else {
        OAuth2Flow::AuthorizationCode
    }
}

fn credentials_have_client_id(credentials: &MCPCredentials) -> bool {
    let Some(client_id) = credentials
        .client_id
        .as_ref()
        .and_then(|value| value.as_deref())
    else {
        return false;
    };
    if client_id.is_empty() {
        return false;
    }
    true
}

fn credentials_have_client_secret(credentials: &MCPCredentials) -> bool {
    let Some(secret) = credentials
        .client_secret
        .as_ref()
        .and_then(|value| value.as_ref())
    else {
        return false;
    };
    if secret.0.expose_secret().is_empty() {
        return false;
    }
    true
}

// ast-grep-ignore: no-long-names
pub fn validate_required_fields(
    payload: &NewMCPServerRequest,
    required: &[String],
) -> Result<(), PayloadError> {
    if required.is_empty() {
        return Ok(());
    }
    let mut parsed = Vec::with_capacity(required.len());
    let mut unknown = Vec::new();
    for name in required {
        match <fields::NewServerField as std::str::FromStr>::from_str(name) {
            Ok(field) => parsed.push((name, field)),
            Err(_) => unknown.push(name.clone()),
        }
    }
    if !unknown.is_empty() {
        return Err(PayloadError::UnknownRequiredFields {
            unknown: repr::list(&unknown),
        });
    }
    let missing = parsed
        .iter()
        .filter(|(_, field)| !fields::present(payload, *field))
        .map(|(name, _)| (*name).clone())
        .collect::<Vec<String>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(PayloadError::MissingRequiredFields {
            missing: repr::list(&missing),
        })
    }
}

#[must_use]
pub fn identifier_conflict_message(conflict: &McpIdentifierConflict) -> String {
    let field = conflict.field.as_str();
    let value = &conflict.value;
    let server_id = &conflict.server_id;
    format!(
        "An MCP server with {field} '{value}' already exists (server_id={server_id}). MCP server names and aliases must be unique, case-insensitive."
    )
}

// ast-grep-ignore: no-long-names
pub fn warn_if_id_jag_server_outruns_sso(
    server_id: Option<&str>,
    auth_type: Option<&str>,
    gap: Option<&str>,
) {
    if auth_type != Some("oauth2_id_jag") {
        return;
    }
    let Some(gap) = gap else {
        return;
    };
    let server = server_id.unwrap_or("None");
    tracing::warn!("MCP server {server} is registered with auth_type=oauth2_id_jag, but {gap}.");
}

#[must_use]
pub fn does_mcp_server_exist(server_ids: impl IntoIterator<Item = &'_ str>, id: &str) -> bool {
    server_ids.into_iter().any(|candidate| candidate == id)
}
