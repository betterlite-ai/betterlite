use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::mcp::{McpOAuth2Flow, McpServerTable};
use betterlite_db::managed_resource_rows::wire::{SecretMap, SecretMapField};
use betterlite_db::mcp_stored_contracts::MCPCredentials;
use secrecy::SecretString;

use crate::server_record::{OAuth2Flow, ServerFields};

pub(super) fn fill_auth(row: &mut McpServerTable, fields: &ServerFields) {
    row.credentials = fields
        .configured_scopes
        .as_deref()
        .filter(|scopes| !scopes.is_empty())
        .map(credentials);
    row.static_headers = fields
        .static_headers
        .as_ref()
        .map(|values| SecretMapField(secret_map(values)));
    row.env = SecretMapField(fields.env.as_ref().map_or_else(SecretMap::new, secret_map));
    row.issuer.clone_from(&fields.issuer);
    row.authorization_url = fields
        .configured_authorization_url
        .clone()
        .or_else(|| fields.authorization_url.clone());
    row.token_url = fields
        .configured_token_url
        .clone()
        .or_else(|| fields.token_url.clone());
    row.registration_url = fields
        .configured_registration_url
        .clone()
        .or_else(|| fields.registration_url.clone());
}

pub(super) fn fill_oauth(row: &mut McpServerTable, fields: &ServerFields) {
    row.oauth2_flow = fields.oauth2_flow.map(row_oauth2_flow);
    row.token_exchange_endpoint
        .clone_from(&fields.token_exchange_endpoint);
    row.audience.clone_from(&fields.audience);
    row.subject_token_type = Some(fields.subject_token_type.clone());
    row.token_exchange_profile = Some(fields.token_exchange_profile.clone());
}

fn credentials(scopes: &[String]) -> MCPCredentials {
    MCPCredentials {
        auth_value: None,
        client_id: None,
        client_secret: None,
        scopes: Some(Some(scopes.to_vec())),
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
        upstream_resource: None,
        upstream_token_header: None,
        client_private_key: None,
        client_private_key_id: None,
        client_assertion_signing_alg: None,
        token_endpoint_auth_method: None,
        redirect_uris: None,
        token_exchange_profile: None,
    }
}

fn secret_map(values: &BTreeMap<String, String>) -> SecretMap {
    let mut secrets = SecretMap::new();
    for (name, value) in values {
        secrets.insert(name.clone(), SecretString::from(value.clone()));
    }
    secrets
}

const fn row_oauth2_flow(flow: OAuth2Flow) -> McpOAuth2Flow {
    match flow {
        OAuth2Flow::ClientCredentials => McpOAuth2Flow::ClientCredentials,
        OAuth2Flow::AuthorizationCode => McpOAuth2Flow::AuthorizationCode,
    }
}
