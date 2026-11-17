use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::server_utils::env_vars::{
    MCPMissingUserEnvVarsError, build_env_var_setup_url, collect_env_var_references,
    find_env_var_references, interpolate_env_vars, interpolate_headers, parse_admin_env_vars,
};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde::Deserialize;
use serde_json::{Value, json};

use super::stored_json;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EnvsInput {
    ParseAdminEnvVars(AdminEnvInput),
    EnvReferences(ReferencesInput),
    Interpolate(InterpolateInput),
    SetupUrl(SetupUrlInput),
    MissingVarsMessage(MissingVarsInput),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdminEnvInput {
    #[serde(default)]
    pub env_vars: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferencesInput {
    pub value: String,
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InterpolateInput {
    pub value: String,
    pub headers: BTreeMap<String, String>,
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetupUrlInput {
    pub server_id: String,
    #[serde(default)]
    pub proxy_base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MissingVarsInput {
    pub server_id: String,
    #[serde(default)]
    pub server_name: Option<String>,
    pub missing: Vec<String>,
    pub setup_url: String,
}

pub(crate) fn run(input: &EnvsInput) -> HelperResult {
    match input {
        EnvsInput::ParseAdminEnvVars(fields) => admin_env(fields),
        EnvsInput::EnvReferences(fields) => Ok(references(fields)),
        EnvsInput::Interpolate(fields) => Ok(interpolate(fields)),
        EnvsInput::SetupUrl(fields) => Ok(json!(build_env_var_setup_url(
            fields.proxy_base_url.as_deref(),
            &fields.server_id
        ))),
        EnvsInput::MissingVarsMessage(fields) => Ok(missing_message(fields)),
    }
}

fn admin_env(fields: &AdminEnvInput) -> HelperResult {
    let entries: Option<Vec<JsonValue>> = fields
        .env_vars
        .as_ref()
        .map(|values| values.iter().map(stored_json).collect())
        .transpose()?;
    let (globals, users) = parse_admin_env_vars(entries.as_deref());
    let user_values: Vec<Value> = users
        .iter()
        .map(|spec| json!({"name": &spec.name, "description": &spec.description}))
        .collect();
    Ok(json!({"globals": globals, "users": user_values}))
}

fn references(fields: &ReferencesInput) -> Value {
    let found = find_env_var_references(&fields.value);
    let collected = collect_env_var_references(fields.strings.iter().map(String::as_str));
    json!({"references": found, "collected": collected})
}

fn interpolate(fields: &InterpolateInput) -> Value {
    json!({
        "value": interpolate_env_vars(&fields.value, &fields.variables),
        "headers": interpolate_headers(&fields.headers, &fields.variables),
    })
}

fn missing_message(fields: &MissingVarsInput) -> Value {
    let error = MCPMissingUserEnvVarsError {
        server_id: fields.server_id.clone(),
        server_name: fields.server_name.clone(),
        missing: fields.missing.clone(),
        setup_url: fields.setup_url.clone(),
    };
    json!(error.to_string())
}
