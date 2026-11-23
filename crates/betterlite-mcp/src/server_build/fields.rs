use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::managed_resource_rows::wire::{RowTimestamp, SecretMap};
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use super::BuildError;
use super::credentials::ResolvedCredentials;
use super::json::normalize_cost_info;
use super::oauth_step::OAuthResolution;
use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;
use crate::server_record::ServerFields;

// the record's own defaults for the three fields serde requires; every other field is set below
const RECORD_SEED: &str = r#"{"server_id":"","name":"","transport":"http"}"#;

pub(super) struct RecordParts {
    pub name: String,
    pub mcp_info: MCPInfo,
    pub env: Option<BTreeMap<String, String>>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub env_vars: Option<Vec<BTreeMap<String, JsonValue>>>,
    pub credentials: ResolvedCredentials,
    pub oauth: OAuthResolution,
}

pub(super) fn server_fields(
    row: &McpServerTable,
    parts: RecordParts,
) -> Result<ServerFields, BuildError> {
    let mut fields: ServerFields = serde_json::from_str(RECORD_SEED)?;
    fields.name = parts.name;
    fields.mcp_info = Some(parts.mcp_info);
    fields.env = parts.env;
    fields.static_headers = parts.static_headers;
    fields.env_vars = parts.env_vars;
    apply_row_identity(&mut fields, row)?;
    apply_row_lists(&mut fields, row);
    apply_row_flags(&mut fields, row);
    apply_credentials(&mut fields, &parts.credentials);
    apply_oauth(&mut fields, parts.oauth);
    apply_token_exchange(&mut fields, row, &parts.credentials);
    apply_blob_credentials(&mut fields, &parts.credentials);
    Ok(fields)
}

fn apply_row_identity(fields: &mut ServerFields, row: &McpServerTable) -> Result<(), BuildError> {
    fields.server_id.clone_from(&row.server_id);
    fields.alias.clone_from(&row.alias);
    fields.server_name.clone_from(&row.server_name);
    fields.url.clone_from(&row.url);
    fields.spec_path.clone_from(&row.spec_path);
    fields.transport = row.transport;
    fields.auth_type = row.auth_type;
    fields.instructions.clone_from(&row.instructions);
    fields.created_at = row.created_at.map(timestamp_utc).transpose()?;
    fields.updated_at = row.updated_at.map(timestamp_utc).transpose()?;
    Ok(())
}

fn apply_row_lists(fields: &mut ServerFields, row: &McpServerTable) {
    fields.allowed_tools = Some(row.allowed_tools.clone());
    fields.extra_headers = Some(row.extra_headers.clone());
    fields.access_groups = Some(row.mcp_access_groups.clone());
    fields.args = Some(row.args.clone());
    fields.tool_name_to_display_name = row
        .tool_name_to_display_name
        .clone()
        .filter(|entries| !entries.is_empty());
    fields.tool_name_to_description = row
        .tool_name_to_description
        .clone()
        .filter(|entries| !entries.is_empty());
    fields.pinned_tools.clone_from(&row.pinned_tools);
}

fn apply_row_flags(fields: &mut ServerFields, row: &McpServerTable) {
    fields.command.clone_from(&row.command);
    fields.allow_all_keys = row.allow_all_keys.0;
    fields.available_on_public_internet = row.available_on_public_internet.0;
    fields.delegate_auth_to_upstream = row.delegate_auth_to_upstream.0;
    fields.oauth_passthrough = row.oauth_passthrough.0;
    fields.dcr_bridge = row.dcr_bridge;
    fields.per_server_oauth_discovery = row.per_server_oauth_discovery.0;
    fields.is_byok = row.is_byok.0;
    fields.byok_description.clone_from(&row.byok_description);
    fields
        .byok_api_key_help_url
        .clone_from(&row.byok_api_key_help_url);
    fields.source_url.clone_from(&row.source_url);
    fields.timeout = row.timeout.as_ref().map(Decimal::as_f64);
    fields.max_concurrent_requests = row.max_concurrent_requests;
}

fn apply_credentials(fields: &mut ServerFields, credentials: &ResolvedCredentials) {
    fields
        .authentication_token
        .clone_from(&credentials.authentication_token);
    fields.client_id.clone_from(&credentials.client_id);
    fields.client_secret.clone_from(&credentials.client_secret);
}

fn apply_oauth(fields: &mut ServerFields, oauth: OAuthResolution) {
    fields.oauth2_flow = oauth.oauth2_flow;
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
    fields.token_endpoint_auth_method = oauth.token_endpoint_auth_method;
}

fn apply_token_exchange(
    fields: &mut ServerFields,
    row: &McpServerTable,
    credentials: &ResolvedCredentials,
) {
    fields.token_exchange_endpoint = first_non_empty(
        row.token_exchange_endpoint.as_deref(),
        credentials.token_exchange_endpoint.as_deref(),
    );
    fields.audience = first_non_empty(row.audience.as_deref(), credentials.audience.as_deref());
    fields.subject_token_type = first_non_empty(
        row.subject_token_type.as_deref(),
        credentials.subject_token_type.as_deref(),
    )
    .filter(|text| !text.is_empty())
    .unwrap_or_else(|| DEFAULT_SUBJECT_TOKEN_TYPE.to_owned());
    fields.token_exchange_profile = first_non_empty(
        row.token_exchange_profile.as_deref(),
        credentials.token_exchange_profile.as_deref(),
    )
    .filter(|text| !text.is_empty())
    .unwrap_or_else(|| String::from("rfc8693"));
}

fn apply_blob_credentials(fields: &mut ServerFields, credentials: &ResolvedCredentials) {
    fields
        .upstream_resource
        .clone_from(&credentials.upstream_resource);
    fields
        .upstream_token_header
        .clone_from(&credentials.upstream_token_header);
    fields
        .id_jag_resource_token_endpoint
        .clone_from(&credentials.id_jag_resource_token_endpoint);
    fields
        .id_jag_resource
        .clone_from(&credentials.id_jag_resource);
    fields
        .client_private_key
        .clone_from(&credentials.client_private_key);
    fields
        .client_private_key_id
        .clone_from(&credentials.client_private_key_id);
    fields.client_assertion_signing_alg = credentials
        .client_assertion_signing_alg
        .as_deref()
        .filter(|text| !text.is_empty())
        .map_or_else(|| String::from("RS256"), str::to_owned);
    let aws = &credentials.aws;
    fields.aws_access_key_id.clone_from(&aws.access_key_id);
    fields
        .aws_secret_access_key
        .clone_from(&aws.secret_access_key);
    fields.aws_session_token.clone_from(&aws.session_token);
    fields.aws_region_name.clone_from(&aws.region_name);
    fields.aws_service_name.clone_from(&aws.service_name);
    fields.aws_role_name.clone_from(&aws.role_name);
    fields.aws_session_name.clone_from(&aws.session_name);
}

pub(super) fn first_non_empty(first: Option<&str>, second: Option<&str>) -> Option<String> {
    first
        .filter(|text| !text.is_empty())
        .map_or_else(|| second.map(str::to_owned), |text| Some(text.to_owned()))
}

pub(super) fn name_for_prefix(row: &McpServerTable) -> String {
    row.alias
        .as_deref()
        .filter(|text| !text.is_empty())
        .map_or_else(
            || {
                row.server_name
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .unwrap_or(&row.server_id)
                    .to_owned()
            },
            str::to_owned,
        )
}

pub(super) fn record_mcp_info(row: &McpServerTable) -> Result<MCPInfo, BuildError> {
    let mut info: BTreeMap<String, JsonValue> = match row.mcp_info.as_ref() {
        Some(stored) => {
            let encoded = serde_json::to_string(stored)?;
            serde_json::from_str(&encoded)?
        }
        None => BTreeMap::new(),
    };
    if !info.contains_key("server_name") {
        let server_name = row
            .server_name
            .as_deref()
            .filter(|text| !text.is_empty())
            .unwrap_or(&row.server_id);
        info.insert(
            "server_name".to_owned(),
            JsonValue::String(server_name.to_owned()),
        );
    }
    if !info.contains_key("description")
        && let Some(description) = row.description.as_deref().filter(|text| !text.is_empty())
    {
        info.insert(
            "description".to_owned(),
            JsonValue::String(description.to_owned()),
        );
    }
    normalize_cost_info(&mut info);
    MCPInfo::try_from(info).map_err(BuildError::from)
}

fn timestamp_utc(value: RowTimestamp) -> Result<DateTime<Utc>, BuildError> {
    let encoded = serde_json::to_string(&value)?;
    let text: String = serde_json::from_str(&encoded)?;
    if let Ok(aware) = DateTime::parse_from_rfc3339(&text) {
        return Ok(aware.with_timezone(&Utc));
    }
    let naive = NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_naive_error| NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S%.f"))?;
    Ok(naive.and_utc())
}

pub(super) fn exposed_map(map: &SecretMap) -> BTreeMap<String, String> {
    let mut exposed = BTreeMap::new();
    for (name, value) in map {
        exposed.insert(name.clone(), value.expose_secret().to_owned());
    }
    exposed
}
