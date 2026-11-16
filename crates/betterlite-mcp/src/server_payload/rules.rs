use crate::server_payload::request::NewMCPServerRequest;
use crate::server_payload::{PayloadError, quoted_list};
use crate::server_record::OAuth2Flow;
use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp_stored_contracts::MCPAuth;
use secrecy::ExposeSecret as _;
mod fields;
mod tests;
pub const MCP_STDIO_DISABLED_MESSAGE: &str = "stdio MCP servers are disabled on this proxy. Set LITELLM_ENABLE_MCP_STDIO=true on the proxy and restart to enable them";
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
    let credentials = payload.credentials.as_ref();
    let client_id = credentials
        .and_then(|stored| stored.client_id.as_ref())
        .and_then(Option::as_ref);
    let client_secret = credentials
        .and_then(|stored| stored.client_secret.as_ref())
        .and_then(Option::as_ref);
    let token_url = payload.token_url.as_deref();
    let authorization_url = payload.authorization_url.as_deref();
    let urls =
        token_url.is_some_and(|url| !url.is_empty()) && authorization_url.is_none_or(str::is_empty);
    let machine = urls
        && client_id.is_some_and(|id| !id.is_empty())
        && client_secret.is_some_and(|secret| !secret.0.expose_secret().is_empty());
    if machine {
        OAuth2Flow::ClientCredentials
    } else {
        OAuth2Flow::AuthorizationCode
    }
}
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
        match name.parse::<fields::NewServerField>() {
            Ok(field) => parsed.push((name, field)),
            Err(_) => unknown.push(name),
        }
    }
    if !unknown.is_empty() {
        return Err(PayloadError::UnknownRequiredFields {
            unknown: quoted_list(unknown),
        });
    }
    let mut missing = Vec::new();
    for (name, field) in parsed {
        if !fields::present(payload, field) {
            missing.push(name);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(PayloadError::MissingRequiredFields {
        missing: quoted_list(missing),
    })
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
    if let Some(reason) = gap {
        let server = server_id.unwrap_or("None");
        tracing::warn!(
            "MCP server {server} is registered with auth_type=oauth2_id_jag, but {reason}."
        );
    }
}
#[must_use]
pub fn does_mcp_server_exist<I>(server_ids: I, id: &str) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    for candidate in server_ids {
        if candidate.as_ref() == id {
            return true;
        }
    }
    false
}
