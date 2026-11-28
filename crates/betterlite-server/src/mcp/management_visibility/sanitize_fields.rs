use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::managed_resource_rows::wire::SecretMapField;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_mcp::registry::servers::ServerRegistry;

use crate::mcp::admin_credential_view::{CredentialViewError, redact_mcp_credentials};

// the URL, header, env and auth fields LiteLLM drops from both the non-admin and the virtual-key view
pub(super) fn strip_shared_secret_fields(server: &mut McpServerTable) {
    server.credentials = None;
    server.pinned_tools = None;
    server.url = None;
    server.static_headers = None;
    server.extra_headers = Vec::new();
    server.env = SecretMapField::default();
    server.command = None;
    server.args = Vec::new();
    server.issuer = None;
    server.authorization_url = None;
    server.token_url = None;
    server.registration_url = None;
    server.token_exchange_endpoint = None;
    server.audience = None;
    server.subject_token_type = None;
    server.token_exchange_profile = None;
    server.env_vars = None;
}

// _sanitize_mcp_server_for_non_admin: the shared strip, and the spec path, which only a non-admin loses
pub(super) fn non_admin_view(
    server: &McpServerTable,
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<McpServerTable, CredentialViewError> {
    let mut sanitized = redact_mcp_credentials(server, registry, public_ids)?;
    strip_shared_secret_fields(&mut sanitized);
    sanitized.spec_path = None;
    Ok(sanitized)
}

// _sanitize_mcp_server_for_virtual_key: the shared strip plus the access metadata no key should see
pub(super) fn virtual_key_view(
    server: &McpServerTable,
    registry: &ServerRegistry,
    public_ids: &[String],
) -> Result<McpServerTable, CredentialViewError> {
    let mut sanitized = redact_mcp_credentials(server, registry, public_ids)?;
    strip_shared_secret_fields(&mut sanitized);
    sanitized.allowed_tools = Vec::new();
    sanitized.mcp_access_groups = Vec::new();
    sanitized.teams = Vec::new();
    sanitized.health_check_error = None;
    sanitized.last_health_check = None;
    sanitized.created_by = None;
    sanitized.updated_by = None;
    sanitized.created_at = None;
    sanitized.updated_at = None;
    reduce_publicity(&mut sanitized)?;
    Ok(sanitized)
}

// LiteLLM keeps only the two publicity booleans of a virtual key's mcp_info, reading them back from the redacted view
pub(super) fn reduce_publicity(server: &mut McpServerTable) -> Result<(), CredentialViewError> {
    let fields = info_fields(server)?;
    let mut reduced = BTreeMap::new();
    reduced.insert(
        String::from("is_public"),
        JsonValue::Bool(is_true(&fields, "is_public")),
    );
    reduced.insert(
        String::from("is_public_explicit"),
        JsonValue::Bool(is_true(&fields, "is_public_explicit")),
    );
    server.mcp_info = Some(MCPInfo::try_from(reduced).map_err(CredentialViewError::Pinned)?);
    Ok(())
}

fn info_fields(
    server: &McpServerTable,
) -> Result<BTreeMap<String, JsonValue>, CredentialViewError> {
    let Some(info) = &server.mcp_info else {
        return Ok(BTreeMap::new());
    };
    let encoded = serde_json::to_string(info).map_err(CredentialViewError::Json)?;
    serde_json::from_str(&encoded).map_err(CredentialViewError::Json)
}

// LiteLLM's `is True`: a missing key and any non-boolean value both read as false
fn is_true(fields: &BTreeMap<String, JsonValue>, key: &str) -> bool {
    match fields.get(key) {
        Some(JsonValue::Bool(flag)) => *flag,
        Some(_) | None => false,
    }
}
