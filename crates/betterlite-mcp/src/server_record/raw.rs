use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_db::mcp_stored_contracts::{
    JsonValue, MCPAuth, MCPTokenEndpointAuthMethod, MCPTransport, MCPUpstreamProtocol,
    PinnedMCPTool,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{MCPOAuthIdentityBinding, MCPServer, OAuth2Flow, SecretCredential, ServerRecordError};
use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;

#[derive(Deserialize)]
pub(super) struct RawMCPServer {
    pub(super) server_id: String,
    pub(super) name: String,
    pub(super) alias: Option<String>,
    pub(super) server_name: Option<String>,
    pub(super) url: Option<String>,
    pub(super) transport: MCPTransport,
    pub(super) protocol_version: Option<MCPUpstreamProtocol>,
    pub(super) spec_path: Option<String>,
    pub(super) auth_type: Option<MCPAuth>,
    pub(super) authentication_token: Option<SecretCredential>,
    pub(super) instructions: Option<String>,
    pub(super) mcp_info: Option<MCPInfo>,
    pub(super) extra_headers: Option<Vec<String>>,
    pub(super) allowed_tools: Option<Vec<String>>,
    pub(super) disallowed_tools: Option<Vec<String>>,
    pub(super) tool_name_to_display_name: Option<BTreeMap<String, String>>,
    pub(super) tool_name_to_description: Option<BTreeMap<String, String>>,
    pub(super) pinned_tools: Option<BTreeMap<String, PinnedMCPTool>>,
    pub(super) allowed_params: Option<BTreeMap<String, Vec<String>>>,
    pub(super) static_headers: Option<BTreeMap<String, String>>,
    pub(super) env_vars: Option<Vec<JsonValue>>,
    pub(super) client_id: Option<String>,
    pub(super) client_secret: Option<SecretCredential>,
    pub(super) issuer: Option<String>,
    pub(super) issuer_is_anchored: Option<bool>,
    pub(super) scopes: Option<Vec<String>>,
    pub(super) authorization_url: Option<String>,
    pub(super) token_url: Option<String>,
    pub(super) registration_url: Option<String>,
    pub(super) configured_authorization_url: Option<String>,
    pub(super) configured_token_url: Option<String>,
    pub(super) configured_registration_url: Option<String>,
    pub(super) configured_scopes: Option<Vec<String>>,
    pub(super) token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
    pub(super) upstream_resource: Option<String>,
    pub(super) upstream_token_header: Option<String>,
    pub(super) aws_access_key_id: Option<SecretCredential>,
    pub(super) aws_secret_access_key: Option<SecretCredential>,
    pub(super) aws_session_token: Option<SecretCredential>,
    pub(super) aws_region_name: Option<String>,
    pub(super) aws_service_name: Option<String>,
    pub(super) aws_role_name: Option<String>,
    pub(super) aws_session_name: Option<String>,
    pub(super) token_exchange_endpoint: Option<String>,
    pub(super) audience: Option<String>,
    #[serde(default = "default_subject_token_type")]
    pub(super) subject_token_type: String,
    pub(super) id_jag_resource_token_endpoint: Option<String>,
    pub(super) id_jag_resource: Option<String>,
    pub(super) client_private_key: Option<SecretCredential>,
    pub(super) client_private_key_id: Option<String>,
    #[serde(default = "default_signing_alg")]
    pub(super) client_assertion_signing_alg: String,
    #[serde(default = "default_token_exchange_profile")]
    pub(super) token_exchange_profile: String,
    pub(super) command: Option<String>,
    pub(super) args: Option<Vec<String>>,
    pub(super) env: Option<BTreeMap<String, String>>,
    pub(super) access_groups: Option<Vec<String>>,
    pub(super) allow_all_keys: Option<bool>,
    pub(super) available_on_public_internet: Option<bool>,
    pub(super) delegate_auth_to_upstream: Option<bool>,
    pub(super) oauth_passthrough: Option<bool>,
    pub(super) dcr_bridge: Option<bool>,
    pub(super) per_server_oauth_discovery: Option<bool>,
    pub(super) is_byok: Option<bool>,
    #[serde(default)]
    pub(super) byok_description: Vec<String>,
    pub(super) byok_api_key_help_url: Option<String>,
    pub(super) source_url: Option<String>,
    pub(super) created_at: Option<DateTime<Utc>>,
    pub(super) updated_at: Option<DateTime<Utc>>,
    pub(super) oauth2_flow: Option<OAuth2Flow>,
    pub(super) token_validation: Option<BTreeMap<String, JsonValue>>,
    pub(super) oauth_identity_binding: Option<MCPOAuthIdentityBinding>,
    pub(super) token_storage_ttl_seconds: Option<i64>,
    pub(super) timeout: Option<f64>,
    pub(super) max_concurrent_requests: Option<i64>,
    pub(super) short_prefix: Option<String>,
    pub(super) allow_sampling: Option<bool>,
    pub(super) allow_elicitation: Option<bool>,
}

fn default_subject_token_type() -> String {
    DEFAULT_SUBJECT_TOKEN_TYPE.to_owned()
}

fn default_signing_alg() -> String {
    "RS256".to_owned()
}

fn default_token_exchange_profile() -> String {
    "rfc8693".to_owned()
}

#[expect(
    clippy::too_many_lines,
    reason = "every field of the 77-field record is named once"
)]
impl TryFrom<RawMCPServer> for MCPServer {
    type Error = ServerRecordError;

    fn try_from(raw: RawMCPServer) -> Result<Self, Self::Error> {
        Self {
            server_id: raw.server_id,
            name: raw.name,
            alias: raw.alias,
            server_name: raw.server_name,
            url: raw.url,
            transport: raw.transport,
            protocol_version: raw.protocol_version,
            spec_path: raw.spec_path,
            auth_type: raw.auth_type,
            authentication_token: raw.authentication_token,
            instructions: raw.instructions,
            mcp_info: raw.mcp_info,
            extra_headers: raw.extra_headers,
            allowed_tools: raw.allowed_tools,
            disallowed_tools: raw.disallowed_tools,
            tool_name_to_display_name: raw.tool_name_to_display_name,
            tool_name_to_description: raw.tool_name_to_description,
            pinned_tools: raw.pinned_tools,
            allowed_params: raw.allowed_params,
            static_headers: raw.static_headers,
            env_vars: raw.env_vars,
            client_id: raw.client_id,
            client_secret: raw.client_secret,
            issuer: raw.issuer,
            issuer_is_anchored: raw.issuer_is_anchored.unwrap_or(false),
            scopes: raw.scopes,
            authorization_url: raw.authorization_url,
            token_url: raw.token_url,
            registration_url: raw.registration_url,
            configured_authorization_url: raw.configured_authorization_url,
            configured_token_url: raw.configured_token_url,
            configured_registration_url: raw.configured_registration_url,
            configured_scopes: raw.configured_scopes,
            token_endpoint_auth_method: raw.token_endpoint_auth_method,
            upstream_resource: raw.upstream_resource,
            upstream_token_header: raw.upstream_token_header,
            aws_access_key_id: raw.aws_access_key_id,
            aws_secret_access_key: raw.aws_secret_access_key,
            aws_session_token: raw.aws_session_token,
            aws_region_name: raw.aws_region_name,
            aws_service_name: raw.aws_service_name,
            aws_role_name: raw.aws_role_name,
            aws_session_name: raw.aws_session_name,
            token_exchange_endpoint: raw.token_exchange_endpoint,
            audience: raw.audience,
            subject_token_type: raw.subject_token_type,
            id_jag_resource_token_endpoint: raw.id_jag_resource_token_endpoint,
            id_jag_resource: raw.id_jag_resource,
            client_private_key: raw.client_private_key,
            client_private_key_id: raw.client_private_key_id,
            client_assertion_signing_alg: raw.client_assertion_signing_alg,
            token_exchange_profile: raw.token_exchange_profile,
            command: raw.command,
            args: raw.args,
            env: raw.env,
            access_groups: raw.access_groups,
            allow_all_keys: raw.allow_all_keys.unwrap_or(false),
            available_on_public_internet: raw.available_on_public_internet.unwrap_or(true),
            delegate_auth_to_upstream: raw.delegate_auth_to_upstream.unwrap_or(false),
            oauth_passthrough: raw.oauth_passthrough.unwrap_or(false),
            dcr_bridge: raw.dcr_bridge,
            per_server_oauth_discovery: raw.per_server_oauth_discovery.unwrap_or(false),
            is_byok: raw.is_byok.unwrap_or(false),
            byok_description: raw.byok_description,
            byok_api_key_help_url: raw.byok_api_key_help_url,
            source_url: raw.source_url,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            oauth2_flow: raw.oauth2_flow,
            token_validation: raw.token_validation,
            oauth_identity_binding: raw.oauth_identity_binding,
            token_storage_ttl_seconds: raw.token_storage_ttl_seconds,
            timeout: raw.timeout,
            max_concurrent_requests: raw.max_concurrent_requests,
            short_prefix: raw.short_prefix,
            allow_sampling: raw.allow_sampling.unwrap_or(false),
            allow_elicitation: raw.allow_elicitation.unwrap_or(false),
        }
        .checked()
    }
}
