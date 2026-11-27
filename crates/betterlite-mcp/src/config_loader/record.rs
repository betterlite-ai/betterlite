use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::pinned::{MCPInfo, parse_pinned_tools};
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPTransport, PinnedMCPTool};
use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::config::{McpServerConfig, ScopesConfig};
use super::error::ConfigError;
use super::values::stored_json;
use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;
use crate::server_build::json::normalize_cost_info;
use crate::server_oauth::merge::{ScopesInput, extract_scopes};
use crate::server_record::{
    MCPOAuthMetadata, MCPServer, OAuth2Flow, SecretCredential, ServerFields, ServerFlag,
    ServerRecordError,
};

// the record's own defaults for the three fields serde requires; every other field is set below
const RECORD_SEED: &str = r#"{"server_id":"","name":"","transport":"http"}"#;

pub(super) struct ResolvedOAuth {
    pub flow: Option<OAuth2Flow>,
    pub scopes: Option<Vec<String>>,
    pub configured_scopes: Option<Vec<String>>,
    pub issuer: Option<String>,
    pub issuer_is_anchored: ServerFlag,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub configured_authorization_url: Option<String>,
    pub configured_token_url: Option<String>,
    pub configured_registration_url: Option<String>,
    pub metadata: Option<MCPOAuthMetadata>,
}

pub(super) struct RecordInputs<'record> {
    pub config: &'record McpServerConfig,
    pub server_name: &'record str,
    pub server_url: &'record str,
    pub server_id: &'record str,
    pub name: &'record str,
    pub alias: Option<&'record str>,
    pub mcp_info: MCPInfo,
    pub oauth: ResolvedOAuth,
    pub per_server_oauth_discovery: bool,
    pub dcr_bridge: Option<bool>,
}

pub(super) fn build_record(inputs: RecordInputs<'_>) -> Result<MCPServer, ConfigError> {
    let mut fields: ServerFields =
        serde_json::from_str(RECORD_SEED).map_err(ServerRecordError::from)?;
    apply_identity(&mut fields, &inputs);
    apply_catalog(
        &mut fields,
        inputs.config,
        inputs.server_name,
        inputs.mcp_info,
    )?;
    apply_oauth(&mut fields, inputs.oauth);
    apply_exchange(&mut fields, inputs.config);
    apply_switches(
        &mut fields,
        inputs.config,
        inputs.server_name,
        inputs.per_server_oauth_discovery,
        inputs.dcr_bridge,
    )?;
    MCPServer::try_from(fields).map_err(ConfigError::from)
}

fn apply_identity(fields: &mut ServerFields, inputs: &RecordInputs<'_>) {
    let config = inputs.config;
    fields.server_id = inputs.server_id.to_owned();
    fields.name = inputs.name.to_owned();
    fields.alias = inputs.alias.map(str::to_owned);
    fields.server_name = Some(inputs.server_name.to_owned());
    fields.url = Some(inputs.server_url.to_owned());
    fields.transport = config.transport.unwrap_or(MCPTransport::Http);
    fields.protocol_version = config.protocol_version;
    fields.spec_path.clone_from(&config.spec_path);
    fields.auth_type = config.auth_type;
    fields.instructions.clone_from(&config.instructions);
    fields.client_id.clone_from(&config.client_id);
    fields.client_secret = config
        .client_secret
        .as_deref()
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
    fields.token_endpoint_auth_method = config.token_endpoint_auth_method;
    fields.timeout = config.timeout;
    fields.max_concurrent_requests = config.max_concurrent_requests;
}

fn apply_catalog(
    fields: &mut ServerFields,
    config: &McpServerConfig,
    server_name: &str,
    mcp_info: MCPInfo,
) -> Result<(), ConfigError> {
    fields.mcp_info = Some(mcp_info);
    fields.extra_headers.clone_from(&config.extra_headers);
    fields.allowed_tools.clone_from(&config.allowed_tools);
    fields.disallowed_tools.clone_from(&config.disallowed_tools);
    fields.pinned_tools = config_pinned_tools(config.pinned_tools.as_ref(), server_name)?;
    fields.allowed_params.clone_from(&config.allowed_params);
    fields.static_headers.clone_from(&config.static_headers);
    fields.env_vars = config_env_vars(config);
    fields.command = Some(config.command.clone().unwrap_or_default());
    fields.args = Some(config.args.clone().unwrap_or_default());
    fields.env = Some(config.env.clone().unwrap_or_default());
    fields.access_groups.clone_from(&config.access_groups);
    fields.allow_sampling = config.allow_sampling.unwrap_or(false);
    fields.allow_elicitation = config.allow_elicitation.unwrap_or(false);
    Ok(())
}

fn apply_oauth(fields: &mut ServerFields, oauth: ResolvedOAuth) {
    fields.oauth2_flow = oauth.flow;
    fields.scopes = oauth.scopes;
    fields.configured_scopes = oauth.configured_scopes;
    fields.issuer = oauth.issuer;
    fields.issuer_is_anchored = oauth.issuer_is_anchored;
    fields.authorization_url = oauth.authorization_url;
    fields.token_url = oauth.token_url;
    fields.registration_url = oauth.registration_url;
    fields.configured_authorization_url = oauth.configured_authorization_url;
    fields.configured_token_url = oauth.configured_token_url;
    fields.configured_registration_url = oauth.configured_registration_url;
}

fn apply_exchange(fields: &mut ServerFields, config: &McpServerConfig) {
    fields
        .upstream_resource
        .clone_from(&config.upstream_resource);
    fields
        .upstream_token_header
        .clone_from(&config.upstream_token_header);
    fields
        .token_exchange_endpoint
        .clone_from(&config.token_exchange_endpoint);
    fields.audience.clone_from(&config.audience);
    fields.subject_token_type = config
        .subject_token_type
        .clone()
        .unwrap_or_else(|| DEFAULT_SUBJECT_TOKEN_TYPE.to_owned());
    fields
        .id_jag_resource_token_endpoint
        .clone_from(&config.id_jag_resource_token_endpoint);
    fields.id_jag_resource.clone_from(&config.id_jag_resource);
    fields.client_private_key = config
        .client_private_key
        .as_deref()
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
    fields
        .client_private_key_id
        .clone_from(&config.client_private_key_id);
    fields.client_assertion_signing_alg = config
        .client_assertion_signing_alg
        .clone()
        .unwrap_or_else(|| String::from("RS256"));
    fields.token_exchange_profile = config
        .token_exchange_profile
        .clone()
        .unwrap_or_else(|| String::from("rfc8693"));
    apply_aws(fields, config);
}

fn apply_aws(fields: &mut ServerFields, config: &McpServerConfig) {
    fields.aws_access_key_id = config
        .aws_access_key_id
        .as_deref()
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
    fields.aws_secret_access_key = config
        .aws_secret_access_key
        .as_deref()
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
    fields.aws_session_token = config
        .aws_session_token
        .as_deref()
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
    fields.aws_region_name.clone_from(&config.aws_region_name);
    fields.aws_service_name.clone_from(&config.aws_service_name);
    fields.aws_role_name.clone_from(&config.aws_role_name);
    fields.aws_session_name.clone_from(&config.aws_session_name);
}

fn apply_switches(
    fields: &mut ServerFields,
    config: &McpServerConfig,
    server_name: &str,
    per_server_oauth_discovery: bool,
    dcr_bridge: Option<bool>,
) -> Result<(), ConfigError> {
    fields.authentication_token = config
        .authentication_token
        .as_deref()
        .or(config.auth_value.as_deref())
        .map(|text| SecretCredential(StoredSecret(SecretString::from(text))));
    fields.allow_all_keys = config.allow_all_keys.unwrap_or(false);
    fields.available_on_public_internet = config.available_on_public_internet.unwrap_or(true);
    fields.delegate_auth_to_upstream = config.delegate_auth_to_upstream.unwrap_or(false);
    fields.oauth_passthrough = config.oauth_passthrough.unwrap_or(false);
    fields.dcr_bridge = dcr_bridge;
    fields.per_server_oauth_discovery = per_server_oauth_discovery;
    fields.token_validation =
        config_token_validation(config.token_validation.as_ref(), server_name)?;
    fields
        .oauth_identity_binding
        .clone_from(&config.oauth_identity_binding);
    Ok(())
}

pub(super) fn build_mcp_info(
    server_name: &str,
    config: &McpServerConfig,
) -> Result<MCPInfo, ConfigError> {
    let mut info: BTreeMap<String, JsonValue> = BTreeMap::new();
    if let Some(entries) = &config.mcp_info {
        for (key, value) in entries {
            info.insert(key.clone(), stored_json(value));
        }
    }
    if !info.contains_key("server_name") {
        info.insert(
            "server_name".to_owned(),
            JsonValue::String(server_name.to_owned()),
        );
    }
    if !info.contains_key("description")
        && let Some(description) = config
            .description
            .as_deref()
            .filter(|text| !text.is_empty())
    {
        info.insert(
            "description".to_owned(),
            JsonValue::String(description.to_owned()),
        );
    }
    normalize_cost_info(&mut info);
    MCPInfo::try_from(info).map_err(|source| ConfigError::McpInfo {
        server_ref: server_name.to_owned(),
        source,
    })
}

fn config_pinned_tools(
    value: Option<&Json>,
    server_ref: &str,
) -> Result<Option<BTreeMap<String, PinnedMCPTool>>, ConfigError> {
    let Some(value) = value else {
        return Ok(None);
    };
    parse_pinned_tools(stored_json(value)).map_err(|source| ConfigError::PinnedTools {
        server_ref: server_ref.to_owned(),
        source,
    })
}

fn config_token_validation(
    value: Option<&Json>,
    server_ref: &str,
) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Json::Object(entries) = value else {
        return Err(ConfigError::TokenValidation {
            server_ref: server_ref.to_owned(),
        });
    };
    Ok(Some(
        entries
            .iter()
            .map(|(key, entry)| (key.clone(), stored_json(entry)))
            .collect(),
    ))
}

fn config_env_vars(config: &McpServerConfig) -> Option<Vec<BTreeMap<String, JsonValue>>> {
    config.env_vars.as_ref().map(|entries| {
        entries
            .iter()
            .map(|entry| {
                entry
                    .iter()
                    .map(|(key, value)| (key.clone(), stored_json(value)))
                    .collect()
            })
            .collect()
    })
}

pub(super) fn config_scopes(config: &McpServerConfig) -> Option<Vec<String>> {
    match config.scopes.as_ref() {
        None => None,
        Some(ScopesConfig::Text(text)) => extract_scopes(ScopesInput::Text(text)),
        Some(ScopesConfig::List(items)) => {
            let values: Vec<JsonValue> = items
                .iter()
                .map(|item| JsonValue::String(item.clone()))
                .collect();
            extract_scopes(ScopesInput::List(&values))
        }
    }
}
