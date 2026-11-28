use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_user_env_vars::store::UserEnvVars;
use betterlite_mcp::server_utils::env_vars::{
    UserEnvSpec, build_env_var_setup_url, collect_env_var_references, parse_admin_env_vars,
};
use secrecy::ExposeSecret;
use thiserror::Error;

use super::types::{MCPUserEnvVarSpec, MCPUserEnvVarsStatus};

mod tests;

#[derive(Debug, Error)]
pub(crate) enum StatusError {
    #[error("the server env vars could not be read as JSON")]
    Encode(#[source] serde_json::Error),
}

// LiteLLM's _compute_user_env_var_status: only a referenced variable without a non-empty global blocks
pub(crate) fn compute_user_env_status(
    server: &McpServerTable,
    stored_values: &UserEnvVars,
    proxy_base_url: Option<&str>,
) -> Result<MCPUserEnvVarsStatus, StatusError> {
    let (mut global_values, user_specs) = parse_server_env_vars(server)?;
    // An empty global is not a usable fallback, so it must not cover a referenced user variable
    global_values.retain(|_, value| !value.is_empty());
    let referenced = referenced_names(server);
    let blocking: BTreeSet<&str> = user_specs
        .iter()
        .map(|spec| spec.name.as_str())
        .filter(|name| referenced.contains(*name))
        .filter(|name| !global_values.contains_key(*name))
        .collect();
    let (required, missing_count) = required_specs(&user_specs, &blocking, stored_values);
    let setup_url =
        (!required.is_empty()).then(|| build_env_var_setup_url(proxy_base_url, &server.server_id));
    Ok(MCPUserEnvVarsStatus {
        server_id: server.server_id.clone(),
        server_name: server.server_name.clone(),
        alias: server.alias.clone(),
        required,
        missing_count,
        setup_url,
    })
}

// The variables any static header value references, as LiteLLM's get_referenced_variables reads them
fn referenced_names(server: &McpServerTable) -> BTreeSet<String> {
    collect_env_var_references(
        server
            .static_headers
            .iter()
            .flat_map(|headers| headers.0.values())
            .map(ExposeSecret::expose_secret),
    )
}

// LiteLLM walks the declared user specs in order and reports only the blocking ones, set or not
fn required_specs(
    user_specs: &[UserEnvSpec],
    blocking: &BTreeSet<&str>,
    stored_values: &UserEnvVars,
) -> (Vec<MCPUserEnvVarSpec>, u64) {
    let mut required = Vec::new();
    let mut missing_count: u64 = 0;
    for spec in user_specs {
        if !blocking.contains(spec.name.as_str()) {
            continue;
        }
        let is_set = stored_values
            .get(&spec.name)
            .is_some_and(|value| !value.expose_secret().is_empty());
        if !is_set {
            missing_count = missing_count.saturating_add(1);
        }
        required.push(MCPUserEnvVarSpec {
            name: spec.name.clone(),
            description: spec.description.clone(),
            is_set,
        });
    }
    (required, missing_count)
}

// The typed record is handed to B6594's parser in the JSON form it reads, as LiteLLM passes its rows
pub(crate) fn parse_server_env_vars(
    server: &McpServerTable,
) -> Result<(BTreeMap<String, String>, Vec<UserEnvSpec>), StatusError> {
    let Some(declared) = server.env_vars.as_deref() else {
        return Ok((BTreeMap::new(), Vec::new()));
    };
    let rows: Vec<JsonValue> = serde_json::to_value(declared)
        .map_err(StatusError::Encode)
        .and_then(|tree| {
            serde_json::from_value::<Vec<JsonValue>>(tree).map_err(StatusError::Encode)
        })?;
    Ok(parse_admin_env_vars(Some(&rows)))
}
