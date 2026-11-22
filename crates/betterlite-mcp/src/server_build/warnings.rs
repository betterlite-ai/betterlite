use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::MCPAuth;

use crate::server_payload::names::validate_tool_name;
use crate::server_record::MCPServer;
use crate::server_utils::prefixes::{PrefixPolicy, get_server_prefix};

pub fn warn_on_server_name_fields(server_id: &str, alias: Option<&str>, server_name: Option<&str>) {
    warn_about_field(server_id, "alias", alias);
    warn_about_field(server_id, "server_name", server_name);
}

fn warn_about_field(server_id: &str, field_name: &str, value: Option<&str>) {
    let Some(text) = value.filter(|candidate| !candidate.is_empty()) else {
        return;
    };
    let result = validate_tool_name(text);
    if result.is_valid {
        return;
    }
    let warning_text = if result.warnings.is_empty() {
        String::from("Validation failed")
    } else {
        result.warnings.join("; ")
    };
    tracing::warn!("MCP server '{server_id}' has invalid {field_name} '{text}': {warning_text}");
}

// ast-grep-ignore: no-long-names
pub fn warn_on_shared_identifier_prefixes(servers: &[MCPServer]) {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for server in servers {
        let alias = server
            .fields
            .alias
            .as_deref()
            .filter(|text| !text.is_empty());
        let server_name = server
            .fields
            .server_name
            .as_deref()
            .filter(|text| !text.is_empty());
        let identifier = match alias.or(server_name) {
            Some(value) => value.to_lowercase(),
            None => continue,
        };
        groups
            .entry(identifier)
            .or_default()
            .push(server.fields.server_id.clone());
    }
    for (identifier, mut server_ids) in groups {
        if server_ids.len() > 1 {
            server_ids.sort();
            tracing::warn!(
                "MCP servers {} share the identifier '{}'; tool routing for that prefix is ambiguous. Rename or delete all but one.",
                list_repr(&server_ids),
                identifier
            );
        }
    }
}

fn list_repr(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("'{value}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ast-grep-ignore: no-long-names
pub fn warn_legacy_delegate_auth_if_applicable(
    server: &MCPServer,
    source: &str,
    policy: &PrefixPolicy,
) {
    if server.fields.auth_type != Some(MCPAuth::Oauth2) {
        return;
    }
    if !server.fields.delegate_auth_to_upstream {
        return;
    }
    if server.has_client_credentials() {
        return;
    }
    let label = get_server_prefix(server, policy);
    tracing::warn!(
        "MCP server '{label}' (id={}, source={source}) uses deprecated auth_type=oauth2 with delegate_auth_to_upstream=true. LiteLLM admission is now required; migrate to auth_type=oauth_delegate for client-forwarded OAuth.",
        server.fields.server_id
    );
}

// ast-grep-ignore: no-long-names
pub fn warn_config_id_jag_server_outruns_sso(
    server: &MCPServer,
    gap: Option<&str>,
    policy: &PrefixPolicy,
) {
    if server.fields.auth_type != Some(MCPAuth::Oauth2IdJag) {
        return;
    }
    let Some(detail) = gap else {
        return;
    };
    let label = get_server_prefix(server, policy);
    tracing::warn!(
        "MCP server '{label}' (id={}, source=config) is declared with auth_type=oauth2_id_jag, but {detail}.",
        server.fields.server_id
    );
}
