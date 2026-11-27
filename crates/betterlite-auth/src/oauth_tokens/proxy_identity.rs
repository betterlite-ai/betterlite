use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use http::header::HeaderMap;
use thiserror::Error;

use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use crate::trusted_network::proxy_utils::{
    TRUSTED_PROXY_RANGES_KEY, TrustedProxyRequirementError, require_trusted_proxy_request,
};
use crate::trusted_network::request::NetworkRequest;
use crate::trusted_network::settings::TrustedNetworkSettings;

pub const FEATURE_NAME: &str = "OAuth2 proxy auth";
// ALLOWED_OAUTH2_PROXY_FIELDS, in Python's sorted() order for the rejection message
pub const ALLOWED_OAUTH2_PROXY_FIELDS: [&str; 6] = [
    "models",
    "org_id",
    "team_alias",
    "team_id",
    "user_email",
    "user_id",
];

#[derive(Debug, Error)]
pub enum ProxyAuthError {
    #[error(transparent)]
    Trust(#[from] TrustedProxyRequirementError),
    #[error("Oauth2 config mappings not found in general_settings")]
    MappingsMissing,
    #[error(
        "Oauth2 proxy auth refuses to map non-identity UserAPIKeyAuth fields from request headers: {rejected}. Only identity fields are accepted ({allowlist}); anything else (privileges, budgets, rate limits, metadata) would let a caller forge enforcement parameters by spoofing the matching header. If you need a trusted upstream to assert anything beyond identity, use JWT auth (signature-validated) instead of header-trust."
    )]
    Disallowed { rejected: String, allowlist: String },
    #[error(
        "OAuth2 proxy auth found no identity headers for the configured oauth2_config_mappings"
    )]
    NoIdentity,
    #[error(transparent)]
    Identity(#[from] AuthError),
}

// handle_oauth2_proxy_request: the direct peer is trusted before any mapping is read
pub fn handle_oauth2_proxy_request(
    headers: &HeaderMap,
    mappings: Option<&BTreeMap<String, String>>,
    network: &NetworkRequest<'_>,
    trusted: &TrustedNetworkSettings,
) -> Result<UserAPIKeyAuth, ProxyAuthError> {
    tracing::debug!("Handling oauth2 proxy request");
    require_trusted_proxy_request(network, trusted, FEATURE_NAME, TRUSTED_PROXY_RANGES_KEY)?;
    let Some(mappings) = mappings.filter(|entries| !entries.is_empty()) else {
        return Err(ProxyAuthError::MappingsMissing);
    };
    // the allowlist rejects a privilege-granting mapping before any header value is read
    let rejected: Vec<&str> = mappings
        .keys()
        .map(String::as_str)
        .filter(|field| !ALLOWED_OAUTH2_PROXY_FIELDS.contains(field))
        .collect();
    if !rejected.is_empty() {
        return Err(ProxyAuthError::Disallowed {
            rejected: python_list(&rejected),
            allowlist: python_list(&ALLOWED_OAUTH2_PROXY_FIELDS),
        });
    }
    let fields = identity_fields(headers, mappings);
    if fields.is_empty() {
        // LiteLLM validates an empty dict into an identity-less auth; that authenticates a header-less request
        return Err(ProxyAuthError::NoIdentity);
    }
    // only the mapped field names reach the log; their values are the caller's identity
    let names: Vec<&str> = fields.keys().map(String::as_str).collect();
    tracing::debug!("OAuth2 proxy identity fields: {names:?}");
    Ok(UserAPIKeyAuth::construct(AuthInput::Dict(fields))?)
}

fn identity_fields(
    headers: &HeaderMap,
    mappings: &BTreeMap<String, String>,
) -> BTreeMap<String, JsonData> {
    let mut fields = BTreeMap::new();
    for (field, header) in mappings {
        let Some(value) = headers.get(header.as_str()) else {
            continue;
        };
        // Starlette reads header bytes as latin-1; an empty value is falsy and never maps
        let text: String = value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect();
        if text.is_empty() {
            continue;
        }
        let data = if field == "models" {
            let models = text
                .split(',')
                .map(|item| JsonData::String(item.trim().to_owned()))
                .collect();
            JsonData::Array(models)
        } else {
            JsonData::String(text)
        };
        fields.insert(field.clone(), data);
    }
    fields
}

fn python_list(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("'{name}'")).collect();
    format!("[{}]", quoted.join(", "))
}
